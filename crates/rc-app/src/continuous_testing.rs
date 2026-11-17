//! Run affected .NET tests after a save or successful build, or repeat a chosen test session.

use crate::runner::{Runner, RunnerOutput};
use rc_core::session::{ContinuousMode, ContinuousScope, ContinuousTestingOptions};
use rc_core::{Inbox, Runtime, Waker};
use rc_dotnet::{ProjectInfo, TestFilter, TestRequest};
use rc_kit::i18n::{tr, trf};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const SAVE_DELAY: Duration = Duration::from_millis(400);

#[derive(Default)]
struct ProjectGraph {
    refs: BTreeMap<PathBuf, Vec<PathBuf>>,
    tests: Vec<PathBuf>,
}

impl ProjectGraph {
    fn load(projects: Vec<PathBuf>, tests: Vec<PathBuf>) -> Self {
        let refs = projects.into_iter().filter_map(|project| {
            ProjectInfo::load(&project).ok().map(|info| (project, info.project_refs))
        }).collect();
        Self { refs, tests }
    }

    fn affected(&self, changed: &BTreeSet<PathBuf>) -> Vec<PathBuf> {
        let mut owners = BTreeSet::new();
        let mut unknown = false;
        for path in changed {
            if let Some(project) = self.refs.keys()
                .filter(|project| *project == path || project.parent().is_some_and(|dir| path.starts_with(dir)))
                .max_by_key(|project| project.as_os_str().len())
            {
                owners.insert(project.clone());
            } else {
                // Shared props, linked source files and solution changes may affect any test.
                unknown = true;
            }
        }
        if unknown {
            return self.tests.clone();
        }
        self.tests.iter().filter(|test| owners.iter().any(|owner| self.depends_on(test, owner))).cloned().collect()
    }

    fn depends_on(&self, test: &Path, owner: &Path) -> bool {
        let mut stack = vec![test.to_path_buf()];
        let mut visited = BTreeSet::new();
        while let Some(project) = stack.pop() {
            if project == owner { return true; }
            if visited.insert(project.clone()) {
                stack.extend(self.refs.get(&project).into_iter().flatten().cloned());
            }
        }
        false
    }
}

pub struct ContinuousTesting {
    pub options: ContinuousTestingOptions,
    pub status: String,
    graph: Option<ProjectGraph>,
    graph_inbox: Inbox<ProjectGraph>,
    graph_task: Option<tokio::task::JoinHandle<()>>,
    changed: BTreeSet<PathBuf>,
    last_save: Option<Instant>,
    last_build: Option<Instant>,
    trigger_pending: bool,
    queue: VecDeque<(PathBuf, Option<TestFilter>)>,
    automatic_running: bool,
}

impl ContinuousTesting {
    pub fn new(waker: Waker) -> Self {
        Self {
            options: ContinuousTestingOptions::default(),
            status: String::new(),
            graph: None,
            graph_inbox: Inbox::new(waker),
            graph_task: None,
            changed: BTreeSet::new(),
            last_save: None,
            last_build: None,
            trigger_pending: false,
            queue: VecDeque::new(),
            automatic_running: false,
        }
    }

    pub fn set_workspace(&mut self, projects: Vec<PathBuf>, tests: Vec<PathBuf>, options: ContinuousTestingOptions) {
        self.reset();
        self.options = options;
        let out = self.graph_inbox.sender();
        self.graph_task = Some(Runtime::spawn(async move {
            if let Ok(graph) = tokio::task::spawn_blocking(move || ProjectGraph::load(projects, tests)).await {
                out.send(graph);
            }
        }));
    }

    pub fn reset(&mut self) {
        if let Some(task) = self.graph_task.take() { task.abort(); }
        self.graph_inbox.drain();
        self.graph = None;
        self.changed.clear();
        self.last_save = None;
        self.last_build = None;
        self.trigger_pending = false;
        self.queue.clear();
        self.automatic_running = false;
        self.status.clear();
        self.options = ContinuousTestingOptions::default();
    }

    pub fn note_saved(&mut self, path: &Path) {
        if !relevant(path) || self.options.mode == ContinuousMode::Off { return; }
        self.changed.insert(path.to_path_buf());
        if self.options.mode == ContinuousMode::OnSave {
            self.last_save = Some(Instant::now());
        }
    }

    pub fn note_disk_change(&mut self, path: &Path) {
        if self.options.mode == ContinuousMode::OnBuild && relevant(path) {
            self.changed.insert(path.to_path_buf());
        }
    }

    pub fn observe_build(&mut self, started: Instant, success: bool) {
        if self.last_build == Some(started) { return; }
        self.last_build = Some(started);
        if self.options.mode == ContinuousMode::OnBuild && success && !self.changed.is_empty() {
            self.trigger_pending = true;
        }
    }

    pub fn select_session(&mut self, request: &TestRequest) {
        self.options.session_target = Some(request.target.clone());
        self.options.session_filter = request.filter.as_ref().map(TestFilter::expression);
    }

    pub fn cancel(&mut self) {
        self.queue.clear();
        self.changed.clear();
        self.last_save = None;
        self.trigger_pending = false;
        if self.automatic_running {
            self.status = tr("Continuous testing cancelled").into();
        }
        self.automatic_running = false;
    }

    pub fn poll(&mut self, runner: &mut Runner) -> Vec<RunnerOutput> {
        for graph in self.graph_inbox.drain() {
            self.graph = Some(graph);
            self.graph_task.take();
        }
        if self.options.mode == ContinuousMode::Off {
            if self.automatic_running {
                if let Some(tests) = &mut runner.tests
                    && let Some(handle) = tests.handle.take()
                {
                    handle.cancel();
                }
            }
            self.queue.clear();
            self.changed.clear();
            self.last_save = None;
            self.trigger_pending = false;
            self.automatic_running = false;
            self.status.clear();
            return Vec::new();
        }
        if self.options.mode == ContinuousMode::OnSave
            && self.last_save.is_some_and(|saved| saved.elapsed() >= SAVE_DELAY)
        {
            self.last_save = None;
            self.trigger_pending = true;
        }
        if self.automatic_running {
            if runner.tests.as_ref().is_some_and(|tests| tests.running) { return Vec::new(); }
            self.automatic_running = false;
            if self.queue.is_empty() && !self.trigger_pending {
                self.status = tr("Continuous testing complete").into();
            }
        }
        if runner.tests.as_ref().is_some_and(|tests| tests.running) { return Vec::new(); }
        if let Some((target, filter)) = self.queue.pop_front() {
            return self.start_next(runner, target, filter);
        }
        if !self.trigger_pending { return Vec::new(); }
        let Some(graph) = &self.graph else { return Vec::new() };
        self.trigger_pending = false;
        let runs = match self.options.scope {
            ContinuousScope::Affected => graph.affected(&self.changed).into_iter().map(|project| (project, None)).collect(),
            ContinuousScope::Session => self.options.session_target.clone().map(|target| {
                let filter = self.options.session_filter.clone().map(TestFilter::Raw);
                vec![(target, filter)]
            }).unwrap_or_default(),
        };
        self.changed.clear();
        self.queue = runs.into();
        if let Some((target, filter)) = self.queue.pop_front() {
            self.start_next(runner, target, filter)
        } else {
            self.status = if self.options.scope == ContinuousScope::Session {
                tr("Choose a test session in the Tests window").into()
            } else {
                tr("No affected test projects").into()
            };
            Vec::new()
        }
    }

    fn start_next(&mut self, runner: &mut Runner, target: PathBuf, filter: Option<TestFilter>) -> Vec<RunnerOutput> {
        self.automatic_running = true;
        let name = target.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_else(|| target.display().to_string());
        self.status = trf("Continuous testing: {project}", &[("project", &name)]);
        runner.run_tests(&target, filter)
    }

    pub fn waiting_for_save(&self) -> bool {
        self.options.mode == ContinuousMode::OnSave && self.last_save.is_some()
    }
}

fn relevant(path: &Path) -> bool {
    matches!(path.extension().and_then(|extension| extension.to_str()), Some("cs" | "csproj" | "props" | "targets" | "sln" | "slnx"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_change_selects_all_dependent_test_projects() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/Shop");
        let core = fixture.join("src/Shop.Core/Shop.Core.csproj");
        let cli = fixture.join("src/Shop.Cli/Shop.Cli.csproj");
        let tests = ["Shop.Core.XunitTests", "Shop.Core.NunitTests", "Shop.Core.MSTests"]
            .map(|name| fixture.join(format!("tests/{name}/{name}.csproj"))).to_vec();
        let mut projects = tests.clone();
        projects.extend([core.clone(), cli]);
        let graph = ProjectGraph::load(projects, tests.clone());
        let affected = graph.affected(&BTreeSet::from([fixture.join("src/Shop.Core/Price.cs")]));
        assert_eq!(affected, tests);
    }

    #[test]
    fn unrelated_changes_run_no_tests_and_unknown_shared_files_run_all() {
        let graph = ProjectGraph {
            refs: BTreeMap::from([(PathBuf::from("/repo/App/App.csproj"), vec![]), (PathBuf::from("/repo/Test/Test.csproj"), vec![])]),
            tests: vec![PathBuf::from("/repo/Test/Test.csproj")],
        };
        assert!(graph.affected(&BTreeSet::from([PathBuf::from("/repo/App/Program.cs")])).is_empty());
        assert_eq!(graph.affected(&BTreeSet::from([PathBuf::from("/repo/Directory.Build.props")])).len(), 1);
    }

    #[test]
    fn save_and_build_modes_collect_only_relevant_changes() {
        let mut continuous = ContinuousTesting::new(Waker::noop());
        continuous.options.mode = ContinuousMode::OnSave;
        continuous.note_saved(Path::new("/repo/README.md"));
        assert!(!continuous.waiting_for_save());
        continuous.note_saved(Path::new("/repo/App/Program.cs"));
        assert!(continuous.waiting_for_save());
        assert!(continuous.changed.contains(Path::new("/repo/App/Program.cs")));

        continuous.reset();
        continuous.options.mode = ContinuousMode::OnBuild;
        continuous.note_disk_change(Path::new("/repo/App/Program.cs"));
        continuous.observe_build(Instant::now(), true);
        assert!(continuous.trigger_pending);
    }
}
