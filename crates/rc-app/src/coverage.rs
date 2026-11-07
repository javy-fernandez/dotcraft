//! Test coverage (Rider's dotCover, with coverlet): runs the tests collecting coverage, shows
//! the result by assembly and class in the Coverage window, and marks lines in the editor.

use rc_core::{Inbox, Waker};
use rc_dotnet::coverage::{CoverageEvent, CoverageReport, LineState};
use rc_dotnet::{Dotnet, OpHandle};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub struct Coverage {
    pub running: bool,
    /// Progress ("Building…", "Shop.Core.XunitTests…").
    pub status: String,
    pub report: Option<CoverageReport>,
    pub error: Option<String>,
    pub warnings: Vec<String>,
    pub tests_failed: bool,
    pub show_in_editor: bool,
    pub expanded: HashSet<String>,
    pub target: Option<PathBuf>,
    /// Documents whose lines carry this report's marks.
    marked: HashSet<u64>,
    handle: Option<OpHandle>,
    inbox: Inbox<CoverageEvent>,
}

impl Coverage {
    pub fn new(waker: Waker) -> Self {
        Coverage { running: false, status: String::new(), report: None, error: None, warnings: Vec::new(), tests_failed: false, show_in_editor: true, expanded: HashSet::new(), target: None, marked: HashSet::new(), handle: None, inbox: Inbox::new(waker) }
    }

    pub fn start(&mut self, dotnet: &Dotnet, target: &Path) {
        if let Some(h) = self.handle.take() {
            h.cancel();
        }
        self.running = true;
        self.status = rc_kit::i18n::tr("Building…").to_string();
        self.error = None;
        self.warnings.clear();
        self.target = Some(target.to_path_buf());
        self.handle = Some(rc_dotnet::coverage::start_coverage(dotnet, target, self.inbox.sender()));
    }

    pub fn stop(&mut self) {
        if let Some(h) = self.handle.take() {
            h.cancel();
        }
        self.running = false;
    }

    /// Takes progress and the result; true when a new report arrived.
    pub fn poll(&mut self) -> bool {
        let mut done = false;
        for ev in self.inbox.drain() {
            match ev {
                CoverageEvent::Build(_) => {}
                CoverageEvent::Warning(w) => self.warnings.push(w),
                CoverageEvent::ProjectStarted { project } => {
                    self.status = rc_kit::i18n::trf("Running {project}…", &[("project", &project.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default())]);
                }
                CoverageEvent::Output { .. } | CoverageEvent::ProjectFinished(_) => {}
                CoverageEvent::Finished(r) => {
                    self.running = false;
                    self.handle = None;
                    if r.cancelled {
                        continue;
                    }
                    if let Some(b) = r.build.as_ref().filter(|b| !b.success) {
                        self.error = Some(rc_kit::i18n::trf("The build failed with {n} errors", &[("n", &b.errors)]));
                        continue;
                    }
                    self.tests_failed = !r.tests_passed;
                    self.report = Some(r.report);
                    self.marked.clear();
                    for (i, a) in self.report.iter().flat_map(|r| &r.assemblies).enumerate() {
                        if i == 0 {
                            self.expanded.insert(a.name.clone());
                        }
                    }
                    done = true;
                }
                CoverageEvent::Failed(e) => {
                    self.running = false;
                    self.handle = None;
                    self.error = Some(e);
                }
            }
        }
        done
    }

    /// Puts (or removes) the line marks on the open documents.
    pub fn mark(&mut self, docs: &mut crate::docs::Docs) {
        let show = self.show_in_editor && self.report.is_some();
        for d in &mut docs.list {
            if !show {
                if self.marked.remove(&d.editor.id) || !d.editor.coverage.is_empty() {
                    d.editor.coverage.clear();
                }
                continue;
            }
            if self.marked.contains(&d.editor.id) {
                continue;
            }
            self.marked.insert(d.editor.id);
            d.editor.coverage.clear();
            if let Some(f) = self.report.as_ref().and_then(|r| r.file(&d.path)) {
                for &line in f.lines.keys() {
                    if let Some(state) = f.line_state(line) {
                        let mark = match state {
                            LineState::Covered => rc_editor::CoverageMark::Covered,
                            LineState::Partial => rc_editor::CoverageMark::Partial,
                            LineState::NotCovered => rc_editor::CoverageMark::NotCovered,
                        };
                        d.editor.coverage.insert(line, mark);
                    }
                }
            }
        }
    }
}
