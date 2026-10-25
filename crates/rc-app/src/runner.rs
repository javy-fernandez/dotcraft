//! Build, run and test: run configurations derived from the solution, the processes they
//! start (through rc-dotnet) and their consoles, and the test markers of the editor gutter.

use rc_core::{Inbox, Waker};
use rc_dotnet::{BuildAction, BuildEvent, BuildRequest, BuildResult, Dotnet, OpHandle, OutputLine, ProjectInfo, RunEvent, RunHandle, RunRequest, Solution, Stream, TestEvent, TestFilter, TestOutcome, TestRequest, TestResult};
use rc_kit::i18n::{tr, trf};
use rc_term::TermSession;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfigKind {
    /// An executable project, with an optional launch profile.
    App { project: PathBuf, profile: Option<String>, web: bool },
    /// Every test of a solution or test project.
    Tests { target: PathBuf },
    /// Several application configurations started together (by name).
    Compound { members: Vec<String> },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunConfig {
    pub name: String,
    pub kind: ConfigKind,
}

impl RunConfig {
    pub fn icon(&self) -> rc_kit::Ico {
        match &self.kind {
            ConfigKind::App { web: true, .. } => rc_kit::ico::CONFIG_WEB,
            ConfigKind::App { .. } => rc_kit::ico::CONFIG_APP,
            ConfigKind::Tests { .. } => rc_kit::ico::RUN_ALL,
            ConfigKind::Compound { .. } => rc_kit::ico::TW_SERVICES,
        }
    }
}

/// A console fed with process output (an output-only terminal, so ANSI colours work).
pub struct Console {
    pub term: TermSession,
}

impl Console {
    pub fn new(waker: &Waker) -> Self {
        Console { term: TermSession::output_only(160, 40, waker.clone()) }
    }

    /// Raw program output (may end mid-line).
    pub fn raw(&self, text: &str, stream: Stream) {
        let mut s = text.replace("\r\n", "\n").replace('\n', "\r\n");
        if stream == Stream::Stderr && !s.contains('\x1b') {
            s = format!("\x1b[31m{s}\x1b[0m");
        }
        self.term.feed(s.as_bytes());
    }

    pub fn line(&self, line: &OutputLine) {
        let mut s = line.text.replace('\n', "\r\n");
        if line.stream == Stream::Stderr && !s.contains('\x1b') {
            s = format!("\x1b[31m{s}\x1b[0m");
        }
        if line.newline {
            s.push_str("\r\n");
        }
        self.term.feed(s.as_bytes());
    }

    /// A message from the IDE itself (grey).
    pub fn note(&self, text: &str) {
        self.term.feed(format!("\x1b[90m{}\x1b[0m\r\n", text.replace('\n', "\r\n")).as_bytes());
    }
}

pub struct BuildState {
    pub handle: Option<OpHandle>,
    inbox: Inbox<BuildEvent>,
    pub console: Console,
    pub diagnostics: Vec<rc_dotnet::Diagnostic>,
    pub result: Option<BuildResult>,
    pub failed: Option<String>,
    pub started: Instant,
    pub title: String,
}

pub struct RunTab {
    pub name: String,
    pub config: RunConfig,
    pub handle: Option<RunHandle>,
    inbox: Inbox<RunEvent>,
    pub console: Console,
    pub exit: Option<Option<i32>>,
    pub urls: Vec<String>,
}

impl RunTab {
    pub fn running(&self) -> bool {
        self.exit.is_none()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TestState {
    Running,
    Passed,
    Failed,
    Skipped,
}

pub struct TestsState {
    pub handle: Option<OpHandle>,
    inbox: Inbox<TestEvent>,
    pub console: Console,
    /// Results by fully qualified name (every data case listed).
    pub results: BTreeMap<String, Vec<TestResult>>,
    pub running: bool,
    pub passed: usize,
    pub failed: usize,
    pub skipped: usize,
    pub duration: Option<std::time::Duration>,
    pub failure: Option<String>,
    pub request: TestRequest,
    pub selected: Option<String>,
    pub show_passed: bool,
}

/// What the runner tells the rest of the UI.
pub enum RunnerOutput {
    ShowTool(crate::toolwindow::ToolWindow),
    Notify(String, bool),
    /// A build finished: its diagnostics for the Problems window.
    BuildDiagnostics(Vec<rc_dotnet::Diagnostic>),
}

pub struct Runner {
    pub configs: Vec<RunConfig>,
    pub selected: usize,
    pub build: Option<BuildState>,
    pub runs: Vec<RunTab>,
    pub active_run: usize,
    pub tests: Option<TestsState>,
    /// Last outcome per test (fully qualified name), for gutter icons.
    pub test_outcomes: BTreeMap<String, TestState>,
    waker: Waker,
    dotnet: Option<Dotnet>,
    /// Compound configurations of the workspace (name, member names), kept in the session.
    pub compounds: Vec<(String, Vec<String>)>,
    /// Edited application configurations, by name (kept in the session).
    pub options: BTreeMap<String, rc_core::session::RunOptions>,
}

impl Runner {
    pub fn new(waker: Waker) -> Self {
        Runner { configs: Vec::new(), selected: 0, build: None, runs: Vec::new(), active_run: 0, tests: None, test_outcomes: BTreeMap::new(), waker, dotnet: None, compounds: Vec::new(), options: BTreeMap::new() }
    }

    /// Builds the run configurations of a workspace: executable projects (one per launch
    /// profile of type `Project`), then "All tests" for the solution and each test project.
    pub fn set_workspace(&mut self, solution: Option<&Solution>, projects: &[PathBuf], dotnet: &Path, remembered: Option<&str>) {
        self.dotnet = Dotnet::locate(Some(dotnet)).or_else(|| Dotnet::locate(None));
        self.configs.clear();
        let paths: Vec<PathBuf> = match solution {
            Some(s) => s.projects().into_iter().map(|p| p.path.clone()).collect(),
            None => projects.to_vec(),
        };
        let mut test_projects = Vec::new();
        for path in &paths {
            let Ok(info) = ProjectInfo::load(path) else { continue };
            if info.is_test {
                test_projects.push(path.clone());
                continue;
            }
            if !info.is_executable() {
                continue;
            }
            let profiles: Vec<String> = rc_dotnet::LaunchSettings::load(path)
                .ok()
                .flatten()
                .map(|s| s.profiles.iter().filter(|p| p.command_name == "Project").map(|p| p.name.clone()).collect())
                .unwrap_or_default();
            if profiles.len() <= 1 {
                self.configs.push(RunConfig { name: info.name.clone(), kind: ConfigKind::App { project: path.clone(), profile: profiles.into_iter().next(), web: info.is_web() } });
            } else {
                for p in profiles {
                    self.configs.push(RunConfig { name: format!("{}: {p}", info.name), kind: ConfigKind::App { project: path.clone(), profile: Some(p), web: info.is_web() } });
                }
            }
        }
        self.insert_compounds();
        if !test_projects.is_empty() {
            if let Some(s) = solution {
                self.configs.push(RunConfig { name: tr("All Tests").to_string(), kind: ConfigKind::Tests { target: s.path.clone() } });
            }
            for t in test_projects {
                let name = t.file_stem().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                self.configs.push(RunConfig { name, kind: ConfigKind::Tests { target: t } });
            }
        }
        self.selected = remembered.and_then(|r| self.configs.iter().position(|c| c.name == r)).unwrap_or(0);
    }

    pub fn clear(&mut self) {
        self.configs.clear();
        self.build = None;
        self.runs.clear();
        self.tests = None;
        self.test_outcomes.clear();
    }

    pub fn selected_config(&self) -> Option<&RunConfig> {
        self.configs.get(self.selected)
    }

    pub fn busy(&self) -> bool {
        self.build.as_ref().is_some_and(|b| b.handle.is_some()) || self.tests.as_ref().is_some_and(|t| t.running) || self.runs.iter().any(|r| r.running())
    }

    fn dotnet(&mut self) -> Result<Dotnet, String> {
        self.dotnet.clone().ok_or_else(|| tr("The .NET SDK was not found. Install it or set its path in Settings › Tools.").to_string())
    }

    // ------------------------------------------------------------------ build

    pub fn build(&mut self, target: &Path, action: BuildAction) -> Vec<RunnerOutput> {
        let dotnet = match self.dotnet() {
            Ok(d) => d,
            Err(e) => return vec![RunnerOutput::Notify(e, true)],
        };
        let inbox = Inbox::new(self.waker.clone());
        let console = Console::new(&self.waker);
        let title = match action {
            BuildAction::Rebuild => tr("Rebuild"),
            BuildAction::Clean => tr("Clean"),
            BuildAction::Restore => tr("Restore"),
            BuildAction::Build => tr("Build"),
        }
        .to_string();
        let handle = rc_dotnet::start_build(&dotnet, BuildRequest::new(target, action), inbox.sender());
        self.build = Some(BuildState { handle: Some(handle), inbox, console, diagnostics: Vec::new(), result: None, failed: None, started: Instant::now(), title });
        vec![RunnerOutput::ShowTool(crate::toolwindow::ToolWindow::Build)]
    }

    pub fn cancel_build(&mut self) {
        if let Some(b) = &mut self.build
            && let Some(h) = b.handle.take()
        {
            h.cancel();
        }
    }

    // ------------------------------------------------------------------ run

    /// Starts the selected configuration (apps run in the Run window, tests in Tests).
    pub fn run_selected(&mut self) -> Vec<RunnerOutput> {
        let Some(cfg) = self.selected_config().cloned() else {
            return vec![RunnerOutput::Notify(tr("There is nothing to run: the workspace has no executable or test projects").into(), false)];
        };
        match cfg.kind.clone() {
            ConfigKind::App { .. } => self.run_app(cfg),
            ConfigKind::Tests { target } => self.run_tests(&target, None),
            ConfigKind::Compound { members } => {
                let apps: Vec<RunConfig> = members.iter().filter_map(|m| self.configs.iter().find(|c| &c.name == m && matches!(c.kind, ConfigKind::App { .. })).cloned()).collect();
                if apps.is_empty() {
                    return vec![RunnerOutput::Notify(tr("The compound configuration has no applications to run").into(), true)];
                }
                let mut out = Vec::new();
                for a in apps {
                    out.extend(self.run_app(a));
                }
                out
            }
        }
    }

    /// Compound configurations go right after the applications (before the tests).
    fn insert_compounds(&mut self) {
        let selected = self.selected_config().map(|c| c.name.clone());
        self.configs.retain(|c| !matches!(c.kind, ConfigKind::Compound { .. }));
        let at = self.configs.iter().rposition(|c| matches!(c.kind, ConfigKind::App { .. })).map_or(0, |i| i + 1);
        for (i, (name, members)) in self.compounds.iter().enumerate() {
            self.configs.insert(at + i, RunConfig { name: name.clone(), kind: ConfigKind::Compound { members: members.clone() } });
        }
        if let Some(n) = selected
            && let Some(i) = self.configs.iter().position(|c| c.name == n)
        {
            self.selected = i;
        }
    }

    /// Adds or replaces (by `old` name) a compound configuration and selects it.
    pub fn save_compound(&mut self, old: Option<&str>, name: String, members: Vec<String>) {
        match old.and_then(|o| self.compounds.iter().position(|(n, _)| n == o)) {
            Some(i) => self.compounds[i] = (name.clone(), members),
            None => self.compounds.push((name.clone(), members)),
        }
        self.insert_compounds();
        if let Some(i) = self.configs.iter().position(|c| c.name == name) {
            self.selected = i;
        }
    }

    pub fn remove_compound(&mut self, name: &str) {
        self.compounds.retain(|(n, _)| n != name);
        self.insert_compounds();
        self.selected = self.selected.min(self.configs.len().saturating_sub(1));
    }

    /// Names of the application configurations (members a compound can have).
    pub fn app_names(&self) -> Vec<String> {
        self.configs.iter().filter(|c| matches!(c.kind, ConfigKind::App { .. })).map(|c| c.name.clone()).collect()
    }

    /// What to run for an application configuration, with the user's edits applied.
    pub fn request_for(&self, cfg: &RunConfig) -> Option<RunRequest> {
        let ConfigKind::App { project, profile, web } = &cfg.kind else { return None };
        let mut req = RunRequest::new(project);
        req.launch_profile = profile.clone();
        // Web apps run under `dotnet watch` by default: saving a view refreshes the browser.
        req.watch = *web;
        if let Some(o) = self.options.get(&cfg.name) {
            match o.profile.as_deref() {
                Some("") => req.use_launch_profile = false,
                Some(p) => req.launch_profile = Some(p.to_string()),
                None => {}
            }
            req.args = rc_dotnet::process::split_command_line(&o.args);
            req.env = o.env_pairs();
            if !o.working_dir.trim().is_empty() {
                req.working_dir = Some(PathBuf::from(o.working_dir.trim()));
            }
            req.build_first = o.build_first;
            req.watch = o.hot_reload.unwrap_or(*web);
        }
        Some(req)
    }

    fn run_app(&mut self, cfg: RunConfig) -> Vec<RunnerOutput> {
        let dotnet = match self.dotnet() {
            Ok(d) => d,
            Err(e) => return vec![RunnerOutput::Notify(e, true)],
        };
        // Running the same configuration again restarts it (Rider's "Rerun").
        if let Some(i) = self.runs.iter().position(|r| r.config == cfg) {
            let old = self.runs.remove(i);
            if let Some(h) = old.handle {
                h.stop();
            }
        }
        let Some(req) = self.request_for(&cfg) else { return Vec::new() };
        let inbox = Inbox::new(self.waker.clone());
        let console = Console::new(&self.waker);
        console.note(&trf("Building {name}…", &[("name", &cfg.name)]));
        let handle = rc_dotnet::start_run(&dotnet, req, inbox.sender());
        self.runs.push(RunTab { name: cfg.name.clone(), config: cfg, handle: Some(handle), inbox, console, exit: None, urls: Vec::new() });
        self.active_run = self.runs.len() - 1;
        vec![RunnerOutput::ShowTool(crate::toolwindow::ToolWindow::Run)]
    }

    pub fn stop_all(&mut self) {
        for r in &mut self.runs {
            if let Some(h) = &r.handle {
                h.stop();
            }
        }
        self.cancel_build();
        if let Some(t) = &mut self.tests
            && let Some(h) = t.handle.take()
        {
            h.cancel();
        }
    }

    pub fn close_run(&mut self, i: usize) {
        if i < self.runs.len() {
            let r = self.runs.remove(i);
            if let Some(h) = r.handle {
                h.stop();
            }
            self.active_run = self.active_run.min(self.runs.len().saturating_sub(1));
        }
    }

    // ------------------------------------------------------------------ tests

    pub fn run_tests(&mut self, target: &Path, filter: Option<TestFilter>) -> Vec<RunnerOutput> {
        let dotnet = match self.dotnet() {
            Ok(d) => d,
            Err(e) => return vec![RunnerOutput::Notify(e, true)],
        };
        if let Some(t) = &mut self.tests
            && let Some(h) = t.handle.take()
        {
            h.cancel();
        }
        let mut req = TestRequest::new(target);
        req.filter = filter;
        let inbox = Inbox::new(self.waker.clone());
        let console = Console::new(&self.waker);
        let handle = rc_dotnet::start_tests(&dotnet, req.clone(), inbox.sender());
        let show_passed = self.tests.as_ref().is_none_or(|t| t.show_passed);
        // Tests about to run show a running icon in the gutter.
        if let Some(TestFilter::Tests(names)) = &req.filter {
            for n in names {
                self.test_outcomes.insert(n.clone(), TestState::Running);
            }
        }
        self.tests = Some(TestsState {
            handle: Some(handle),
            inbox,
            console,
            results: BTreeMap::new(),
            running: true,
            passed: 0,
            failed: 0,
            skipped: 0,
            duration: None,
            failure: None,
            request: req,
            selected: None,
            show_passed,
        });
        vec![RunnerOutput::ShowTool(crate::toolwindow::ToolWindow::Tests)]
    }

    /// Runs every test of the workspace: the "All Tests" configuration, or `dotnet test` on the
    /// solution or folder when no test project was recognised (it still finds them).
    pub fn run_all_tests(&mut self, workspace: &Path) -> Vec<RunnerOutput> {
        let target = self
            .configs
            .iter()
            .find_map(|c| match &c.kind {
                ConfigKind::Tests { target } if !rc_core::workspace::is_project(target) => Some(target.clone()),
                _ => None,
            })
            .or_else(|| {
                // A single test project (no solution).
                let tests: Vec<&PathBuf> = self.configs.iter().filter_map(|c| if let ConfigKind::Tests { target } = &c.kind { Some(target) } else { None }).collect();
                (tests.len() == 1).then(|| tests[0].clone())
            })
            .unwrap_or_else(|| workspace.to_path_buf());
        self.run_tests(&target, None)
    }

    /// Runs the tests that failed in the last run.
    pub fn rerun_failed(&mut self) -> Vec<RunnerOutput> {
        let Some(t) = &self.tests else { return Vec::new() };
        let failed: Vec<String> = t.results.iter().filter(|(_, r)| r.iter().any(|x| x.outcome == TestOutcome::Failed)).map(|(k, _)| k.clone()).collect();
        if failed.is_empty() {
            return vec![RunnerOutput::Notify(tr("No failed tests to run again").into(), false)];
        }
        let target = t.request.target.clone();
        self.run_tests(&target, Some(TestFilter::Tests(failed)))
    }

    pub fn rerun_tests(&mut self) -> Vec<RunnerOutput> {
        let Some(t) = &self.tests else { return Vec::new() };
        let (target, filter) = (t.request.target.clone(), t.request.filter.clone());
        self.run_tests(&target, filter)
    }

    // ------------------------------------------------------------------ per frame

    pub fn update(&mut self) -> Vec<RunnerOutput> {
        let mut out = Vec::new();
        if let Some(b) = &mut self.build {
            for ev in b.inbox.drain() {
                match ev {
                    BuildEvent::Started { command } => b.console.note(&command),
                    BuildEvent::Output(line) => b.console.line(&line),
                    BuildEvent::Diagnostic(d) => b.diagnostics.push(d),
                    BuildEvent::Finished(r) => {
                        b.handle = None;
                        let msg = build_summary(&r);
                        b.console.note(&msg);
                        out.push(RunnerOutput::Notify(msg, !r.success && !r.cancelled));
                        out.push(RunnerOutput::BuildDiagnostics(r.diagnostics.clone()));
                        b.diagnostics = r.diagnostics.clone();
                        b.result = Some(r);
                    }
                    BuildEvent::Failed(e) => {
                        b.handle = None;
                        b.console.note(&e);
                        out.push(RunnerOutput::Notify(e.clone(), true));
                        b.failed = Some(e);
                    }
                }
            }
        }
        for r in &mut self.runs {
            for ev in r.inbox.drain() {
                match ev {
                    RunEvent::Build(BuildEvent::Diagnostic(d)) => {
                        if d.severity == rc_dotnet::Severity::Error {
                            r.console.note(&format!("{}({},{}): {}", d.file.as_ref().map(|f| f.display().to_string()).unwrap_or_default(), d.line.unwrap_or(0), d.column.unwrap_or(0), d.message));
                        }
                    }
                    RunEvent::Build(BuildEvent::Finished(res)) if !res.success => {
                        out.push(RunnerOutput::BuildDiagnostics(res.diagnostics.clone()));
                    }
                    RunEvent::Build(_) => {}
                    RunEvent::Started { plan, .. } => {
                        r.console.note(&plan.command_line());
                        if !plan.urls.is_empty() {
                            r.console.note(&trf("Listening on {urls}", &[("urls", &plan.urls.join(", "))]));
                        }
                        r.urls = plan.urls.clone();
                    }
                    RunEvent::Output(line) => r.console.line(&line),
                    RunEvent::Exited { code, cancelled, .. } => {
                        r.console.note(&match (cancelled, code) {
                            (true, _) => tr("Process stopped").to_string(),
                            (false, Some(c)) => trf("Process finished with exit code {code}", &[("code", &c)]),
                            (false, None) => tr("Process finished").to_string(),
                        });
                        r.exit = Some(code);
                        r.handle = None;
                    }
                    RunEvent::Failed(e) => {
                        r.console.note(&e);
                        r.exit = Some(None);
                        r.handle = None;
                        out.push(RunnerOutput::Notify(e, true));
                    }
                }
            }
        }
        if let Some(t) = &mut self.tests {
            for ev in t.inbox.drain() {
                match ev {
                    TestEvent::Build(BuildEvent::Output(line)) => t.console.line(&line),
                    TestEvent::Build(BuildEvent::Finished(res)) if !res.success => {
                        t.console.note(&build_summary(&res));
                        out.push(RunnerOutput::BuildDiagnostics(res.diagnostics.clone()));
                    }
                    TestEvent::Build(_) => {}
                    TestEvent::ProjectStarted { project } => t.console.note(&trf("Running tests of {project}…", &[("project", &project.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default())])),
                    TestEvent::Output { line, .. } => t.console.line(&line),
                    TestEvent::Progress(p) => match p.outcome {
                        TestOutcome::Passed => t.passed += 1,
                        TestOutcome::Failed => t.failed += 1,
                        TestOutcome::Skipped => t.skipped += 1,
                    },
                    TestEvent::ProjectFinished(run) => {
                        for r in run.results {
                            t.results.entry(r.fully_qualified_name.clone()).or_default().push(r);
                        }
                    }
                    TestEvent::Finished(res) => {
                        t.running = false;
                        t.handle = None;
                        t.passed = res.passed;
                        t.failed = res.failed;
                        t.skipped = res.skipped;
                        t.duration = Some(res.duration);
                        t.results.clear();
                        for r in res.results {
                            t.results.entry(r.fully_qualified_name.clone()).or_default().push(r);
                        }
                        let msg = if res.cancelled {
                            tr("Tests stopped").to_string()
                        } else if res.failed > 0 {
                            trf("Tests failed: {failed}, passed: {passed}", &[("failed", &res.failed), ("passed", &res.passed)])
                        } else if res.passed + res.skipped == 0 {
                            tr("No tests were run").to_string()
                        } else {
                            trf("Tests passed: {passed}", &[("passed", &res.passed)])
                        };
                        t.console.note(&msg);
                        out.push(RunnerOutput::Notify(msg, res.failed > 0));
                        if t.selected.is_none() {
                            t.selected = t.results.iter().find(|(_, r)| r.iter().any(|x| x.outcome == TestOutcome::Failed)).map(|(k, _)| k.clone());
                        }
                    }
                    TestEvent::Failed(e) => {
                        t.running = false;
                        t.handle = None;
                        t.console.note(&e);
                        t.failure = Some(e.clone());
                        out.push(RunnerOutput::Notify(e, true));
                    }
                }
            }
            if !t.running {
                // The gutter remembers the last outcome of every test that ran.
                for (name, results) in &t.results {
                    let state = if results.iter().any(|r| r.outcome == TestOutcome::Failed) {
                        TestState::Failed
                    } else if results.iter().all(|r| r.outcome == TestOutcome::Skipped) {
                        TestState::Skipped
                    } else {
                        TestState::Passed
                    };
                    self.test_outcomes.insert(name.clone(), state);
                }
                self.test_outcomes.retain(|_, s| *s != TestState::Running);
            }
        }
        out
    }
}

fn build_summary(r: &BuildResult) -> String {
    let secs = format!("{:.1}", r.duration.as_secs_f32());
    if r.cancelled {
        tr("Build cancelled").to_string()
    } else if r.success {
        trf("Build succeeded with {warnings} warnings in {secs} s", &[("warnings", &r.warnings), ("secs", &secs)])
    } else {
        trf("Build failed: {errors} errors, {warnings} warnings", &[("errors", &r.errors), ("warnings", &r.warnings)])
    }
}

/// A test (or test class) declared in a file, found with the syntax tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TestMarker {
    pub line: usize,
    /// `Namespace.Class.Method`, or `Namespace.Class` for a class.
    pub name: String,
    pub is_class: bool,
}

const TEST_ATTRIBUTES: &[&str] = &["Fact", "Theory", "Test", "TestCase", "TestCaseSource", "TestMethod", "DataTestMethod", "TestFixture", "TestClass"];

/// Test methods and classes of a C# file (xUnit, NUnit, MSTest attributes).
pub fn test_markers(rope: &rc_text::Rope, tree: &rc_syntax::tree_sitter::Tree) -> Vec<TestMarker> {
    use rc_syntax::tree_sitter::Node;
    let text = |n: Node| rope.byte_slice(n.start_byte()..n.end_byte()).to_string();
    let has_test_attr = |n: Node| -> bool {
        let mut c = n.walk();
        n.children(&mut c).filter(|ch| ch.kind() == "attribute_list").any(|list| {
            let mut c2 = list.walk();
            list.children(&mut c2).filter(|a| a.kind() == "attribute").any(|a| {
                let name = a.child_by_field_name("name").map(text).unwrap_or_default();
                let short = name.rsplit('.').next().unwrap_or(&name).trim_end_matches("Attribute").to_string();
                TEST_ATTRIBUTES.contains(&short.as_str())
            })
        })
    };
    let mut out = Vec::new();
    fn walk<'a>(n: Node<'a>, ns: &str, class: &str, out: &mut Vec<TestMarker>, text: &dyn Fn(Node) -> String, has_test_attr: &dyn Fn(Node) -> bool) {
        let mut cursor = n.walk();
        let mut file_ns = ns.to_string();
        for ch in n.children(&mut cursor) {
            match ch.kind() {
                "file_scoped_namespace_declaration" => {
                    file_ns = ch.child_by_field_name("name").map(text).unwrap_or_default();
                    walk(ch, &file_ns, class, out, text, has_test_attr);
                }
                "namespace_declaration" => {
                    let name = ch.child_by_field_name("name").map(text).unwrap_or_default();
                    let full = if file_ns.is_empty() { name } else { format!("{file_ns}.{name}") };
                    if let Some(body) = ch.child_by_field_name("body") {
                        walk(body, &full, class, out, text, has_test_attr);
                    }
                }
                "class_declaration" | "record_declaration" => {
                    let name = ch.child_by_field_name("name").map(text).unwrap_or_default();
                    let full_class = if class.is_empty() { name.clone() } else { format!("{class}+{name}") };
                    let qualified = if file_ns.is_empty() { full_class.clone() } else { format!("{file_ns}.{full_class}") };
                    let before = out.len();
                    if let Some(body) = ch.child_by_field_name("body") {
                        walk(body, &file_ns, &full_class, out, text, has_test_attr);
                    }
                    if out.len() > before || has_test_attr(ch) {
                        let line = ch.child_by_field_name("name").map_or(ch.start_position().row, |n| n.start_position().row);
                        out.insert(before, TestMarker { line, name: qualified, is_class: true });
                    }
                }
                "method_declaration" if !class.is_empty() && has_test_attr(ch) => {
                    let name = ch.child_by_field_name("name").map(text).unwrap_or_default();
                    let line = ch.child_by_field_name("name").map_or(ch.start_position().row, |n| n.start_position().row);
                    let qualified = if file_ns.is_empty() { format!("{class}.{name}") } else { format!("{file_ns}.{class}.{name}") };
                    out.push(TestMarker { line, name: qualified, is_class: false });
                }
                "declaration_list" | "compilation_unit" => walk(ch, &file_ns, class, out, text, has_test_attr),
                _ => {}
            }
        }
    }
    walk(tree.root_node(), "", "", &mut out, &text, &has_test_attr);
    out
}

/// The test project that contains `file`.
pub fn project_of(file: &Path, configs: &[RunConfig]) -> Option<PathBuf> {
    configs
        .iter()
        .filter_map(|c| match &c.kind {
            ConfigKind::Tests { target } if target.extension().is_some_and(|e| e == "csproj") => Some(target.clone()),
            _ => None,
        })
        .filter(|p| p.parent().is_some_and(|d| file.starts_with(d)))
        .max_by_key(|p| p.as_os_str().len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_xunit_nunit_and_mstest_markers() {
        let src = "using Xunit;\nnamespace Shop.Tests;\n\npublic class PriceTests\n{\n    [Fact]\n    public void Adds() {}\n\n    [Theory, InlineData(1)]\n    public void Data(int x) {}\n\n    public void Helper() {}\n}\n\n[TestFixture]\npublic class Other\n{\n    [Test]\n    public void One() {}\n}\n";
        let rope = rc_text::Rope::from_str(src);
        let tree = rc_syntax::parse_now(rc_syntax::Lang::CSharp, &rope, None).unwrap();
        let m = test_markers(&rope, &tree);
        let names: Vec<(&str, usize, bool)> = m.iter().map(|t| (t.name.as_str(), t.line, t.is_class)).collect();
        assert_eq!(
            names,
            vec![
                ("Shop.Tests.PriceTests", 3, true),
                ("Shop.Tests.PriceTests.Adds", 6, false),
                ("Shop.Tests.PriceTests.Data", 9, false),
                ("Shop.Tests.Other", 15, true),
                ("Shop.Tests.Other.One", 18, false),
            ]
        );
    }
}
