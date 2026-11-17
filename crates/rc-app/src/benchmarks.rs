//! Cancellable BenchmarkDotNet runs and their exported result summaries.

use rc_core::{Inbox, Runtime, Waker};
use rc_dotnet::{BuildEvent, Dotnet, RunEvent, RunHandle, RunRequest};
use rc_kit::i18n::{tr, trf};
use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_RUN: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, PartialEq)]
pub struct BenchmarkResult {
    pub name: String,
    pub mean_ns: f64,
    pub error_ns: Option<f64>,
    pub stddev_ns: Option<f64>,
    pub allocated_bytes: Option<f64>,
}

pub struct Benchmarks {
    waker: Waker,
    events: Inbox<RunEvent>,
    parsed: Inbox<Result<Vec<BenchmarkResult>, String>>,
    run: Option<RunHandle>,
    parser: Option<tokio::task::JoinHandle<()>>,
    artifacts: Option<PathBuf>,
    last: Option<(Dotnet, PathBuf, String)>,
    stop_requested: bool,
    pub running: bool,
    /// The last run failed (build, process or report): the status is an error.
    pub failed: bool,
    pub status: String,
    pub results: Vec<BenchmarkResult>,
    pub output: VecDeque<String>,
    /// The Output section of the window is expanded.
    pub show_output: bool,
}

/// Lines of build and run output kept for the window.
const OUTPUT_LINES: usize = 500;

impl Benchmarks {
    pub fn new(waker: Waker) -> Self {
        Self {
            events: Inbox::new(waker.clone()),
            parsed: Inbox::new(waker.clone()),
            waker,
            run: None,
            parser: None,
            artifacts: None,
            last: None,
            stop_requested: false,
            running: false,
            failed: false,
            status: String::new(),
            results: Vec::new(),
            output: VecDeque::new(),
            show_output: false,
        }
    }

    pub fn start(
        &mut self,
        dotnet: Dotnet,
        project: PathBuf,
        filter: String,
    ) -> Result<(), String> {
        self.stop();
        let run_id = NEXT_RUN.fetch_add(1, Ordering::Relaxed);
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let root = rc_core::paths::data_dir().join("benchmarks");
        // Only the latest run keeps its artifacts: the results are already in memory.
        if let Some(previous) = self.artifacts.take() {
            discard(vec![previous]);
        }
        discard(stale_runs(&root, STALE_AFTER));
        let artifacts = root.join(format!("{stamp}-{}-{run_id}", std::process::id()));
        self.events = Inbox::new(self.waker.clone());
        self.parsed = Inbox::new(self.waker.clone());
        let request = run_request(&project, &filter, &artifacts);
        self.run = Some(rc_dotnet::start_run(&dotnet, request, self.events.sender()));
        self.artifacts = Some(artifacts);
        self.last = Some((dotnet, project, filter));
        self.stop_requested = false;
        self.running = true;
        self.failed = false;
        self.status = rc_kit::i18n::tr("Building benchmark project…").into();
        self.results.clear();
        self.output.clear();
        Ok(())
    }

    /// A run or the reading of its reports is in progress.
    pub fn busy(&self) -> bool {
        self.running || self.parser.is_some()
    }

    pub fn rerun(&mut self) -> Result<(), String> {
        let (dotnet, project, filter) = self
            .last
            .clone()
            .ok_or_else(|| tr("No benchmark run to repeat").to_string())?;
        self.start(dotnet, project, filter)
    }

    pub fn stop(&mut self) {
        self.stop_requested = true;
        let was_running = self.run.is_some() || self.parser.is_some();
        if let Some(run) = self.run.take() {
            run.stop();
        }
        if let Some(parser) = self.parser.take() {
            parser.abort();
        }
        self.events.drain();
        self.parsed.drain();
        self.running = false;
        if was_running {
            self.status = tr("Benchmarks stopped").into();
        }
    }

    /// Forgets everything (the workspace closed): stops the run and drops its results and artifacts.
    pub fn reset(&mut self) {
        self.stop();
        if let Some(artifacts) = self.artifacts.take() {
            discard(vec![artifacts]);
        }
        self.last = None;
        self.failed = false;
        self.status.clear();
        self.results.clear();
        self.output.clear();
        self.show_output = false;
    }

    pub fn update(&mut self) {
        for event in self.events.drain() {
            match event {
                RunEvent::Build(BuildEvent::Started { command }) => self.push_output(command),
                RunEvent::Build(BuildEvent::Output(line)) => {
                    self.push_output(rc_dotnet::strip_ansi(&line.text))
                }
                RunEvent::Build(BuildEvent::Finished(result)) if !result.success => {
                    self.failed = true;
                    self.status = rc_kit::i18n::tr("Benchmark project build failed").into();
                }
                RunEvent::Started { .. } => {
                    self.status = rc_kit::i18n::tr("Running benchmarks…").into()
                }
                RunEvent::Output(line) => self.push_output(rc_dotnet::strip_ansi(&line.text)),
                RunEvent::Exited {
                    code, cancelled, ..
                } => {
                    self.run.take();
                    self.running = false;
                    if cancelled || self.stop_requested {
                        self.status = rc_kit::i18n::tr("Benchmarks stopped").into();
                    } else if code == Some(0) {
                        self.status = rc_kit::i18n::tr("Reading benchmark results…").into();
                        if let Some(artifacts) = &self.artifacts {
                            let path = artifacts.clone();
                            let out = self.parsed.sender();
                            self.parser = Some(Runtime::spawn(async move {
                                let result =
                                    tokio::task::spawn_blocking(move || read_reports(&path)).await;
                                out.send(
                                    result
                                        .map_err(|error| error.to_string())
                                        .and_then(|result| result),
                                );
                            }));
                        }
                    } else {
                        self.failed = true;
                        self.status = rc_kit::i18n::tr("Benchmark run failed").into();
                    }
                }
                RunEvent::Failed(error) => {
                    self.run.take();
                    self.running = false;
                    if !self.stop_requested {
                        self.failed = true;
                        self.status = error;
                    }
                }
                _ => {}
            }
        }
        for result in self.parsed.drain() {
            self.parser.take();
            if self.stop_requested {
                continue;
            }
            match result {
                Ok(rows) => {
                    self.results = rows;
                    self.status = rc_kit::i18n::tr("Benchmark results ready").into();
                }
                Err(error) => {
                    self.failed = true;
                    self.status = error;
                }
            }
        }
    }

    fn push_output(&mut self, line: String) {
        if self.output.len() == OUTPUT_LINES {
            self.output.pop_front();
        }
        self.output.push_back(line);
    }
}

impl Drop for Benchmarks {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Runs left behind by other sessions (or a crash) are removed after this long.
const STALE_AFTER: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

/// Artifact folders under `root` not modified for `age`.
fn stale_runs(root: &Path, age: std::time::Duration) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(root) else { return Vec::new() };
    entries
        .flatten()
        .filter(|entry| {
            entry.metadata().and_then(|meta| meta.modified()).ok().and_then(|modified| modified.elapsed().ok()).is_some_and(|elapsed| elapsed >= age)
        })
        .map(|entry| entry.path())
        .collect()
}

/// Deletes artifact folders off the UI thread.
fn discard(dirs: Vec<PathBuf>) {
    if dirs.is_empty() {
        return;
    }
    std::thread::spawn(move || {
        for dir in dirs {
            let _ = std::fs::remove_dir_all(dir);
        }
    });
}

pub fn run_request(project: &Path, filter: &str, artifacts: &Path) -> RunRequest {
    let mut request = RunRequest::new(project);
    request.configuration = "Release".into();
    request.use_launch_profile = false;
    request.args = vec![
        "--filter".into(),
        filter.into(),
        "--exporters".into(),
        "json".into(),
        "--artifacts".into(),
        artifacts.to_string_lossy().into_owned(),
    ];
    request
}

fn read_reports(artifacts: &Path) -> Result<Vec<BenchmarkResult>, String> {
    let dir = artifacts.join("results");
    let entries = std::fs::read_dir(&dir).map_err(|error| {
        trf(
            "Could not read benchmark reports: {error}",
            &[("error", &error)],
        )
    })?;
    let mut by_name = BTreeMap::new();
    for entry in entries {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        if !name.contains("-report") || path.extension().is_none_or(|extension| extension != "json")
        {
            continue;
        }
        let metadata = entry.metadata().map_err(|error| error.to_string())?;
        if metadata.len() > 16 * 1024 * 1024 {
            continue;
        }
        let text = std::fs::read_to_string(&path).map_err(|error| error.to_string())?;
        for row in parse_report(&text)? {
            by_name.insert(row.name.clone(), row);
        }
    }
    if by_name.is_empty() {
        return Err(tr("BenchmarkDotNet produced no JSON results. Ensure the app passes arguments to BenchmarkSwitcher.").into());
    }
    Ok(by_name.into_values().collect())
}

fn parse_report(text: &str) -> Result<Vec<BenchmarkResult>, String> {
    let report: serde_json::Value =
        serde_json::from_str(text).map_err(|error| error.to_string())?;
    let benchmarks = report
        .get("Benchmarks")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| tr("Benchmark report has no Benchmarks array").to_string())?;
    let mut rows = Vec::new();
    for benchmark in benchmarks {
        let Some(mean_ns) = benchmark
            .pointer("/Statistics/Mean")
            .and_then(serde_json::Value::as_f64)
        else {
            continue;
        };
        let name = benchmark
            .get("DisplayInfo")
            .and_then(serde_json::Value::as_str)
            .or_else(|| {
                benchmark
                    .get("FullName")
                    .and_then(serde_json::Value::as_str)
            })
            .map(str::to_string)
            .unwrap_or_else(|| {
                format!(
                    "{}.{}",
                    benchmark["Type"].as_str().unwrap_or("?"),
                    benchmark["Method"].as_str().unwrap_or("?")
                )
            });
        rows.push(BenchmarkResult {
            name,
            mean_ns,
            error_ns: benchmark
                .pointer("/Statistics/ConfidenceInterval/Margin")
                .and_then(serde_json::Value::as_f64),
            stddev_ns: benchmark
                .pointer("/Statistics/StandardDeviation")
                .and_then(serde_json::Value::as_f64),
            allocated_bytes: benchmark
                .pointer("/Memory/BytesAllocatedPerOperation")
                .and_then(serde_json::Value::as_f64),
        });
    }
    Ok(rows)
}

pub fn format_time(ns: f64) -> String {
    if ns >= 1_000_000_000.0 {
        format!("{:.2} s", ns / 1_000_000_000.0)
    } else if ns >= 1_000_000.0 {
        format!("{:.2} ms", ns / 1_000_000.0)
    } else if ns >= 1_000.0 {
        format!("{:.2} µs", ns / 1_000.0)
    } else {
        format!("{ns:.2} ns")
    }
}

/// Bytes allocated per operation, in B, KB or MB.
pub fn format_bytes(bytes: f64) -> String {
    if bytes >= 1024.0 * 1024.0 {
        format!("{:.2} MB", bytes / (1024.0 * 1024.0))
    } else if bytes >= 1024.0 {
        format!("{:.2} KB", bytes / 1024.0)
    } else {
        format!("{bytes:.0} B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_request_passes_benchmark_cli_arguments() {
        let request = run_request(
            Path::new("/work/Perf.csproj"),
            "Perf.SortBench.Quick",
            Path::new("/tmp/results"),
        );
        assert_eq!(request.configuration, "Release");
        assert!(!request.use_launch_profile);
        assert_eq!(
            request.args,
            [
                "--filter",
                "Perf.SortBench.Quick",
                "--exporters",
                "json",
                "--artifacts",
                "/tmp/results"
            ]
        );
    }

    #[test]
    fn parses_benchmarkdotnet_json_statistics() {
        let json = r#"{"Benchmarks":[{"DisplayInfo":"SortBench.Quick: DefaultJob","Statistics":{"Mean":1240.0,"StandardDeviation":40.0,"ConfidenceInterval":{"Margin":20.0}},"Memory":{"BytesAllocatedPerOperation":64.0}}]}"#;
        let rows = parse_report(json).unwrap();
        assert_eq!(
            rows,
            [BenchmarkResult {
                name: "SortBench.Quick: DefaultJob".into(),
                mean_ns: 1240.0,
                error_ns: Some(20.0),
                stddev_ns: Some(40.0),
                allocated_bytes: Some(64.0)
            }]
        );
        assert_eq!(format_time(rows[0].mean_ns), "1.24 µs");
        assert_eq!(format_bytes(64.0), "64 B");
        assert_eq!(format_bytes(3072.0), "3.00 KB");
    }

    #[test]
    fn output_keeps_only_the_latest_lines() {
        let mut benchmarks = Benchmarks::new(Waker::noop());
        for i in 0..OUTPUT_LINES + 3 {
            benchmarks.push_output(i.to_string());
        }
        assert_eq!(benchmarks.output.len(), OUTPUT_LINES);
        assert_eq!(benchmarks.output.front().map(String::as_str), Some("3"));
    }

    #[test]
    fn only_old_artifact_folders_are_stale() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("run")).unwrap();
        assert!(stale_runs(root.path(), std::time::Duration::from_secs(3600)).is_empty());
        assert_eq!(stale_runs(root.path(), std::time::Duration::ZERO), [root.path().join("run")]);
        assert!(stale_runs(&root.path().join("missing"), std::time::Duration::ZERO).is_empty());
    }

    #[test]
    fn reads_exported_report_from_artifacts_directory() {
        let artifacts = tempfile::tempdir().unwrap();
        let results = artifacts.path().join("results");
        std::fs::create_dir(&results).unwrap();
        std::fs::write(
            results.join("SortBench-report.json"),
            r#"{"Benchmarks":[{"Type":"SortBench","Method":"Quick","Statistics":{"Mean":120.0}}]}"#,
        )
        .unwrap();
        let rows = read_reports(artifacts.path()).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "SortBench.Quick");
    }
}
