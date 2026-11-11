//! Code insight for the MSBuild files that hold package items (`.csproj`, `.fsproj`, `.vbproj`,
//! `Directory.Packages.props`, `Directory.Build.props`/`.targets`), drawn with the editor's own
//! completion list, hover, gutter and diagnostics: package ids and versions from the NuGet
//! source, the version in effect and where it comes from, "update available" markers, and the
//! Central Package Management problems (NU1008, NU1010, unused versions) with their fixes.
//!
//! Network requests run on the runtime and are cached for the session: completion searches are
//! debounced and cancelled by newer ones; versions are fetched when a file is opened or saved
//! (never while typing) and on hover. CPM is read statically on a background thread.

use crate::docs::{Doc, Docs};
use rc_core::{Inbox, Runtime, Waker};
use rc_dotnet::cpm::{self, ProjectCpm, SolutionCpm};
use rc_dotnet::nuget::{NugetClient, PackageSource, PackageSummary, compare_versions};
use rc_dotnet::project_xml::{self, PackageAttr, PackageItem, byte_of, char_of};
use rc_editor::{Completion, CompletionItem, EditorEvent, GutterIcon, Hover, InlayHint, Severity};
use rc_kit::i18n::{tr, trf};
use rc_kit::ico;
use rc_lsp::lsp_types as lsp;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

const SEARCH_DELAY: Duration = Duration::from_millis(200);
const ANALYZE_DELAY: Duration = Duration::from_millis(250);
const SCAN_DELAY: Duration = Duration::from_millis(400);
/// Command of the context actions offered here (the Context Actions popup hands them back).
pub const FIX_COMMAND: &str = "dotcraft.packages.fix";

/// A change offered by a marker, a problem or the hover.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PackageFix {
    /// The version in a central file (`condition` picks a conditional `PackageVersion`).
    SetCentral { file: PathBuf, id: String, condition: Option<String>, version: String },
    /// `Version` or `VersionOverride` on a project's reference.
    SetReference { project: PathBuf, id: String, name: String, version: String },
    /// NU1008: the reference's `Version` moves to the central file.
    MoveVersion { project: PathBuf, file: PathBuf, id: String },
    /// NU1010: a central version (empty: the latest stable, fetched first).
    AddCentral { file: PathBuf, id: String, version: String },
    /// A `PackageVersion` no project uses.
    DropCentral { file: PathBuf, id: String },
    /// Show the package in the NuGet window.
    Open { id: String },
}

impl PackageFix {
    fn title(&self) -> String {
        let name = |p: &Path| p.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        match self {
            PackageFix::SetCentral { id, version, .. } | PackageFix::SetReference { id, version, .. } => trf("Update {id} to {version}", &[("id", id), ("version", version)]),
            PackageFix::MoveVersion { file, id, .. } => trf("Move the version of {id} to {file}", &[("id", id), ("file", &name(file))]),
            PackageFix::AddCentral { file, id, version } if version.is_empty() => trf("Add the latest version of {id} to {file}", &[("id", id), ("file", &name(file))]),
            PackageFix::AddCentral { file, id, version } => trf("Add {id} {version} to {file}", &[("id", id), ("version", version), ("file", &name(file))]),
            PackageFix::DropCentral { id, .. } => trf("Remove unused version of {id}", &[("id", id)]),
            PackageFix::Open { id } => trf("Open {id} in NuGet Window", &[("id", id)]),
        }
    }

    /// As an entry of the Context Actions popup.
    fn action(&self, quickfix: bool) -> lsp::CodeActionOrCommand {
        let title = self.title();
        let command = lsp::Command { title: title.clone(), command: FIX_COMMAND.into(), arguments: Some(vec![serde_json::to_value(self).unwrap_or_default()]) };
        let kind = if quickfix { lsp::CodeActionKind::QUICKFIX } else { lsp::CodeActionKind::REFACTOR };
        lsp::CodeActionOrCommand::CodeAction(lsp::CodeAction { title, kind: Some(kind), command: Some(command), ..Default::default() })
    }
}

/// The fix behind a Context Actions entry made here.
pub fn fix_of(action: &lsp::CodeActionOrCommand) -> Option<PackageFix> {
    let cmd = match action {
        lsp::CodeActionOrCommand::Command(c) => c,
        lsp::CodeActionOrCommand::CodeAction(a) => a.command.as_ref()?,
    };
    (cmd.command == FIX_COMMAND).then(|| serde_json::from_value(cmd.arguments.as_ref()?.first()?.clone()).ok()).flatten()
}

/// What the rest of the UI must do.
#[derive(Debug)]
pub enum PackageOutput {
    /// Context actions to choose from (the Context Actions popup).
    Actions(Vec<lsp::CodeActionOrCommand>),
    /// Apply this fix (hover buttons, fixes waiting for a version).
    Fix(PackageFix),
    Message(String),
}

/// A problem found in a file (byte range of its text).
#[derive(Clone, Debug, PartialEq)]
struct Finding {
    range: Range<usize>,
    line: usize,
    severity: Severity,
    code: Option<&'static str>,
    message: String,
    unnecessary: bool,
    fix: Option<PackageFix>,
}

/// Where the version in effect for an item comes from.
#[derive(Clone, Debug, PartialEq)]
enum Source {
    /// `VersionOverride` on the reference (with the central version it overrides).
    Override(Option<(String, PathBuf)>),
    /// `Version` on the reference.
    Reference,
    /// A `PackageVersion` of a central file.
    Central(PathBuf),
    /// A `GlobalPackageReference` of a central file.
    Global(PathBuf),
    /// The item itself, in a central file.
    Own,
}

#[derive(Clone, Debug, PartialEq)]
struct Effective {
    version: String,
    source: Source,
}

/// An item whose package has a newer stable version.
#[derive(Clone, Debug, PartialEq)]
struct Marker {
    line: usize,
    /// Byte offset of the inline hint.
    at: usize,
    latest: String,
    fix: PackageFix,
}

/// CPM of the workspace, read statically.
#[derive(Default)]
struct Context {
    solution: SolutionCpm,
    /// Open files outside the solution's projects (`Directory.Build.props`, other projects).
    extra: Vec<ProjectCpm>,
}

impl Context {
    fn project(&self, p: &Path) -> Option<&ProjectCpm> {
        self.solution.project(p).or_else(|| self.extra.iter().find(|c| c.project == p))
    }

    /// CPM projects whose central files include `file`.
    fn readers(&self, file: &Path) -> Vec<&ProjectCpm> {
        self.solution.projects.iter().filter(|p| p.enabled && self.solution.chain(&p.project).iter().any(|f| f.path == file)).collect()
    }

    /// The version `path` gets for `item`.
    fn effective(&self, path: &Path, text: &str, item: &PackageItem) -> Option<Effective> {
        if project_xml::is_central_file(path) {
            let resolved = match item.version.as_deref() {
                Some(v) if v.contains("$(") => cpm::CentralFile::parse(path, text).ok()?.items.into_iter().find(|i| i.line == item.line && i.id.eq_ignore_ascii_case(&item.id)).map(|i| i.resolved)?,
                Some(v) => v.to_string(),
                None => return None,
            };
            return Some(Effective { version: resolved, source: Source::Own });
        }
        if item.element != "PackageReference" {
            return None;
        }
        let pc = self.project(path).filter(|p| p.enabled);
        let central = pc.and_then(|p| p.central_version(&item.id)).map(|v| (v.version.clone(), v.file.clone()));
        if let Some(v) = &item.version_override {
            return Some(Effective { version: v.clone(), source: Source::Override(central) });
        }
        if let Some(v) = &item.version {
            return Some(Effective { version: v.clone(), source: Source::Reference });
        }
        let pc = pc?;
        if let Some(g) = pc.global(&item.id) {
            return Some(Effective { version: g.version.clone(), source: Source::Global(g.file.clone()) });
        }
        central.map(|(version, file)| Effective { version, source: Source::Central(file) })
    }
}

/// The fix that sets `item`'s version in effect to `version`.
fn update_fix(path: &Path, item: &PackageItem, source: &Source, version: &str) -> PackageFix {
    let (id, version) = (item.id.clone(), version.to_string());
    match source {
        Source::Override(_) => PackageFix::SetReference { project: path.to_path_buf(), id, name: "VersionOverride".into(), version },
        Source::Reference => PackageFix::SetReference { project: path.to_path_buf(), id, name: "Version".into(), version },
        Source::Central(file) | Source::Global(file) => PackageFix::SetCentral { file: file.clone(), id, condition: None, version },
        Source::Own => PackageFix::SetCentral { file: path.to_path_buf(), id, condition: item.condition.clone(), version },
    }
}

fn line_of(text: &str, byte: usize) -> usize {
    text[..byte.min(text.len())].matches('\n').count()
}

/// The CPM problems of a file.
fn analyze(path: &Path, text: &str, items: &[PackageItem], ctx: &Context) -> Vec<Finding> {
    let mut out = Vec::new();
    if project_xml::is_central_file(path) {
        let readers = ctx.readers(path);
        // Versions pinned for transitive packages are used without a reference.
        if readers.is_empty() || readers.iter().any(|p| p.transitive_pinning) {
            return out;
        }
        for i in items.iter().filter(|i| i.element == "PackageVersion") {
            if !readers.iter().any(|p| p.reference(&i.id).is_some()) {
                out.push(Finding {
                    range: i.id_range.clone(),
                    line: i.line,
                    severity: Severity::Warning,
                    code: None,
                    message: trf("{id} has a central version but no project references it", &[("id", &i.id)]),
                    unnecessary: true,
                    fix: Some(PackageFix::DropCentral { file: path.to_path_buf(), id: i.id.clone() }),
                });
            }
        }
        return out;
    }
    let Some(pc) = ctx.project(path).filter(|p| p.enabled) else { return out };
    for i in items.iter().filter(|i| i.element == "PackageReference") {
        if let Some(span) = i.version_span.clone().filter(|_| i.version.is_some()) {
            let file = pc.central_version(&i.id).map(|v| v.file.clone()).or_else(|| pc.central_file.clone());
            out.push(Finding {
                line: line_of(text, span.start),
                range: span,
                severity: Severity::Error,
                code: Some("NU1008"),
                message: trf("{id}: with central package management the version belongs in Directory.Packages.props, not on the reference", &[("id", &i.id)]),
                unnecessary: false,
                fix: file.map(|file| PackageFix::MoveVersion { project: path.to_path_buf(), file, id: i.id.clone() }),
            });
        } else if i.version_override.is_none() && pc.central_version(&i.id).is_none() && pc.global(&i.id).is_none() {
            out.push(Finding {
                range: i.id_range.clone(),
                line: i.line,
                severity: Severity::Error,
                code: Some("NU1010"),
                message: trf("{id} has no version in Directory.Packages.props", &[("id", &i.id)]),
                unnecessary: false,
                fix: pc.central_file.clone().map(|file| PackageFix::AddCentral { file, id: i.id.clone(), version: String::new() }),
            });
        }
    }
    out
}

fn diagnostics(text: &str, findings: &[Finding]) -> Vec<rc_editor::Diagnostic> {
    findings
        .iter()
        .map(|f| rc_editor::Diagnostic {
            start: char_of(text, f.range.start),
            end: char_of(text, f.range.end),
            severity: f.severity,
            message: f.message.clone(),
            code: f.code.map(str::to_string),
            unnecessary: f.unnecessary,
            deprecated: false,
        })
        .collect()
}

/// Reads CPM for the projects and the open files, and the problems of every file on disk.
fn scan(projects: Vec<PathBuf>, open: Vec<PathBuf>) -> (Context, BTreeMap<PathBuf, Vec<rc_editor::Diagnostic>>) {
    let canon = |p: &PathBuf| dunce::canonicalize(p).unwrap_or_else(|_| p.clone());
    let projects: Vec<PathBuf> = projects.iter().map(canon).collect();
    let solution = SolutionCpm::new(projects.iter().map(|p| cpm::detect_static(p)).collect());
    let extra = open.iter().filter(|p| !projects.contains(p) && !project_xml::is_central_file(p)).map(|p| cpm::detect_static(p)).collect();
    let ctx = Context { solution, extra };
    let mut files: Vec<PathBuf> = projects.clone();
    files.extend(ctx.solution.files.keys().cloned());
    files.extend(open);
    files.sort();
    files.dedup();
    let mut problems = BTreeMap::new();
    for f in files {
        let Ok(text) = std::fs::read_to_string(&f) else { continue };
        let text = text.strip_prefix('\u{feff}').unwrap_or(&text).replace("\r\n", "\n");
        let Some(items) = project_xml::package_items(&text) else { continue };
        let found = analyze(&f, &text, &items, &ctx);
        if !found.is_empty() {
            problems.insert(f, diagnostics(&text, &found));
        }
    }
    (ctx, problems)
}

enum Event {
    /// The first source of `nuget.config` for workspace generation `u64`.
    Source(u64, PackageSource),
    Scan(u64, Arc<Context>, BTreeMap<PathBuf, Vec<rc_editor::Diagnostic>>, Vec<PathBuf>),
    /// Versions of a package (lowercase id), ordered for completion; `None`: the source failed.
    Versions(String, String, Option<Vec<String>>),
    Details(String, String, Option<PackageSummary>),
    /// (source, query, results).
    Search(String, String, Result<Vec<PackageSummary>, String>),
    /// A delay elapsed (debounces).
    Tick,
}

/// One open file's analysis.
struct DocState {
    path: PathBuf,
    /// Buffer version analyzed (`None`: never).
    version: Option<u64>,
    /// Last version seen and when (to wait for typing to pause).
    seen: (u64, Instant),
    ctx_gen: u64,
    latest_gen: u64,
    items: Vec<PackageItem>,
    findings: Vec<Finding>,
    markers: Vec<Marker>,
    diags: Vec<rc_editor::Diagnostic>,
    /// Version whose packages' versions were fetched (after opening or saving).
    fetched: Option<u64>,
    /// The analyzed text was well-formed (`items` match it).
    valid: bool,
}

struct CompletionState {
    doc: u64,
    anchor: usize,
    attr: PackageAttr,
}

struct SearchState {
    query: String,
    due: Instant,
    task: Option<tokio::task::JoinHandle<()>>,
}

struct HoverState {
    doc: u64,
    pos: usize,
    screen: egui::Pos2,
    id: String,
}

pub struct PackageInsight {
    inbox: Inbox<Event>,
    root: Option<PathBuf>,
    projects: Vec<PathBuf>,
    workspace_gen: u64,
    /// First enabled source of `nuget.config` (when the NuGet window has not chosen one).
    config_source: Option<PackageSource>,
    client: Option<NugetClient>,
    /// Ordered versions by lowercase id (`None`: the source failed).
    versions: HashMap<String, Option<Vec<String>>>,
    details: HashMap<String, Option<PackageSummary>>,
    in_flight: HashSet<String>,
    /// Bumped when versions arrive (markers are recomputed).
    latest_gen: u64,
    searches: HashMap<String, Vec<PackageSummary>>,
    search: Option<SearchState>,
    completion: Option<CompletionState>,
    hover: Option<HoverState>,
    hover_actions: Vec<PackageFix>,
    /// A fix waiting for the package's versions (NU1010 with the latest version).
    pending_fix: Option<PackageFix>,
    ctx: Arc<Context>,
    ctx_gen: u64,
    scan_gen: u64,
    scan_due: Option<Instant>,
    scanned: HashSet<PathBuf>,
    /// Problems of the files on disk (shown while they are closed).
    disk: BTreeMap<PathBuf, Vec<rc_editor::Diagnostic>>,
    states: HashMap<u64, DocState>,
    /// What was last put in the Problems window, per file.
    published: HashMap<PathBuf, Vec<rc_editor::Diagnostic>>,
    problems_dirty: bool,
}

impl PackageInsight {
    pub fn new(waker: Waker) -> Self {
        PackageInsight {
            inbox: Inbox::new(waker),
            root: None,
            projects: Vec::new(),
            workspace_gen: 0,
            config_source: None,
            client: None,
            versions: HashMap::new(),
            details: HashMap::new(),
            in_flight: HashSet::new(),
            latest_gen: 0,
            searches: HashMap::new(),
            search: None,
            completion: None,
            hover: None,
            hover_actions: Vec::new(),
            pending_fix: None,
            ctx: Arc::new(Context::default()),
            ctx_gen: 0,
            scan_gen: 0,
            scan_due: None,
            scanned: HashSet::new(),
            disk: BTreeMap::new(),
            states: HashMap::new(),
            published: HashMap::new(),
            problems_dirty: false,
        }
    }

    /// A workspace was opened: reads its package source and its projects' CPM.
    pub fn set_workspace(&mut self, root: &Path, projects: Vec<PathBuf>) {
        let inbox = std::mem::replace(&mut self.inbox, Inbox::new(Waker::noop()));
        // Generations keep counting so results for the previous workspace are dropped.
        let (workspace_gen, scan_gen) = (self.workspace_gen + 1, self.scan_gen);
        *self = PackageInsight { inbox, workspace_gen, scan_gen, ..PackageInsight::new(Waker::noop()) };
        self.root = Some(root.to_path_buf());
        self.projects = projects;
        let (out, generation, root) = (self.inbox.sender(), self.workspace_gen, root.to_path_buf());
        std::thread::spawn(move || {
            if let Some(s) = rc_dotnet::nuget::package_sources(&root).into_iter().find(|s| s.enabled) {
                out.send(Event::Source(generation, s));
            }
        });
        self.schedule_scan(Duration::ZERO);
    }

    /// Project or MSBuild files changed on disk (saves, fixes, other tools).
    pub fn files_changed(&mut self, paths: &[PathBuf]) {
        if self.root.is_some() && paths.iter().any(|p| project_xml::is_package_file(p)) {
            self.schedule_scan(SCAN_DELAY);
        }
    }

    /// Build results replaced the Problems window's entries: show ours again.
    pub fn republish(&mut self) {
        self.published.clear();
        self.problems_dirty = true;
    }

    fn schedule_scan(&mut self, delay: Duration) {
        self.scan_due = Some(Instant::now() + delay);
        self.wake_after(delay);
    }

    fn wake_after(&self, delay: Duration) {
        let out = self.inbox.sender();
        Runtime::spawn(async move {
            tokio::time::sleep(delay).await;
            out.send(Event::Tick);
        });
    }

    fn sender(&self) -> rc_core::inbox::Outbox<Event> {
        self.inbox.sender()
    }

    fn source_key(&self) -> String {
        self.client.as_ref().map(|c| c.source().to_string()).unwrap_or_default()
    }

    /// Switches to `source` (the NuGet window's, else `nuget.config`'s, else nuget.org).
    fn use_source(&mut self, source: Option<&PackageSource>) {
        let (url, creds) = match source.or(self.config_source.as_ref()) {
            Some(s) => (s.url.clone(), s.credentials.clone()),
            None => (rc_dotnet::nuget::NUGET_ORG.to_string(), None),
        };
        if self.client.as_ref().is_some_and(|c| c.source() == url) {
            return;
        }
        let creds = creds.map(|(username, password)| rc_dotnet::nuget_auth::Credential::Basic { username, password });
        self.client = NugetClient::with_credentials(&url, creds).ok();
        self.versions.clear();
        self.details.clear();
        self.in_flight.clear();
        self.searches.clear();
        self.latest_gen += 1;
        for st in self.states.values_mut() {
            st.fetched = None;
        }
    }

    fn fetch_versions(&mut self, id: &str) {
        let key = id.to_ascii_lowercase();
        let Some(client) = self.client.clone() else { return };
        if self.versions.contains_key(&key) || !self.in_flight.insert(format!("v:{key}")) {
            return;
        }
        let (out, id) = (self.sender(), id.to_string());
        Runtime::spawn(async move {
            let v = client.versions(&id).await.ok().map(project_xml::order_versions);
            out.send(Event::Versions(client.source().to_string(), key, v));
        });
    }

    fn fetch_details(&mut self, id: &str) {
        let key = id.to_ascii_lowercase();
        let Some(client) = self.client.clone() else { return };
        if self.details.contains_key(&key) || !self.in_flight.insert(format!("d:{key}")) {
            return;
        }
        let (out, id) = (self.sender(), id.to_string());
        Runtime::spawn(async move {
            let d = client.search(&format!("packageid:{id}"), true, 0, 1).await.ok().and_then(|r| r.packages.into_iter().find(|p| p.id.eq_ignore_ascii_case(&id)));
            out.send(Event::Details(client.source().to_string(), key, d));
        });
    }

    fn latest(&self, id: &str) -> Option<String> {
        self.versions.get(&id.to_ascii_lowercase())?.as_ref().and_then(|v| project_xml::latest_stable(v))
    }

    // ------------------------------------------------------------------ per frame

    /// Applies results, analyzes the open package files and keeps the Problems window up to date.
    pub fn update(&mut self, docs: &mut Docs, problems: &mut BTreeMap<PathBuf, Vec<rc_editor::Diagnostic>>, window_source: Option<&PackageSource>) -> Vec<PackageOutput> {
        let mut out = Vec::new();
        if self.root.is_none() && !docs.list.iter().any(|d| project_xml::is_package_file(&d.path)) {
            return out;
        }
        self.use_source(window_source);
        for ev in self.inbox.drain() {
            self.on_event(ev, docs, &mut out);
        }
        // Open files the last scan did not see (outside the solution).
        if docs.list.iter().any(|d| project_xml::is_package_file(&d.path) && !self.scanned.contains(&d.path)) && self.scan_due.is_none() {
            self.schedule_scan(Duration::ZERO);
        }
        if self.scan_due.is_some_and(|t| Instant::now() >= t) {
            self.scan_due = None;
            self.scan_gen += 1;
            let open: Vec<PathBuf> = docs.list.iter().filter(|d| project_xml::is_package_file(&d.path)).map(|d| d.path.clone()).collect();
            self.scanned.extend(open.iter().cloned());
            let (out, generation, projects) = (self.sender(), self.scan_gen, self.projects.clone());
            std::thread::spawn(move || {
                let (ctx, disk) = scan(projects, open.clone());
                out.send(Event::Scan(generation, Arc::new(ctx), disk, open));
            });
        }
        if let Some(s) = &mut self.search
            && s.task.is_none()
            && Instant::now() >= s.due
        {
            let query = s.query.clone();
            match (self.searches.get(&query.to_ascii_lowercase()).cloned(), self.client.clone()) {
                (Some(results), _) => {
                    self.search = None;
                    self.show_search(docs, &query, results);
                }
                (None, Some(client)) => {
                    let out = self.inbox.sender();
                    s.task = Some(Runtime::spawn(async move {
                        let r = client.search(&query, false, 0, 30).await.map(|r| r.packages).map_err(|e| format!("{e:#}"));
                        out.send(Event::Search(client.source().to_string(), query, r));
                    }));
                }
                (None, None) => self.search = None,
            }
        }
        let open: HashSet<u64> = docs.list.iter().map(|d| d.editor.id).collect();
        let before = self.states.len();
        self.states.retain(|id, _| open.contains(id));
        if self.states.len() != before {
            self.problems_dirty = true;
        }
        for d in docs.list.iter_mut().filter(|d| project_xml::is_package_file(&d.path)) {
            self.refresh(d);
        }
        self.try_show_hover(docs);
        if self.problems_dirty {
            self.problems_dirty = false;
            self.sync_problems(docs, problems);
        }
        out
    }

    fn on_event(&mut self, ev: Event, docs: &mut Docs, out: &mut Vec<PackageOutput>) {
        match ev {
            Event::Tick => {}
            Event::Source(generation, s) => {
                if generation == self.workspace_gen {
                    self.config_source = Some(s);
                }
            }
            Event::Scan(generation, ctx, disk, open) => {
                if generation == self.scan_gen {
                    self.ctx = ctx;
                    self.ctx_gen += 1;
                    self.disk = disk;
                    self.scanned.extend(open);
                    self.problems_dirty = true;
                }
            }
            Event::Versions(source, key, v) => {
                if source != self.source_key() {
                    return;
                }
                self.in_flight.remove(&format!("v:{key}"));
                self.versions.insert(key.clone(), v.clone());
                self.latest_gen += 1;
                // A version completion waiting for them.
                if let Some(cs) = &self.completion
                    && cs.attr != PackageAttr::Include
                    && let Some(d) = docs.list.iter_mut().find(|d| d.editor.id == cs.doc)
                    && d.editor.popups.completion.as_ref().is_some_and(|c| c.anchor == cs.anchor && c.items.is_empty())
                {
                    match v.as_deref() {
                        Some(vs) => self.show_versions(d, vs),
                        None => d.editor.popups.completion = None,
                    }
                }
                if let Some(PackageFix::AddCentral { file, id, version }) = &self.pending_fix
                    && version.is_empty()
                    && id.eq_ignore_ascii_case(&key)
                {
                    let fix = PackageFix::AddCentral { file: file.clone(), id: id.clone(), version: self.latest(id).unwrap_or_default() };
                    self.pending_fix = None;
                    match &fix {
                        PackageFix::AddCentral { version, .. } if version.is_empty() => out.push(PackageOutput::Message(trf("No stable version of {id} was found", &[("id", &key)]))),
                        _ => out.push(PackageOutput::Fix(fix)),
                    }
                }
            }
            Event::Details(source, key, d) => {
                if source == self.source_key() {
                    self.in_flight.remove(&format!("d:{key}"));
                    self.details.insert(key, d);
                }
            }
            Event::Search(source, query, r) => {
                if source != self.source_key() {
                    return;
                }
                if self.search.as_ref().is_some_and(|s| s.query == query) {
                    self.search = None;
                }
                if let Ok(results) = r {
                    self.searches.insert(query.to_ascii_lowercase(), results.clone());
                    self.show_search(docs, &query, results);
                } else if let Some(cs) = &self.completion
                    && let Some(d) = docs.list.iter_mut().find(|d| d.editor.id == cs.doc)
                    && let Some(c) = d.editor.popups.completion.as_mut()
                {
                    c.loading = false;
                }
            }
        }
    }

    /// Re-analyzes `d` when its text (after a pause), the CPM context or the known versions changed.
    fn refresh(&mut self, d: &mut Doc) {
        let id = d.editor.id;
        let version = d.editor.buffer.version();
        let now = Instant::now();
        let st = self.states.entry(id).or_insert_with(|| DocState {
            path: d.path.clone(),
            version: None,
            seen: (version, now.checked_sub(ANALYZE_DELAY).unwrap_or(now)),
            ctx_gen: 0,
            latest_gen: 0,
            items: Vec::new(),
            findings: Vec::new(),
            markers: Vec::new(),
            diags: Vec::new(),
            fetched: None,
            valid: false,
        });
        if st.seen.0 != version {
            st.seen = (version, now);
        }
        let text_changed = st.version != Some(version);
        if text_changed && now - st.seen.1 < ANALYZE_DELAY {
            let wait = ANALYZE_DELAY - (now - st.seen.1);
            self.wake_after(wait);
            return;
        }
        if !text_changed && st.ctx_gen == self.ctx_gen && st.latest_gen == self.latest_gen {
            return;
        }
        let text = d.editor.buffer.rope().to_string();
        if text_changed {
            st.version = Some(version);
            match project_xml::package_items(&text) {
                Some(items) => {
                    st.items = items;
                    st.valid = true;
                }
                None => st.valid = false,
            }
        }
        // Not well-formed while typing: keep what the editor shows.
        if !st.valid {
            return;
        }
        st.ctx_gen = self.ctx_gen;
        st.latest_gen = self.latest_gen;
        let path = d.path.clone();
        let findings = analyze(&path, &text, &st.items, &self.ctx);
        let mut markers = Vec::new();
        for item in &st.items {
            let Some(eff) = self.ctx.effective(&path, &text, item) else { continue };
            let Some(latest) = self.versions.get(&item.id.to_ascii_lowercase()).and_then(|v| v.as_ref()).and_then(|v| project_xml::latest_stable(v)) else { continue };
            if !project_xml::is_plain_version(&eff.version) || compare_versions(&latest, &eff.version).is_le() {
                continue;
            }
            let line_end = text[item.range.start..].find('\n').map_or(text.len(), |i| item.range.start + i);
            markers.push(Marker { line: item.line, at: item.range.end.min(line_end), fix: update_fix(&path, item, &eff.source, &latest), latest });
        }
        let diags = diagnostics(&text, &findings);
        let st = self.states.get_mut(&id).expect("inserted above");
        if st.diags != diags {
            self.problems_dirty = true;
        }
        st.diags = diags;
        st.findings = findings;
        d.editor.gutter_icons.retain(|_, icon| *icon != GutterIcon::PackageUpdate);
        d.editor.inlays.clear();
        for m in &markers {
            d.editor.gutter_icons.insert(m.line, GutterIcon::PackageUpdate);
            d.editor.inlays.push(InlayHint { pos: char_of(&text, m.at), label: trf("{version} available", &[("version", &m.latest)]), pad_left: true, pad_right: false });
        }
        st.markers = markers;
        // Versions after opening or saving the file (never while typing).
        if !d.is_modified() && st.fetched != Some(version) {
            st.fetched = Some(version);
            let ids: Vec<String> = st.items.iter().map(|i| i.id.clone()).collect();
            for id in ids {
                self.fetch_versions(&id);
            }
        }
    }

    /// Puts the problems of the open files (live) and the closed ones (from disk) in the Problems
    /// window, and the build's next to ours in the editors.
    fn sync_problems(&mut self, docs: &mut Docs, problems: &mut BTreeMap<PathBuf, Vec<rc_editor::Diagnostic>>) {
        let mut want: BTreeMap<PathBuf, Vec<rc_editor::Diagnostic>> = self.disk.clone();
        for st in self.states.values().filter(|s| s.version.is_some()) {
            want.insert(st.path.clone(), st.diags.clone());
        }
        for p in self.published.keys() {
            want.entry(p.clone()).or_default();
        }
        for (path, list) in want {
            let old = self.published.remove(&path).unwrap_or_default();
            let entry = problems.entry(path.clone()).or_default();
            entry.retain(|d| !old.contains(d));
            for d in &list {
                if !entry.contains(d) {
                    entry.push(d.clone());
                }
            }
            if entry.is_empty() {
                problems.remove(&path);
            }
            if !list.is_empty() {
                self.published.insert(path, list);
            }
        }
        for d in docs.list.iter_mut() {
            let Some(st) = self.states.get(&d.editor.id).filter(|s| s.version.is_some()) else { continue };
            let mut all = st.diags.clone();
            all.extend(problems.get(&d.path).into_iter().flatten().filter(|x| !st.diags.contains(x)).cloned());
            d.editor.set_diagnostics(all);
        }
    }

    // ------------------------------------------------------------------ editor events

    /// Reacts to what a package file's editor did this frame.
    pub fn on_editor_events(&mut self, d: &mut Doc, events: &[EditorEvent]) -> Vec<PackageOutput> {
        let mut out = Vec::new();
        for ev in events {
            match ev {
                EditorEvent::Typed(_) => self.complete(d, false),
                EditorEvent::Changed => self.fix_replace_ranges(d),
                EditorEvent::HoverAt { pos, screen } => self.request_hover(d, *pos, *screen),
                EditorEvent::HoverEnd => self.hover = None,
                EditorEvent::HoverAction(i) => {
                    if let Some(fix) = self.hover_actions.get(*i).cloned() {
                        out.push(PackageOutput::Fix(fix));
                    }
                }
                EditorEvent::GutterIconClicked { line, icon: GutterIcon::PackageUpdate } => {
                    // The popup opens at the caret: put it on the package.
                    if let Some(item) = self.states.get(&d.editor.id).and_then(|s| s.items.iter().find(|i| i.line == *line)) {
                        let text = d.editor.buffer.rope().to_string();
                        let pos = char_of(&text, item.id_range.start);
                        d.editor.set_selection(rc_text::Selection::point(pos));
                    }
                    out.extend(self.actions_at(d, *line));
                }
                _ => {}
            }
        }
        out
    }

    /// ⌥⏎ on a package file: the fixes and updates of the caret's line.
    pub fn context_actions(&mut self, d: &Doc) -> Vec<PackageOutput> {
        let line = d.editor.caret_line_col().0;
        let out = self.actions_at(d, line);
        if out.is_empty() { vec![PackageOutput::Message(tr("No context actions here").into())] } else { out }
    }

    fn actions_at(&self, d: &Doc, line: usize) -> Vec<PackageOutput> {
        let Some(st) = self.states.get(&d.editor.id) else { return Vec::new() };
        let mut list = Vec::new();
        for f in st.findings.iter().filter(|f| f.line == line) {
            if let Some(mut fix) = f.fix.clone() {
                if let PackageFix::AddCentral { id, version, .. } = &mut fix
                    && let Some(latest) = self.latest(id)
                {
                    *version = latest;
                }
                list.push(fix.action(true));
            }
        }
        for m in st.markers.iter().filter(|m| m.line == line) {
            list.push(m.fix.action(false));
        }
        if let Some(item) = st.items.iter().find(|i| i.line == line) {
            list.push(PackageFix::Open { id: item.id.clone() }.action(false));
        }
        if list.is_empty() { Vec::new() } else { vec![PackageOutput::Actions(list)] }
    }

    // ------------------------------------------------------------------ completion

    /// Completion in a package id or version (typing, or ⌃Space when `explicit`).
    pub fn complete(&mut self, d: &mut Doc, explicit: bool) {
        let text = d.editor.buffer.rope().to_string();
        let caret = d.editor.caret();
        let Some(cx) = project_xml::context_at(&text, byte_of(&text, caret)) else {
            if explicit {
                d.editor.popups.completion = None;
            }
            return;
        };
        let anchor = char_of(&text, cx.value.start);
        let reopen = explicit || d.editor.popups.completion.as_ref().is_none_or(|c| c.anchor != anchor);
        match cx.attr {
            PackageAttr::Include => {
                let query = cx.typed.trim().to_string();
                if reopen {
                    // What was found for a shorter query stays until the new results arrive.
                    let known = self.searches.get(&query.to_ascii_lowercase()).cloned();
                    let shown = known.clone().unwrap_or_else(|| self.searches.iter().filter(|(q, _)| query.to_ascii_lowercase().starts_with(q.as_str())).max_by_key(|(q, _)| q.len()).map(|(_, r)| r.clone()).unwrap_or_default());
                    self.completion = Some(CompletionState { doc: d.editor.id, anchor, attr: cx.attr });
                    self.set_search_items(d, &shown, known.is_none());
                }
                if !self.searches.contains_key(&query.to_ascii_lowercase()) {
                    if let Some(t) = self.search.take().and_then(|s| s.task) {
                        t.abort();
                    }
                    let due = Instant::now() + if explicit { Duration::ZERO } else { SEARCH_DELAY };
                    self.search = Some(SearchState { query, due, task: None });
                    self.wake_after(if explicit { Duration::ZERO } else { SEARCH_DELAY });
                } else if !reopen {
                    let results = self.searches[&query.to_ascii_lowercase()].clone();
                    self.set_search_items(d, &results, false);
                }
            }
            PackageAttr::Version | PackageAttr::VersionOverride => {
                if !reopen {
                    return;
                }
                let Some(id) = cx.package_id else { return };
                self.completion = Some(CompletionState { doc: d.editor.id, anchor, attr: cx.attr });
                match self.versions.get(&id.to_ascii_lowercase()).cloned() {
                    Some(Some(vs)) => self.show_versions(d, &vs),
                    Some(None) if explicit => self.set_items(d, Vec::new(), false),
                    Some(None) => {}
                    None => {
                        self.set_items(d, Vec::new(), true);
                        self.fetch_versions(&id);
                    }
                }
            }
        }
    }

    /// Opens or refreshes the list with `items`, replacing the attribute's whole value.
    fn set_items(&mut self, d: &mut Doc, mut items: Vec<CompletionItem>, loading: bool) {
        let Some(cs) = &self.completion else { return };
        let text = d.editor.buffer.rope().to_string();
        let caret = d.editor.caret();
        let Some(cx) = project_xml::context_at(&text, byte_of(&text, caret)) else { return };
        let (anchor, end) = (char_of(&text, cx.value.start), char_of(&text, cx.value.end));
        if anchor != cs.anchor {
            return;
        }
        for it in &mut items {
            it.replace = Some((anchor, end));
        }
        let prefix = d.editor.buffer.rope().slice(anchor..caret.max(anchor)).to_string();
        let mut c = Completion::new(anchor, items, loading);
        c.keep_order = true;
        c.filter(prefix.trim());
        if c.filtered.is_empty() && !loading && !c.items.is_empty() {
            d.editor.popups.completion = None;
            return;
        }
        d.editor.popups.completion = Some(c);
    }

    fn set_search_items(&mut self, d: &mut Doc, results: &[PackageSummary], loading: bool) {
        let items = results
            .iter()
            .enumerate()
            .map(|(i, p)| CompletionItem {
                label: p.id.clone(),
                // The description's first line, greyed after the id.
                detail: p.description.lines().next().unwrap_or("").trim().to_string(),
                type_text: p.version.clone(),
                icon: ico::PACKAGE,
                insert: p.id.clone(),
                replace: None,
                deprecated: false,
                data: i,
            })
            .collect();
        self.set_items(d, items, loading);
    }

    fn show_versions(&mut self, d: &mut Doc, versions: &[String]) {
        let latest = project_xml::latest_stable(versions);
        let items = versions
            .iter()
            .enumerate()
            .map(|(i, v)| CompletionItem {
                label: v.clone(),
                detail: String::new(),
                type_text: if Some(v) == latest.as_ref() {
                    tr("latest").into()
                } else if v.contains('-') {
                    tr("prerelease").into()
                } else {
                    String::new()
                },
                icon: ico::TAG,
                insert: v.clone(),
                replace: None,
                deprecated: false,
                data: i,
            })
            .collect();
        self.set_items(d, items, false);
    }

    /// Search results arrived: refresh the open list (when the caret is still in that id).
    fn show_search(&mut self, docs: &mut Docs, query: &str, results: Vec<PackageSummary>) {
        let Some(cs) = &self.completion else { return };
        if cs.attr != PackageAttr::Include {
            return;
        }
        let Some(d) = docs.list.iter_mut().find(|d| d.editor.id == cs.doc) else { return };
        if !d.editor.popups.completion.as_ref().is_some_and(|c| c.anchor == cs.anchor) {
            return;
        }
        let text = d.editor.buffer.rope().to_string();
        let typed = project_xml::context_at(&text, byte_of(&text, d.editor.caret())).map(|c| c.typed.trim().to_ascii_lowercase()).unwrap_or_default();
        if !typed.starts_with(&query.to_ascii_lowercase()) {
            return;
        }
        let pending = self.search.is_some();
        self.set_search_items(d, &results, pending);
    }

    /// Typing moved the end of the value: accepting must still replace all of it.
    fn fix_replace_ranges(&mut self, d: &mut Doc) {
        if d.editor.popups.completion.is_none() {
            return;
        }
        let text = d.editor.buffer.rope().to_string();
        let Some(cx) = project_xml::context_at(&text, byte_of(&text, d.editor.caret())) else { return };
        let (anchor, end) = (char_of(&text, cx.value.start), char_of(&text, cx.value.end));
        let Some(c) = d.editor.popups.completion.as_mut() else { return };
        if anchor == c.anchor {
            for it in &mut c.items {
                it.replace = Some((anchor, end));
            }
        }
    }

    // ------------------------------------------------------------------ hover

    fn request_hover(&mut self, d: &mut Doc, pos: usize, screen: egui::Pos2) {
        self.hover = None;
        let Some(st) = self.states.get(&d.editor.id) else { return };
        let text = d.editor.buffer.rope().to_string();
        let b = byte_of(&text, pos);
        let inside = |r: &Option<Range<usize>>| r.as_ref().is_some_and(|r| r.start <= b && b <= r.end);
        let item = st.items.iter().find(|i| inside(&Some(i.id_range.clone())) || inside(&i.version_range) || inside(&i.override_range));
        match item {
            Some(i) => {
                let id = i.id.clone();
                self.fetch_versions(&id);
                self.fetch_details(&id);
                self.hover = Some(HoverState { doc: d.editor.id, pos, screen, id });
            }
            None => {
                if let Some(x) = d.editor.diagnostic_at(pos) {
                    let problem = Some((x.severity, x.message.clone()));
                    d.editor.popups.hover = Some(Hover { pos, text: String::new(), problem, screen, actions: Vec::new() });
                }
            }
        }
    }

    /// F1 on a package file: the hover of the caret's package, under the caret.
    pub fn quick_doc(&mut self, d: &mut Doc, screen: egui::Pos2) {
        self.request_hover(d, d.editor.caret(), screen);
    }

    /// Shows the waiting hover once the package's versions and details are known.
    fn try_show_hover(&mut self, docs: &mut Docs) {
        let Some(h) = &self.hover else { return };
        let key = h.id.to_ascii_lowercase();
        if !self.versions.contains_key(&key) || !self.details.contains_key(&key) {
            return;
        }
        let h = self.hover.take().expect("checked");
        let Some(d) = docs.list.iter_mut().find(|d| d.editor.id == h.doc) else { return };
        let Some(st) = self.states.get(&h.doc) else { return };
        let text = d.editor.buffer.rope().to_string();
        let b = byte_of(&text, h.pos);
        let Some(item) = st.items.iter().find(|i| i.id.eq_ignore_ascii_case(&h.id) && i.range.start <= b && b <= i.range.end) else { return };
        let mut lines = vec![format!("`{}`", item.id)];
        if let Some(desc) = self.details.get(&key).and_then(|d| d.as_ref()).map(|d| d.description.trim().to_string()).filter(|s| !s.is_empty()) {
            let short: String = desc.chars().take(400).collect();
            lines.push(if short.len() < desc.len() { format!("{short}…") } else { short });
        }
        lines.push("---".into());
        let code = |v: &str| format!("`{v}`");
        let shown = |p: &Path| self.root.as_deref().and_then(|r| p.strip_prefix(r).ok()).unwrap_or(p).display().to_string();
        if let Some(eff) = self.ctx.effective(&d.path, &text, item) {
            lines.push(trf("Version in effect: {version}", &[("version", &code(&eff.version))]));
            match &eff.source {
                Source::Override(central) => {
                    lines.push(tr("Set by VersionOverride in this project").into());
                    if let Some((v, f)) = central {
                        lines.push(trf("Central version: {version} in {file}", &[("version", &code(v)), ("file", &code(&shown(f)))]));
                    }
                }
                Source::Central(f) => lines.push(trf("Central version from {file}", &[("file", &code(&shown(f)))])),
                Source::Global(f) => lines.push(trf("Global package reference in {file}", &[("file", &code(&shown(f)))])),
                Source::Own => {
                    let users = self.ctx.readers(&d.path).iter().filter(|p| p.reference(&item.id).is_some()).count();
                    if users > 0 {
                        lines.push(trf("Used by {n} projects", &[("n", &users)]));
                    }
                }
                Source::Reference => {}
            }
        }
        match self.versions.get(&key).and_then(|v| v.as_ref()) {
            Some(vs) => match project_xml::latest_stable(vs) {
                Some(latest) => lines.push(trf("Latest stable version: {version}", &[("version", &code(&latest))])),
                None => lines.push(tr("No stable version published").into()),
            },
            None => lines.push(tr("The package source could not be reached").into()),
        }
        let mut actions = Vec::new();
        self.hover_actions.clear();
        if let Some(m) = st.markers.iter().find(|m| m.line == item.line) {
            actions.push((ico::UP, trf("Update to {version}", &[("version", &m.latest)])));
            self.hover_actions.push(m.fix.clone());
        }
        actions.push((ico::PACKAGE, tr("Open in NuGet Window").to_string()));
        self.hover_actions.push(PackageFix::Open { id: item.id.clone() });
        let problem = d.editor.diagnostic_at(h.pos).map(|x| (x.severity, x.message.clone()));
        d.editor.popups.hover = Some(Hover { pos: h.pos, text: lines.join("\n"), problem, screen: h.screen, actions });
    }

    // ------------------------------------------------------------------ fixes

    /// Applies a fix (not [`PackageFix::Open`], which the app handles); the message to show.
    pub fn apply(&mut self, docs: &mut Docs, fix: &PackageFix) -> Result<Option<String>, String> {
        let name = |p: &Path| p.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        let edit = |docs: &mut Docs, path: &Path, f: &mut dyn FnMut(&str) -> anyhow::Result<Vec<cpm::TextEdit>>| -> Result<(), String> {
            docs.edit_file(path, |t| f(t).map(|v| v.into_iter().map(|e| (e.range, e.text)).collect()).map_err(|e| format!("{}: {e:#}", name(path)))).map(|_| ())
        };
        let msg = match fix {
            PackageFix::SetCentral { file, id, condition, version } => {
                edit(docs, file, &mut |t| cpm::set_central_version(t, id, condition.as_deref(), version))?;
                trf("Central version of {id} set to {version}", &[("id", id), ("version", version)])
            }
            PackageFix::SetReference { project, id, name: attr, version } => {
                edit(docs, project, &mut |t| cpm::set_reference_metadata(t, id, attr, version))?;
                trf("Updated {id} to {version}", &[("id", id), ("version", version)])
            }
            PackageFix::MoveVersion { project, file, id } => {
                let mut moved = String::new();
                edit(docs, project, &mut |t| {
                    let (edits, v) = cpm::remove_reference_version(t, id)?;
                    moved = v;
                    Ok(edits)
                })?;
                edit(docs, file, &mut |t| if cpm::CentralFile::parse(file, t)?.unconditional(id).is_some() { Ok(Vec::new()) } else { cpm::add_central_version(t, id, &moved) })?;
                trf("Moved the version of {id} to {file}", &[("id", id), ("file", &name(file))])
            }
            PackageFix::AddCentral { file, id, version } if version.is_empty() => {
                if let Some(latest) = self.latest(id) {
                    return self.apply(docs, &PackageFix::AddCentral { file: file.clone(), id: id.clone(), version: latest });
                }
                // Applied when the versions arrive.
                self.pending_fix = Some(fix.clone());
                self.versions.remove(&id.to_ascii_lowercase());
                self.fetch_versions(id);
                return Ok(None);
            }
            PackageFix::AddCentral { file, id, version } => {
                edit(docs, file, &mut |t| cpm::add_central_version(t, id, version))?;
                trf("Added {id} {version} to {file}", &[("id", id), ("version", version), ("file", &name(file))])
            }
            PackageFix::DropCentral { file, id } => {
                edit(docs, file, &mut |t| cpm::remove_central_version(t, id))?;
                trf("Removed {id} from {file}", &[("id", id), ("file", &name(file))])
            }
            PackageFix::Open { .. } => return Ok(None),
        };
        self.schedule_scan(Duration::ZERO);
        Ok(Some(msg))
    }

    #[cfg(test)]
    pub fn idle_for_test(&self) -> bool {
        self.scan_due.is_none() && self.search.is_none() && self.in_flight.is_empty() && self.hover.is_none() && self.ctx_gen > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn central_shop() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::Builder::new().prefix("dotcraft-pkg").tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap().join("CentralShop");
        copy(&crate::screens::fixtures().join("CentralShop"), &root);
        (tmp, root)
    }

    fn copy(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for e in std::fs::read_dir(from).unwrap().flatten() {
            if e.file_name() == "bin" || e.file_name() == "obj" {
                continue;
            }
            if e.file_type().unwrap().is_dir() {
                copy(&e.path(), &to.join(e.file_name()));
            } else {
                std::fs::copy(e.path(), to.join(e.file_name())).unwrap();
            }
        }
    }

    fn projects(root: &Path) -> Vec<PathBuf> {
        vec![root.join("src/CentralShop.Cli/CentralShop.Cli.csproj"), root.join("src/CentralShop.Core/CentralShop.Core.csproj")]
    }

    #[test]
    fn problems_have_the_ranges_of_their_items() {
        let (_tmp, root) = central_shop();
        let core = root.join("src/CentralShop.Core/CentralShop.Core.csproj");
        let text = std::fs::read_to_string(&core).unwrap().replace(r#"<PackageReference Include="Humanizer.Core" />"#, "<PackageReference Include=\"Humanizer.Core\" Version=\"2.14.1\" />\n    <PackageReference Include=\"Serilog\" />");
        std::fs::write(&core, &text).unwrap();
        let (ctx, disk) = scan(projects(&root), Vec::new());
        let items = project_xml::package_items(&text).unwrap();
        let found = analyze(&core, &text, &items, &ctx);
        let nu1008 = found.iter().find(|f| f.code == Some("NU1008")).unwrap();
        assert_eq!(&text[nu1008.range.clone()], r#"Version="2.14.1""#);
        assert_eq!(nu1008.line, line_of(&text, text.find("Humanizer").unwrap()));
        assert!(matches!(&nu1008.fix, Some(PackageFix::MoveVersion { file, .. }) if file.ends_with("Directory.Packages.props")));
        let nu1010 = found.iter().find(|f| f.code == Some("NU1010")).unwrap();
        assert_eq!(&text[nu1010.range.clone()], "Serilog");
        // The unused Dapper version in the central file, as chars of its text.
        let central = root.join("Directory.Packages.props");
        let ctext = std::fs::read_to_string(&central).unwrap();
        let unused = &disk[&central];
        assert_eq!(unused.len(), 1);
        let chars: String = ctext.chars().skip(unused[0].start).take(unused[0].end - unused[0].start).collect();
        assert_eq!((chars.as_str(), unused[0].unnecessary), ("Dapper", true));
        assert_eq!(disk[&core].iter().filter_map(|d| d.code.as_deref()).collect::<Vec<_>>(), ["NU1008", "NU1010"]);
    }

    #[test]
    fn effective_versions_say_where_they_come_from() {
        let (_tmp, root) = central_shop();
        let (ctx, _) = scan(projects(&root), Vec::new());
        let cli = root.join("src/CentralShop.Cli/CentralShop.Cli.csproj");
        let text = std::fs::read_to_string(&cli).unwrap();
        let items = project_xml::package_items(&text).unwrap();
        let eff = ctx.effective(&cli, &text, &items[0]).unwrap();
        assert_eq!(eff.version, "13.0.3");
        assert!(matches!(&eff.source, Source::Override(Some((v, f))) if v == "13.0.1" && f.ends_with("Directory.Packages.props")));
        assert!(matches!(update_fix(&cli, &items[0], &eff.source, "13.0.4"), PackageFix::SetReference { name, .. } if name == "VersionOverride"));
        let core = root.join("src/CentralShop.Core/CentralShop.Core.csproj");
        let text = std::fs::read_to_string(&core).unwrap();
        let items = project_xml::package_items(&text).unwrap();
        let eff = ctx.effective(&core, &text, &items[0]).unwrap();
        assert!(eff.version == "2.14.1" && matches!(eff.source, Source::Central(_)));
        assert!(matches!(update_fix(&core, &items[0], &eff.source, "3.0.0"), PackageFix::SetCentral { file, condition: None, .. } if file.ends_with("Directory.Packages.props")));
    }

    #[test]
    fn fixes_round_trip_through_context_actions() {
        let fix = PackageFix::AddCentral { file: "/a/Directory.Packages.props".into(), id: "Serilog".into(), version: "4.0.0".into() };
        let action = fix.action(true);
        assert_eq!(fix_of(&action), Some(fix));
        let other = lsp::CodeActionOrCommand::Command(lsp::Command { title: "x".into(), command: "roslyn.x".into(), arguments: None });
        assert_eq!(fix_of(&other), None);
    }
}
