//! Unit test coverage (like Rider's dotCover) with coverlet, which the xUnit, NUnit and MSTest
//! templates already reference (`coverlet.collector`).
//!
//! A run builds the target once, then runs `dotnet test --no-build --collect "XPlat Code
//! Coverage"` for every test project that references the collector, in parallel, each with
//! its own results folder. Every project writes a `coverage.cobertura.xml`; they are parsed
//! with [`parse_cobertura`] and merged into one [`CoverageReport`] (hits add up, so a line
//! covered by any test project counts as covered).

use std::collections::{BTreeMap, BTreeSet};
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
use crate::test::{TestFilter, test_projects};

/// The NuGet package that makes `--collect "XPlat Code Coverage"` work.
pub const COLLECTOR_PACKAGE: &str = "coverlet.collector";

/// Covered items out of the total (lines or branches).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Ratio {
    pub covered: u32,
    pub total: u32,
}

impl Ratio {
    pub fn new(covered: u32, total: u32) -> Self {
        Ratio { covered, total }
    }

    /// 0–100; `None` when there is nothing to cover.
    pub fn percent(&self) -> Option<f64> {
        (self.total > 0).then(|| self.covered as f64 * 100.0 / self.total as f64)
    }

    pub fn add(&mut self, other: Ratio) {
        self.covered += other.covered;
        self.total += other.total;
    }
}

/// Coverage of one source file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileCoverage {
    /// Coverable lines (0-based) and how many times they ran (0 = not covered).
    pub lines: BTreeMap<usize, u32>,
    /// Lines (0-based) with conditions: covered branches out of the total.
    pub branches: BTreeMap<usize, Ratio>,
    /// Lines with hits out of the coverable lines.
    pub ratio: Ratio,
    pub branch_ratio: Ratio,
}

impl FileCoverage {
    /// Not covered, partly covered (some branches missed) or covered; `None` when the line
    /// is not coverable.
    pub fn line_state(&self, line: usize) -> Option<LineState> {
        let hits = *self.lines.get(&line)?;
        Some(match (hits, self.branches.get(&line)) {
            (0, _) => LineState::NotCovered,
            (_, Some(b)) if b.covered < b.total => LineState::Partial,
            _ => LineState::Covered,
        })
    }

    fn recompute(&mut self) {
        self.ratio = Ratio::new(self.lines.values().filter(|h| **h > 0).count() as u32, self.lines.len() as u32);
        self.branch_ratio = Ratio::default();
        for b in self.branches.values() {
            self.branch_ratio.add(*b);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineState {
    Covered,
    Partial,
    NotCovered,
}

/// A class (compiler-generated nested classes such as `<>c` or async state machines are
/// folded into the class that declares them).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClassCoverage {
    /// Full name; nested classes as `Outer/Inner`.
    pub name: String,
    pub assembly: String,
    pub file: PathBuf,
    /// Its coverable lines in `file` (0-based).
    pub lines: BTreeSet<usize>,
    pub ratio: Ratio,
    pub branch_ratio: Ratio,
}

/// An instrumented assembly (a cobertura `package`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AssemblyCoverage {
    pub name: String,
    pub ratio: Ratio,
    pub branch_ratio: Ratio,
}

/// Coverage of one or several test runs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CoverageReport {
    /// By absolute path.
    pub files: BTreeMap<PathBuf, FileCoverage>,
    /// Every coverable line of every file.
    pub total: Ratio,
    pub branch_total: Ratio,
    /// Sorted by name.
    pub assemblies: Vec<AssemblyCoverage>,
    /// Sorted by assembly, then name.
    pub classes: Vec<ClassCoverage>,
}

impl CoverageReport {
    /// Adds another report (another test project): hits add up, branches keep the best run.
    pub fn merge(&mut self, other: CoverageReport) {
        for (path, f) in other.files {
            let mine = self.files.entry(path).or_default();
            for (line, hits) in f.lines {
                let h = mine.lines.entry(line).or_insert(0);
                *h = h.saturating_add(hits);
            }
            for (line, b) in f.branches {
                let mb = mine.branches.entry(line).or_default();
                mb.total = mb.total.max(b.total);
                mb.covered = mb.covered.max(b.covered).min(mb.total);
            }
        }
        for c in other.classes {
            match self.classes.iter_mut().find(|m| m.assembly == c.assembly && m.name == c.name && m.file == c.file) {
                Some(m) => m.lines.extend(c.lines),
                None => self.classes.push(c),
            }
        }
        for a in other.assemblies {
            if !self.assemblies.iter().any(|m| m.name == a.name) {
                self.assemblies.push(a);
            }
        }
        self.recompute();
    }

    /// The coverage of a file, if instrumented.
    pub fn file(&self, path: &Path) -> Option<&FileCoverage> {
        self.files.get(path)
    }

    /// Recomputes every ratio from `files` and the class lines.
    fn recompute(&mut self) {
        self.total = Ratio::default();
        self.branch_total = Ratio::default();
        for f in self.files.values_mut() {
            f.recompute();
            self.total.add(f.ratio);
            self.branch_total.add(f.branch_ratio);
        }
        let mut per_assembly: BTreeMap<String, BTreeSet<(PathBuf, usize)>> = BTreeMap::new();
        for c in &mut self.classes {
            let (ratio, branches) = lines_ratio(self.files.get(&c.file), c.lines.iter().copied());
            c.ratio = ratio;
            c.branch_ratio = branches;
            per_assembly.entry(c.assembly.clone()).or_default().extend(c.lines.iter().map(|l| (c.file.clone(), *l)));
        }
        self.classes.sort_by(|a, b| (&a.assembly, &a.name, &a.file).cmp(&(&b.assembly, &b.name, &b.file)));
        for a in &mut self.assemblies {
            a.ratio = Ratio::default();
            a.branch_ratio = Ratio::default();
            if let Some(lines) = per_assembly.get(&a.name) {
                for (file, line) in lines {
                    let (r, b) = lines_ratio(self.files.get(file), std::iter::once(*line));
                    a.ratio.add(r);
                    a.branch_ratio.add(b);
                }
            }
        }
        self.assemblies.sort_by(|a, b| a.name.cmp(&b.name));
    }
}

fn lines_ratio(file: Option<&FileCoverage>, lines: impl Iterator<Item = usize>) -> (Ratio, Ratio) {
    let (mut r, mut b) = (Ratio::default(), Ratio::default());
    let Some(f) = file else { return (r, b) };
    for l in lines {
        if let Some(h) = f.lines.get(&l) {
            r.add(Ratio::new(u32::from(*h > 0), 1));
        }
        if let Some(br) = f.branches.get(&l) {
            b.add(*br);
        }
    }
    (r, b)
}

/// `Ns.Outer/<Method>d__3` and `Ns.Outer/<>c` → `Ns.Outer`; `Ns.Outer/Inner` stays.
fn fold_class_name(name: &str) -> &str {
    match name.find("/<") {
        Some(i) => &name[..i],
        None => name,
    }
}

/// `50% (1/2)` → 1 of 2.
fn parse_condition_coverage(s: &str) -> Option<Ratio> {
    let inner = s.split('(').nth(1)?.split(')').next()?;
    let (c, t) = inner.split_once('/')?;
    Some(Ratio::new(c.trim().parse().ok()?, t.trim().parse().ok()?))
}

/// Parses a cobertura report (as written by coverlet). File names are resolved against the
/// `<sources>` (the first one where the file exists, else the first one).
pub fn parse_cobertura(xml: &str) -> anyhow::Result<CoverageReport> {
    let opts = roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() };
    let doc = roxmltree::Document::parse_with_options(xml, opts)?;
    let root = doc.root_element();
    anyhow::ensure!(root.has_tag_name("coverage"), "not a cobertura report (root is <{}>)", root.tag_name().name());
    let sources: Vec<PathBuf> = root
        .children()
        .filter(|n| n.has_tag_name("sources"))
        .flat_map(|s| s.children().filter(|n| n.has_tag_name("source")))
        .filter_map(|n| n.text())
        .map(|t| PathBuf::from(t.trim()))
        .filter(|p| !p.as_os_str().is_empty())
        .collect();
    let resolve = |filename: &str| -> PathBuf {
        let rel = Path::new(filename);
        if rel.is_absolute() || sources.is_empty() {
            return rel.to_path_buf();
        }
        sources.iter().map(|s| s.join(rel)).find(|p| p.exists()).unwrap_or_else(|| sources[0].join(rel))
    };

    let mut report = CoverageReport::default();
    let packages = root.children().filter(|n| n.has_tag_name("packages")).flat_map(|p| p.children().filter(|n| n.has_tag_name("package")));
    for package in packages {
        let assembly = package.attribute("name").unwrap_or_default().to_string();
        if !report.assemblies.iter().any(|a| a.name == assembly) {
            report.assemblies.push(AssemblyCoverage { name: assembly.clone(), ..Default::default() });
        }
        let classes = package.children().filter(|n| n.has_tag_name("classes")).flat_map(|c| c.children().filter(|n| n.has_tag_name("class")));
        for class in classes {
            let Some(filename) = class.attribute("filename") else { continue };
            let file = resolve(filename);
            let name = fold_class_name(class.attribute("name").unwrap_or_default()).to_string();
            let fc = report.files.entry(file.clone()).or_default();
            let mut class_lines = BTreeSet::new();
            // Lines appear under <class><lines> and again under each <method><lines>.
            for line in class.descendants().filter(|n| n.has_tag_name("line")) {
                let Some(number) = line.attribute("number").and_then(|n| n.parse::<usize>().ok()) else { continue };
                let Some(idx) = number.checked_sub(1) else { continue };
                let hits = line.attribute("hits").and_then(|h| h.parse::<u64>().ok()).unwrap_or(0).min(u32::MAX as u64) as u32;
                let h = fc.lines.entry(idx).or_insert(0);
                *h = (*h).max(hits);
                class_lines.insert(idx);
                if line.attribute("branch").is_some_and(|b| b.eq_ignore_ascii_case("true"))
                    && let Some(r) = line.attribute("condition-coverage").and_then(parse_condition_coverage)
                {
                    let b = fc.branches.entry(idx).or_default();
                    if r.total > b.total || (r.total == b.total && r.covered > b.covered) {
                        *b = r;
                    }
                }
            }
            match report.classes.iter_mut().find(|c| c.assembly == assembly && c.name == name && c.file == file) {
                Some(c) => c.lines.extend(class_lines),
                None => report.classes.push(ClassCoverage { name, assembly: assembly.clone(), file, lines: class_lines, ..Default::default() }),
            }
        }
    }
    report.recompute();
    Ok(report)
}

/// Reads and merges every `coverage.cobertura.xml` under `dir` (any depth).
pub fn load_reports(dir: &Path) -> anyhow::Result<(CoverageReport, Vec<PathBuf>)> {
    let files = find_cobertura_files(dir);
    let mut report = CoverageReport::default();
    for f in &files {
        let text = std::fs::read_to_string(f)?;
        report.merge(parse_cobertura(&text).map_err(|e| anyhow::anyhow!("{}: {e}", f.display()))?);
    }
    Ok((report, files))
}

fn find_cobertura_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.file_name().is_some_and(|n| n.eq_ignore_ascii_case("coverage.cobertura.xml")) {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

/// Whether a test project references `coverlet.collector` (in the project file or, for
/// central package management and imported props, in the restored assets).
pub fn has_collector(project: &Path) -> bool {
    if ProjectInfo::load(project).is_ok_and(|i| i.packages.iter().any(|p| p.name.eq_ignore_ascii_case(COLLECTOR_PACKAGE))) {
        return true;
    }
    let assets = project.parent().map(|d| d.join("obj").join("project.assets.json"));
    assets.and_then(|a| std::fs::read_to_string(a).ok()).is_some_and(|t| t.to_ascii_lowercase().contains("\"coverlet.collector/"))
}

/// A coverage run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoverageRequest {
    /// Solution or test project.
    pub target: PathBuf,
    pub configuration: String,
    pub target_framework: Option<String>,
    pub filter: Option<TestFilter>,
    pub no_build: bool,
    /// Where the cobertura files go (a new temporary folder when `None`).
    pub results_dir: Option<PathBuf>,
    pub max_parallel: usize,
}

impl CoverageRequest {
    pub fn new(target: impl Into<PathBuf>) -> Self {
        CoverageRequest { target: target.into(), configuration: "Debug".into(), target_framework: None, filter: None, no_build: false, results_dir: None, max_parallel: 4 }
    }
}

/// One test project's run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoverageProjectRun {
    pub project: PathBuf,
    /// Non-zero when tests failed (coverage is still collected).
    pub exit_code: Option<i32>,
    /// The cobertura files it wrote.
    pub reports: Vec<PathBuf>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CoverageRunResult {
    pub report: CoverageReport,
    pub projects: Vec<CoverageProjectRun>,
    /// Test projects skipped because they do not reference `coverlet.collector`.
    pub skipped: Vec<PathBuf>,
    /// Every test passed in every project.
    pub tests_passed: bool,
    pub cancelled: bool,
    /// `None` with `no_build`; a failed build leaves the report empty.
    pub build: Option<BuildResult>,
    pub results_dir: PathBuf,
    pub duration: Duration,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CoverageEvent {
    Build(BuildEvent),
    /// Something to show the user (a project skipped for lack of the collector).
    Warning(String),
    ProjectStarted { project: PathBuf },
    Output { project: PathBuf, line: OutputLine },
    ProjectFinished(CoverageProjectRun),
    /// Always the last event.
    Finished(CoverageRunResult),
    /// The run could not start (no test project with the collector…); also the last event.
    Failed(String),
}

/// Runs coverage for a solution or test project with the defaults (Debug, every test).
pub fn start_coverage(dotnet: &Dotnet, target: &Path, outbox: Outbox<CoverageEvent>) -> OpHandle {
    start_coverage_with(dotnet, CoverageRequest::new(target), outbox)
}

/// Starts a coverage run on the shared runtime; events go to `outbox`.
pub fn start_coverage_with(dotnet: &Dotnet, req: CoverageRequest, outbox: Outbox<CoverageEvent>) -> OpHandle {
    let cancel = CancelToken::new();
    let token = cancel.clone();
    let dotnet = dotnet.clone();
    let task = Runtime::spawn(async move {
        let out = outbox.clone();
        match run_coverage(&dotnet, &req, &token, &mut |e| {
            out.send(e);
        })
        .await
        {
            Ok(r) => outbox.send(CoverageEvent::Finished(r)),
            Err(e) => outbox.send(CoverageEvent::Failed(format!("{e:#}"))),
        };
    });
    OpHandle::new(cancel, task)
}

fn missing_collector_message(projects: &[PathBuf]) -> String {
    let names: Vec<String> = projects.iter().map(|p| p.file_stem().unwrap_or_default().to_string_lossy().into_owned()).collect();
    format!(
        "{} {} not reference the {COLLECTOR_PACKAGE} package, needed to collect coverage. Add it with `dotnet add package {COLLECTOR_PACKAGE}`.",
        names.join(", "),
        if names.len() == 1 { "does" } else { "do" }
    )
}

/// Builds (unless `no_build`), runs every test project with coverlet and merges the reports;
/// reports every event but the last.
pub async fn run_coverage(dotnet: &Dotnet, req: &CoverageRequest, cancel: &CancelToken, on_event: &mut (dyn FnMut(CoverageEvent) + Send)) -> anyhow::Result<CoverageRunResult> {
    let started = Instant::now();
    let all = test_projects(&req.target)?;
    anyhow::ensure!(!all.is_empty(), "no test projects in {}", req.target.display());
    let (projects, skipped): (Vec<PathBuf>, Vec<PathBuf>) = all.into_iter().partition(|p| has_collector(p));
    anyhow::ensure!(!projects.is_empty(), "{}", missing_collector_message(&skipped));
    if !skipped.is_empty() {
        on_event(CoverageEvent::Warning(format!("{} Skipped.", missing_collector_message(&skipped))));
    }
    let results_dir = match &req.results_dir {
        Some(d) => {
            std::fs::create_dir_all(d)?;
            d.clone()
        }
        None => tempfile::Builder::new().prefix("dotcraft-coverage-").tempdir()?.keep(),
    };
    let mut result = CoverageRunResult { results_dir: results_dir.clone(), skipped, ..Default::default() };

    if !req.no_build {
        let mut b = BuildRequest::new(&req.target, BuildAction::Build);
        b.configuration = req.configuration.clone();
        b.target_framework = req.target_framework.clone();
        let br = build(dotnet, &b, cancel, &mut |e| on_event(CoverageEvent::Build(e))).await?;
        on_event(CoverageEvent::Build(BuildEvent::Finished(br.clone())));
        let ok = br.success;
        result.cancelled = br.cancelled;
        result.build = Some(br);
        if !ok {
            result.duration = started.elapsed();
            return Ok(result);
        }
    }

    let (tx, mut rx) = mpsc::unbounded_channel::<CoverageEvent>();
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
        for f in &run.reports {
            let text = std::fs::read_to_string(f)?;
            result.report.merge(parse_cobertura(&text).map_err(|e| anyhow::anyhow!("{}: {e}", f.display()))?);
        }
        result.projects.push(run);
    }
    result.cancelled |= cancel.is_cancelled();
    result.tests_passed = all_ok && !result.cancelled;
    result.duration = started.elapsed();
    Ok(result)
}

async fn run_project(dotnet: &Dotnet, project: &Path, req: &CoverageRequest, dir: &Path, cancel: &CancelToken, tx: &mpsc::UnboundedSender<CoverageEvent>) -> CoverageProjectRun {
    let _ = tx.send(CoverageEvent::ProjectStarted { project: project.to_path_buf() });
    let mut cmd = dotnet.command();
    cmd.arg("test").arg(project).arg("--no-build").args(["-c", &req.configuration]);
    if let Some(tf) = &req.target_framework {
        cmd.args(["-f", tf]);
    }
    cmd.args(["--collect", "XPlat Code Coverage"]);
    cmd.arg("--results-directory").arg(dir);
    if let Some(f) = &req.filter {
        cmd.arg("--filter").arg(f.expression());
    }
    if let Some(d) = project.parent() {
        cmd.current_dir(d);
    }
    let mut on_line = |line: OutputLine| {
        let _ = tx.send(CoverageEvent::Output { project: project.to_path_buf(), line });
    };
    let outcome = run_streaming(cmd, cancel, StreamOpts::default(), &mut |_| {}, &mut on_line).await;
    let run = CoverageProjectRun {
        project: project.to_path_buf(),
        exit_code: match outcome {
            Ok(o) => o.code,
            Err(e) => {
                let _ = tx.send(CoverageEvent::Output { project: project.to_path_buf(), line: OutputLine::stdout(format!("failed to start dotnet test: {e}")) });
                None
            }
        },
        reports: if cancel.is_cancelled() { Vec::new() } else { find_cobertura_files(dir) },
    };
    let _ = tx.send(CoverageEvent::ProjectFinished(run.clone()));
    run
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<!DOCTYPE coverage SYSTEM "http://cobertura.sourceforge.net/xml/coverage-04.dtd">
<coverage line-rate="0.6" branch-rate="0.5" version="1.9" timestamp="1700000000" lines-covered="3" lines-valid="5" branches-covered="1" branches-valid="2">
  <sources>
    <source>/repo/</source>
  </sources>
  <packages>
    <package name="Shop.Core" line-rate="0.6" branch-rate="0.5" complexity="3">
      <classes>
        <class name="Shop.Core.PriceCalculator" filename="src/Shop.Core/PriceCalculator.cs" line-rate="0.75" branch-rate="0.5" complexity="2">
          <methods>
            <method name="Calculate" signature="(Shop.Core.Order)" line-rate="0.75" branch-rate="0.5" complexity="2">
              <lines>
                <line number="10" hits="4" branch="False" />
                <line number="11" hits="4" branch="True" condition-coverage="50% (1/2)">
                  <conditions><condition number="7" type="jump" coverage="50%" /></conditions>
                </line>
                <line number="12" hits="0" branch="False" />
              </lines>
            </method>
          </methods>
          <lines>
            <line number="10" hits="4" branch="False" />
            <line number="11" hits="4" branch="True" condition-coverage="50% (1/2)" />
            <line number="12" hits="0" branch="False" />
          </lines>
        </class>
        <class name="Shop.Core.PriceCalculator/&lt;&gt;c" filename="src/Shop.Core/PriceCalculator.cs" line-rate="1" branch-rate="1" complexity="1">
          <methods />
          <lines>
            <line number="20" hits="2" branch="False" />
          </lines>
        </class>
        <class name="Shop.Core.Order" filename="src/Shop.Core/Order.cs" line-rate="0" branch-rate="1" complexity="1">
          <methods />
          <lines>
            <line number="5" hits="0" branch="False" />
          </lines>
        </class>
      </classes>
    </package>
  </packages>
</coverage>"#;

    fn calc() -> PathBuf {
        Path::new("/repo/").join("src/Shop.Core/PriceCalculator.cs")
    }

    #[test]
    fn parses_coverlet_cobertura() {
        let r = parse_cobertura(SAMPLE).unwrap();
        assert_eq!(r.files.len(), 2);
        let f = r.file(&calc()).unwrap();
        assert_eq!(f.lines, BTreeMap::from([(9, 4), (10, 4), (11, 0), (19, 2)]));
        assert_eq!(f.branches, BTreeMap::from([(10, Ratio::new(1, 2))]));
        assert_eq!(f.ratio, Ratio::new(3, 4));
        assert_eq!(f.branch_ratio, Ratio::new(1, 2));
        assert_eq!(f.line_state(9), Some(LineState::Covered));
        assert_eq!(f.line_state(10), Some(LineState::Partial));
        assert_eq!(f.line_state(11), Some(LineState::NotCovered));
        assert_eq!(f.line_state(0), None);
        assert_eq!(r.total, Ratio::new(3, 5));
        assert_eq!(r.branch_total, Ratio::new(1, 2));
        assert_eq!(r.total.percent(), Some(60.0));

        // `<>c` folded into its class.
        let names: Vec<(&str, Ratio)> = r.classes.iter().map(|c| (c.name.as_str(), c.ratio)).collect();
        assert_eq!(names, vec![("Shop.Core.Order", Ratio::new(0, 1)), ("Shop.Core.PriceCalculator", Ratio::new(3, 4))]);
        assert_eq!(r.assemblies, vec![AssemblyCoverage { name: "Shop.Core".into(), ratio: Ratio::new(3, 5), branch_ratio: Ratio::new(1, 2) }]);
    }

    #[test]
    fn merges_reports_of_several_test_projects() {
        let mut a = parse_cobertura(SAMPLE).unwrap();
        let other = SAMPLE.replace(r#"number="12" hits="0""#, r#"number="12" hits="1""#).replace("50% (1/2)", "100% (2/2)").replace(r#"number="5" hits="0""#, r#"number="5" hits="3""#);
        a.merge(parse_cobertura(&other).unwrap());
        let f = a.file(&calc()).unwrap();
        assert_eq!(f.lines[&9], 8);
        assert_eq!(f.lines[&11], 1);
        assert_eq!(f.branches[&10], Ratio::new(2, 2));
        assert_eq!(a.total, Ratio::new(5, 5));
        assert_eq!(a.classes.len(), 2);
        assert_eq!(a.assemblies.len(), 1);
        assert_eq!(a.assemblies[0].ratio, Ratio::new(5, 5));
    }

    #[test]
    fn resolves_paths_and_rejects_other_xml() {
        let abs = r#"<coverage><packages><package name="A"><classes><class name="A.B" filename="/x/B.cs"><lines><line number="1" hits="1"/></lines></class></classes></package></packages></coverage>"#;
        let r = parse_cobertura(abs).unwrap();
        assert!(r.files.contains_key(Path::new("/x/B.cs")));
        assert_eq!(Ratio::default().percent(), None);
        assert!(parse_cobertura("<TestRun/>").is_err());
        assert_eq!(parse_condition_coverage("33.33% (1/3)"), Some(Ratio::new(1, 3)));
        assert_eq!(fold_class_name("N.C/<Run>d__2"), "N.C");
        assert_eq!(fold_class_name("N.C/Inner"), "N.C/Inner");
    }

    #[test]
    fn detects_the_collector() {
        let tests = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/Shop/tests");
        assert!(has_collector(&tests.join("Shop.Core.XunitTests/Shop.Core.XunitTests.csproj")));
        assert!(has_collector(&tests.join("Shop.Core.NunitTests/Shop.Core.NunitTests.csproj")));
        assert!(!has_collector(&tests.join("Shop.Core.MSTests/Shop.Core.MSTests.csproj")));
        assert!(missing_collector_message(&[PathBuf::from("/a/T.csproj")]).contains("dotnet add package coverlet.collector"));
    }
}
