//! Cancellable BenchmarkDotNet runs and their exported result summaries.

use rc_core::{Inbox, Runtime, Waker};
use rc_dotnet::{BuildEvent, Dotnet, RunEvent, RunHandle, RunRequest};
use rc_kit::i18n::{tr, trf};
use std::collections::BTreeMap;
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
    pub status: String,
    pub results: Vec<BenchmarkResult>,
    pub output: Vec<String>,
}

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
            status: String::new(),
            results: Vec::new(),
            output: Vec::new(),
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
        let artifacts = rc_core::paths::data_dir()
            .join("benchmarks")
            .join(format!("{stamp}-{}-{run_id}", std::process::id()));
        self.events = Inbox::new(self.waker.clone());
        self.parsed = Inbox::new(self.waker.clone());
        let request = run_request(&project, &filter, &artifacts);
        self.run = Some(rc_dotnet::start_run(&dotnet, request, self.events.sender()));
        self.artifacts = Some(artifacts);
        self.last = Some((dotnet, project, filter));
        self.stop_requested = false;
        self.running = true;
        self.status = rc_kit::i18n::tr("Building benchmark project…").into();
        self.results.clear();
        self.output.clear();
        Ok(())
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

    pub fn update(&mut self) {
        for event in self.events.drain() {
            match event {
                RunEvent::Build(BuildEvent::Started { command }) => self.push_output(command),
                RunEvent::Build(BuildEvent::Output(line)) => {
                    self.push_output(rc_dotnet::strip_ansi(&line.text))
                }
                RunEvent::Build(BuildEvent::Finished(result)) if !result.success => {
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
                        self.status = rc_kit::i18n::tr("Benchmark run failed").into();
                    }
                }
                RunEvent::Failed(error) => {
                    self.run.take();
                    self.running = false;
                    if !self.stop_requested {
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
                Err(error) => self.status = error,
            }
        }
    }

    fn push_output(&mut self, line: String) {
        self.output.push(line);
        if self.output.len() > 500 {
            self.output.remove(0);
        }
    }
}

impl Drop for Benchmarks {
    fn drop(&mut self) {
        self.stop();
    }
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
