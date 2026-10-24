//! Test discovery and runs for xUnit, NUnit and MSTest (VSTest).
//!
//! A run builds the target once, then runs `dotnet test --no-build` for every test project
//! in parallel, each with its own TRX file (one shared `LogFileName` would be overwritten by
//! every project). Progress comes from the console logger; final results from the TRX files.
//! Discovery uses `dotnet vstest --ListFullyQualifiedTests`, which, unlike
//! `dotnet test --list-tests`, gives fully qualified names for every framework.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rc_core::Runtime;
use rc_core::inbox::Outbox;
use tokio::sync::{Semaphore, mpsc};

use crate::build::{BuildAction, BuildEvent, BuildRequest, BuildResult, build};
use crate::process::{CancelToken, OpHandle, OutputLine, StreamOpts, run_streaming};
use crate::project::ProjectInfo;
use crate::sdk::Dotnet;
use crate::solution::Solution;
use crate::trx::{TestOutcome, TestResult, load_trx, split_test_name};

/// Which tests to run (a VSTest `--filter`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TestFilter {
    /// Fully qualified names (`Ns.Class.Method`, every data case; or one NUnit case `M(9,0)`).
    Tests(Vec<String>),
    /// Every test of a class (`Ns.Class`).
    Class(String),
    /// Every test under a namespace.
    Namespace(String),
    /// A filter expression passed as is.
    Raw(String),
}

impl TestFilter {
    /// The `--filter` expression.
    pub fn expression(&self) -> String {
        match self {
            // `Ns.C.M` matches every data case of M with the three frameworks; a NUnit case
            // name (`Ns.C.M(9,0)`) selects only that case.
            TestFilter::Tests(names) => names.iter().map(|n| format!("FullyQualifiedName={}", escape_filter_value(n))).collect::<Vec<_>>().join("|"),
            TestFilter::Class(c) => format!("FullyQualifiedName~{}.", escape_filter_value(c)),
            TestFilter::Namespace(n) => format!("FullyQualifiedName~{}.", escape_filter_value(n)),
            TestFilter::Raw(r) => r.clone(),
        }
    }
}

/// Escapes the characters that VSTest filters treat as operators.
pub fn escape_filter_value(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '\\' | '(' | ')' | '&' | '|' | '=' | '!' | '~') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// A test run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TestRequest {
    /// Solution or test project.
    pub target: PathBuf,
    pub configuration: String,
    pub target_framework: Option<String>,
    pub filter: Option<TestFilter>,
    /// Skip the build (the assemblies must be up to date).
    pub no_build: bool,
    /// Where TRX files go (a new temporary folder when `None`).
    pub results_dir: Option<PathBuf>,
    /// Test projects run at the same time.
    pub max_parallel: usize,
}

impl TestRequest {
    pub fn new(target: impl Into<PathBuf>) -> Self {
        TestRequest {
            target: target.into(),
            configuration: "Debug".into(),
            target_framework: None,
            filter: None,
            no_build: false,
            results_dir: None,
            max_parallel: 4,
        }
    }
}

/// One finished test as reported by the console logger while running.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TestProgress {
    pub project: PathBuf,
    pub outcome: TestOutcome,
    /// The display name printed by the framework.
    pub name: String,
    /// As printed (`12 ms`, `< 1 ms`).
    pub duration: Option<String>,
}

/// Results of one test project.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectRun {
    pub project: PathBuf,
    pub exit_code: Option<i32>,
    pub trx: Option<PathBuf>,
    pub results: Vec<TestResult>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TestRunResult {
    /// Every result of every project.
    pub results: Vec<TestResult>,
    pub projects: Vec<ProjectRun>,
    pub passed: usize,
    pub failed: usize,
    pub skipped: usize,
    /// Build ok, every project exited with 0 and no test failed.
    pub success: bool,
    pub cancelled: bool,
    /// `None` with `no_build`.
    pub build: Option<BuildResult>,
    pub results_dir: PathBuf,
    pub duration: Duration,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TestEvent {
    /// Events of the build that precedes the run.
    Build(BuildEvent),
    ProjectStarted { project: PathBuf },
    Output { project: PathBuf, line: OutputLine },
    Progress(TestProgress),
    ProjectFinished(ProjectRun),
    /// Always the last event.
    Finished(TestRunResult),
    /// The run could not start; also the last event.
    Failed(String),
}

/// A test found by discovery.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveredTest {
    /// As listed by the adapter (NUnit includes data arguments: `Ns.C.M(9,0)`).
    pub name: String,
    /// `Ns.Class.Method` (no arguments).
    pub fully_qualified_name: String,
    pub class_name: String,
    pub method_name: String,
    pub project: PathBuf,
    pub assembly: PathBuf,
}

/// The test projects of a solution (by their package references), or the project itself.
pub fn test_projects(target: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let ext = target.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    if ext == "sln" || ext == "slnx" {
        let sln = Solution::load(target)?;
        Ok(sln
            .projects()
            .into_iter()
            .filter(|p| ProjectInfo::load(&p.path).is_ok_and(|i| i.is_test))
            .map(|p| p.path.clone())
            .collect())
    } else if ext.ends_with("proj") {
        Ok(vec![target.to_path_buf()])
    } else {
        anyhow::bail!("not a solution or project: {}", target.display())
    }
}

/// Starts a test run on the shared runtime; events go to `outbox`.
pub fn start_tests(dotnet: &Dotnet, req: TestRequest, outbox: Outbox<TestEvent>) -> OpHandle {
    let cancel = CancelToken::new();
    let token = cancel.clone();
    let dotnet = dotnet.clone();
    let task = Runtime::spawn(async move {
        let out = outbox.clone();
        match run_tests(&dotnet, &req, &token, &mut |e| {
            out.send(e);
        })
        .await
        {
            Ok(r) => outbox.send(TestEvent::Finished(r)),
            Err(e) => outbox.send(TestEvent::Failed(format!("{e:#}"))),
        };
    });
    OpHandle::new(cancel, task)
}

/// Builds (unless `no_build`) and runs the tests; reports every event but the last.
pub async fn run_tests(dotnet: &Dotnet, req: &TestRequest, cancel: &CancelToken, on_event: &mut (dyn FnMut(TestEvent) + Send)) -> anyhow::Result<TestRunResult> {
    let started = Instant::now();
    let projects = test_projects(&req.target)?;
    let results_dir = match &req.results_dir {
        Some(d) => {
            std::fs::create_dir_all(d)?;
            d.clone()
        }
        None => tempfile::Builder::new().prefix("dotcraft-tests-").tempdir()?.keep(),
    };
    let mut result = TestRunResult { results_dir: results_dir.clone(), ..Default::default() };

    if !req.no_build {
        let mut b = BuildRequest::new(&req.target, BuildAction::Build);
        b.configuration = req.configuration.clone();
        b.target_framework = req.target_framework.clone();
        let br = build(dotnet, &b, cancel, &mut |e| on_event(TestEvent::Build(e))).await?;
        on_event(TestEvent::Build(BuildEvent::Finished(br.clone())));
        let ok = br.success;
        result.cancelled = br.cancelled;
        result.build = Some(br);
        if !ok {
            result.duration = started.elapsed();
            return Ok(result);
        }
    }

    let (tx, mut rx) = mpsc::unbounded_channel::<TestEvent>();
    let sem = Arc::new(Semaphore::new(req.max_parallel.max(1)));
    let mut tasks = Vec::new();
    for (i, project) in projects.into_iter().enumerate() {
        let (dotnet, req, cancel, tx, sem) = (dotnet.clone(), req.clone(), cancel.clone(), tx.clone(), sem.clone());
        let dir = results_dir.join(format!("{i}-{}", project.file_stem().unwrap_or_default().to_string_lossy()));
        tasks.push(tokio::spawn(async move {
            let _permit = sem.acquire_owned().await;
            run_project(&dotnet, &project, &req, &dir, &cancel, &tx).await
        }));
    }
    drop(tx);
    while let Some(e) = rx.recv().await {
        on_event(e);
    }
    let mut all_ok = true;
    for t in tasks {
        let run = t.await?;
        all_ok &= run.exit_code == Some(0);
        result.results.extend(run.results.iter().cloned());
        result.projects.push(run);
    }
    let count = |o| result.results.iter().filter(|r| r.outcome == o).count();
    result.passed = count(TestOutcome::Passed);
    result.failed = count(TestOutcome::Failed);
    result.skipped = count(TestOutcome::Skipped);
    result.cancelled |= cancel.is_cancelled();
    result.success = all_ok && result.failed == 0 && !result.cancelled;
    result.duration = started.elapsed();
    Ok(result)
}

async fn run_project(dotnet: &Dotnet, project: &Path, req: &TestRequest, dir: &Path, cancel: &CancelToken, tx: &mpsc::UnboundedSender<TestEvent>) -> ProjectRun {
    let _ = tx.send(TestEvent::ProjectStarted { project: project.to_path_buf() });
    let trx_name = "results.trx";
    let mut cmd = dotnet.command();
    cmd.arg("test").arg(project).arg("--no-build").args(["-c", &req.configuration]);
    if let Some(tf) = &req.target_framework {
        cmd.args(["-f", tf]);
    }
    cmd.arg("--logger").arg(format!("trx;LogFileName={trx_name}"));
    cmd.arg("--logger").arg("console;verbosity=normal");
    cmd.arg("--results-directory").arg(dir);
    if let Some(f) = &req.filter {
        cmd.arg("--filter").arg(f.expression());
    }
    if let Some(d) = project.parent() {
        cmd.current_dir(d);
    }
    let mut on_line = |line: OutputLine| {
        if let Some((outcome, name, duration)) = parse_progress_line(&line.text) {
            let _ = tx.send(TestEvent::Progress(TestProgress { project: project.to_path_buf(), outcome, name, duration }));
        }
        let _ = tx.send(TestEvent::Output { project: project.to_path_buf(), line });
    };
    let outcome = run_streaming(cmd, cancel, StreamOpts::default(), &mut |_| {}, &mut on_line).await;
    let trx = dir.join(trx_name);
    let (results, trx) = match load_trx(&trx) {
        Ok(r) => (r, Some(trx)),
        Err(_) => (Vec::new(), None),
    };
    let run = ProjectRun {
        project: project.to_path_buf(),
        exit_code: match outcome {
            Ok(o) => o.code,
            Err(e) => {
                let _ = tx.send(TestEvent::Output { project: project.to_path_buf(), line: OutputLine::stdout(format!("failed to start dotnet test: {e}")) });
                None
            }
        },
        trx,
        results,
    };
    let _ = tx.send(TestEvent::ProjectFinished(run.clone()));
    run
}

/// `  Passed Ns.C.M [12 ms]` → (Passed, `Ns.C.M`, `12 ms`).
pub fn parse_progress_line(line: &str) -> Option<(TestOutcome, String, Option<String>)> {
    let l = crate::process::strip_ansi(line);
    let l = l.trim();
    let (outcome, rest) = [("Passed ", TestOutcome::Passed), ("Failed ", TestOutcome::Failed), ("Skipped ", TestOutcome::Skipped)]
        .into_iter()
        .find_map(|(p, o)| l.strip_prefix(p).map(|r| (o, r)))?;
    let rest = rest.trim();
    if rest.is_empty() || rest.starts_with(':') || rest.starts_with('!') {
        return None; // "Passed: 4" summaries, "Failed!  - Failed: 1 …"
    }
    if let Some(body) = rest.strip_suffix(']')
        && let Some(open) = body.rfind(" [")
    {
        return Some((outcome, body[..open].to_string(), Some(body[open + 2..].to_string())));
    }
    Some((outcome, rest.to_string(), None))
}

/// Lists the tests of a solution or project (building first when `build`).
pub async fn discover_tests(dotnet: &Dotnet, target: &Path, configuration: &str, build_first: bool, cancel: &CancelToken) -> anyhow::Result<Vec<DiscoveredTest>> {
    if build_first {
        let mut b = BuildRequest::new(target, BuildAction::Build);
        b.configuration = configuration.to_string();
        let r = build(dotnet, &b, cancel, &mut |_| {}).await?;
        if !r.success {
            let first = r.errors().next().map(|d| d.message.clone()).unwrap_or_default();
            anyhow::bail!("build failed ({} errors) {first}", r.errors);
        }
    }
    let tmp = tempfile::Builder::new().prefix("dotcraft-discovery-").tempdir()?;
    let mut out = Vec::new();
    for (i, project) in test_projects(target)?.into_iter().enumerate() {
        if cancel.is_cancelled() {
            break;
        }
        let props = crate::msbuild::evaluate(dotnet, &project, configuration, None).await?;
        let list = tmp.path().join(format!("{i}.txt"));
        let mut cmd = dotnet.command();
        cmd.arg("vstest").arg(&props.target_path).arg("--ListFullyQualifiedTests").arg(format!("--ListTestsTargetPath:{}", list.display()));
        let mut lines = Vec::new();
        run_streaming(cmd, cancel, StreamOpts::default(), &mut |_| {}, &mut |l| lines.push(l.text)).await?;
        let text = std::fs::read_to_string(&list).map_err(|e| anyhow::anyhow!("discovery failed for {}: {e}\n{}", project.display(), lines.join("\n")))?;
        out.extend(parse_test_list(&text, &project, &props.target_path));
    }
    Ok(out)
}

/// Parses the file written by `--ListFullyQualifiedTests` (one name per line).
pub fn parse_test_list(text: &str, project: &Path, assembly: &Path) -> Vec<DiscoveredTest> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|name| {
            let (class_name, method_name) = split_test_name(name);
            DiscoveredTest {
                name: name.to_string(),
                fully_qualified_name: if class_name.is_empty() { method_name.clone() } else { format!("{class_name}.{method_name}") },
                class_name,
                method_name,
                project: project.to_path_buf(),
                assembly: assembly.to_path_buf(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_filters() {
        assert_eq!(TestFilter::Tests(vec!["A.B.C".into(), "A.B.D(9,0)".into()]).expression(), "FullyQualifiedName=A.B.C|FullyQualifiedName=A.B.D\\(9,0\\)");
        assert_eq!(TestFilter::Class("A.B".into()).expression(), "FullyQualifiedName~A.B.");
        assert_eq!(escape_filter_value("M(1,!x)"), "M\\(1,\\!x\\)");
        assert_eq!(TestFilter::Raw("Category=Fast".into()).expression(), "Category=Fast");
    }

    #[test]
    fn parses_console_progress() {
        assert_eq!(parse_progress_line("  Passed Shop.T.M(coupon: \"x\") [11 ms]"), Some((TestOutcome::Passed, "Shop.T.M(coupon: \"x\")".into(), Some("11 ms".into()))));
        assert_eq!(parse_progress_line("  Failed BulkOrders(9,0) [< 1 ms]"), Some((TestOutcome::Failed, "BulkOrders(9,0)".into(), Some("< 1 ms".into()))));
        assert_eq!(parse_progress_line("  Skipped Later"), Some((TestOutcome::Skipped, "Later".into(), None)));
        assert_eq!(parse_progress_line("     Passed: 4"), None);
        assert_eq!(parse_progress_line("Failed!  - Failed:     1, Passed:     3"), None);
        assert_eq!(parse_progress_line("Test Run Successful."), None);
    }

    #[test]
    fn parses_test_lists_and_finds_test_projects() {
        let t = parse_test_list("Shop.T.OrderTests.Bulk(9,0)\nShop.T.OrderTests.Add\n\n", Path::new("/p/T.csproj"), Path::new("/p/T.dll"));
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].fully_qualified_name, "Shop.T.OrderTests.Bulk");
        assert_eq!(t[0].name, "Shop.T.OrderTests.Bulk(9,0)");
        assert_eq!(t[1].class_name, "Shop.T.OrderTests");

        let sln = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/Shop/Shop.sln");
        let names: Vec<String> = test_projects(&sln).unwrap().iter().map(|p| p.file_stem().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, vec!["Shop.Core.MSTests", "Shop.Core.NunitTests", "Shop.Core.XunitTests"]);
    }
}
