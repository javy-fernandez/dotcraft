//! Aspire AppHost resources and console logs, queried through the Aspire CLI off the UI thread.

use rc_core::{Inbox, Waker};
use rc_kit::i18n::{tr, trf};
use serde_json::Value;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resource {
    pub name: String,
    pub kind: String,
    pub state: String,
    pub health: String,
    pub urls: Vec<String>,
}

enum ResultEvent {
    Resources(PathBuf, Result<Vec<Resource>, String>),
    Logs(PathBuf, String, Result<String, String>),
}

pub struct Aspire {
    pub hosts: Vec<PathBuf>,
    pub selected_host: usize,
    pub resources: Vec<Resource>,
    pub selected_resource: Option<String>,
    pub logs: String,
    pub error: Option<String>,
    pub loading_resources: bool,
    pub loading_logs: bool,
    inbox: Inbox<ResultEvent>,
    cancel: Arc<AtomicBool>,
    last_refresh: Option<Instant>,
}

impl Aspire {
    pub fn new(waker: Waker) -> Self {
        Self {
            hosts: Vec::new(),
            selected_host: 0,
            resources: Vec::new(),
            selected_resource: None,
            logs: String::new(),
            error: None,
            loading_resources: false,
            loading_logs: false,
            inbox: Inbox::new(waker),
            cancel: Arc::new(AtomicBool::new(false)),
            last_refresh: None,
        }
    }

    pub fn set_hosts(&mut self, hosts: Vec<PathBuf>) {
        self.cancel.store(true, Ordering::Relaxed);
        self.cancel = Arc::new(AtomicBool::new(false));
        self.hosts = hosts;
        self.selected_host = 0;
        self.resources.clear();
        self.selected_resource = None;
        self.logs.clear();
        self.error = None;
        self.loading_resources = false;
        self.loading_logs = false;
        self.last_refresh = None;
    }

    pub fn host(&self) -> Option<&Path> {
        self.hosts.get(self.selected_host).map(PathBuf::as_path)
    }

    pub fn select_host(&mut self, index: usize) {
        if index >= self.hosts.len() || index == self.selected_host {
            return;
        }
        let hosts = self.hosts.clone();
        self.set_hosts(hosts);
        self.selected_host = index;
    }

    pub fn refresh(&mut self) {
        if self.loading_resources {
            return;
        }
        let Some(host) = self.host().map(Path::to_path_buf) else {
            return;
        };
        self.loading_resources = true;
        self.last_refresh = Some(Instant::now());
        let out = self.inbox.sender();
        let cancel = self.cancel.clone();
        std::thread::spawn(move || {
            let result = run_cli(&host, &["describe", "--format", "Json"], &cancel)
                .and_then(|text| parse_resources(&text));
            out.send(ResultEvent::Resources(host, result));
        });
    }

    pub fn refresh_logs(&mut self) {
        if self.loading_logs {
            return;
        }
        let (Some(host), Some(resource)) = (
            self.host().map(Path::to_path_buf),
            self.selected_resource.clone(),
        ) else {
            return;
        };
        self.loading_logs = true;
        let out = self.inbox.sender();
        let cancel = self.cancel.clone();
        std::thread::spawn(move || {
            let result = run_cli(
                &host,
                &["logs", &resource, "--tail", "200", "--timestamps"],
                &cancel,
            );
            out.send(ResultEvent::Logs(host, resource, result));
        });
    }

    pub fn select_resource(&mut self, name: &str) {
        if self.selected_resource.as_deref() == Some(name) {
            return;
        }
        self.selected_resource = Some(name.to_string());
        self.logs.clear();
        self.loading_logs = false;
        self.refresh_logs();
    }

    pub fn poll(&mut self, active: bool) {
        if !active && self.last_refresh.is_some() {
            self.cancel.store(true, Ordering::Relaxed);
            self.cancel = Arc::new(AtomicBool::new(false));
            self.resources.clear();
            self.selected_resource = None;
            self.logs.clear();
            self.loading_resources = false;
            self.loading_logs = false;
            self.last_refresh = None;
        }
        if !active {
            self.inbox.drain();
            return;
        }
        for event in self.inbox.drain() {
            match event {
                ResultEvent::Resources(host, result) if self.host() == Some(host.as_path()) => {
                    self.loading_resources = false;
                    match result {
                        Ok(resources) => {
                            self.error = None;
                            self.resources = resources;
                            if self
                                .selected_resource
                                .as_ref()
                                .is_some_and(|name| !self.resources.iter().any(|r| &r.name == name))
                            {
                                self.selected_resource = None;
                                self.logs.clear();
                            }
                            self.refresh_logs();
                        }
                        Err(error) => self.error = Some(error),
                    }
                }
                ResultEvent::Logs(host, resource, result)
                    if self.host() == Some(host.as_path())
                        && self.selected_resource.as_deref() == Some(&resource) =>
                {
                    self.loading_logs = false;
                    match result {
                        Ok(logs) => {
                            self.logs = logs;
                            self.error = None;
                        }
                        Err(error) => self.error = Some(error),
                    }
                }
                _ => {}
            }
        }
        if active
            && self.host().is_some()
            && !self.loading_resources
            && self
                .last_refresh
                .is_none_or(|t| t.elapsed() >= Duration::from_secs(4))
        {
            self.refresh();
        }
    }
}

impl Drop for Aspire {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

fn run_cli(host: &Path, args: &[&str], cancel: &AtomicBool) -> Result<String, String> {
    let mut child = Command::new("aspire")
        .args(args)
        .arg("--apphost")
        .arg(host)
        .args(["--non-interactive", "--nologo"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                tr("Install Aspire CLI to inspect resources and logs.").to_string()
            } else {
                trf(
                    "Aspire CLI could not start: {error}",
                    &[("error", &e.to_string())],
                )
            }
        })?;
    let mut stdout = child.stdout.take().expect("piped stdout");
    let mut stderr = child.stderr.take().expect("piped stderr");
    let out_reader = std::thread::spawn(move || {
        let mut text = Vec::new();
        let _ = stdout.read_to_end(&mut text);
        text
    });
    let err_reader = std::thread::spawn(move || {
        let mut text = Vec::new();
        let _ = stderr.read_to_end(&mut text);
        text
    });
    let start = Instant::now();
    let status = loop {
        if cancel.load(Ordering::Relaxed) || start.elapsed() > Duration::from_secs(15) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(tr("Aspire CLI request was cancelled or timed out").to_string());
        }
        match child.try_wait().map_err(|e| e.to_string())? {
            Some(status) => break status,
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    };
    let stdout = String::from_utf8_lossy(&out_reader.join().unwrap_or_default()).into_owned();
    let stderr = String::from_utf8_lossy(&err_reader.join().unwrap_or_default()).into_owned();
    if !status.success() {
        return Err(stderr
            .lines()
            .last()
            .unwrap_or(tr("Aspire CLI request failed"))
            .to_string());
    }
    Ok(stdout)
}

fn field(value: &Value, keys: &[&str]) -> String {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
        .unwrap_or("")
        .to_string()
}

pub fn parse_resources(text: &str) -> Result<Vec<Resource>, String> {
    let value: Value = serde_json::from_str(text).map_err(|e| {
        trf(
            "Invalid Aspire resource response: {error}",
            &[("error", &e.to_string())],
        )
    })?;
    let rows = value
        .get("resources")
        .or_else(|| value.get("Resources"))
        .and_then(Value::as_array)
        .ok_or_else(|| tr("Aspire response has no resources array").to_string())?;
    rows.iter()
        .map(|row| {
            let name = field(row, &["name", "Name"]);
            if name.is_empty() {
                return Err(tr("Aspire resource has no name").to_string());
            }
            let urls = row
                .get("urls")
                .or_else(|| row.get("Urls"))
                .or_else(|| row.get("endpoints"))
                .or_else(|| row.get("Endpoints"))
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|u| {
                            u.as_str().map(str::to_string).or_else(|| {
                                u.get("url")
                                    .or_else(|| u.get("Url"))
                                    .and_then(Value::as_str)
                                    .map(str::to_string)
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            Ok(Resource {
                name,
                kind: field(row, &["type", "Type", "resourceType", "ResourceType"]),
                state: field(row, &["state", "State"]),
                health: field(row, &["healthStatus", "HealthStatus", "health", "Health"]),
                urls,
            })
        })
        .collect()
}

/// The AppHost prints a token-bearing dashboard URL on startup. Keep it in memory only.
pub fn dashboard_url(line: &str) -> Option<String> {
    let lower = line.to_ascii_lowercase();
    if !lower.contains("dashboard") {
        return None;
    }
    let start = line.find("https://").or_else(|| line.find("http://"))?;
    let url = line[start..]
        .split_whitespace()
        .next()?
        .split('\u{1b}')
        .next()?
        .trim_end_matches(['.', ',', ';', ')', '\u{1b}']);
    Some(url.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_project_container_and_database_resources() {
        let rows = parse_resources(r#"{"resources":[{"name":"api","resourceType":"project","state":"Running","healthStatus":"Healthy","urls":[{"url":"https://localhost:7001"}]},{"name":"redis","resourceType":"container","state":"Running","urls":[{"url":"tcp://localhost:6379"}]},{"name":"postgres","resourceType":"database","state":"Starting"}]}"#).unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].kind, "project");
        assert_eq!(rows[0].health, "Healthy");
        assert_eq!(rows[0].urls, ["https://localhost:7001"]);
        assert_eq!(rows[1].urls, ["tcp://localhost:6379"]);
        assert_eq!(rows[2].state, "Starting");
    }

    #[test]
    fn captures_only_dashboard_url() {
        assert_eq!(
            dashboard_url("Now listening on: https://localhost:1234"),
            None
        );
        assert_eq!(
            dashboard_url("Aspire dashboard: https://localhost:17287/login?t=abc"),
            Some("https://localhost:17287/login?t=abc".into())
        );
        assert_eq!(
            dashboard_url("Aspire dashboard: https://localhost:17287/login?t=abc\u{1b}[0m"),
            Some("https://localhost:17287/login?t=abc".into())
        );
    }

    #[test]
    fn apphost_is_available_as_a_run_configuration_and_resource_host() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("Store.AppHost.csproj");
        std::fs::write(&project, "<Project Sdk=\"Aspire.AppHost.Sdk/13.6.0\"><PropertyGroup><TargetFramework>net10.0</TargetFramework></PropertyGroup></Project>").unwrap();
        let mut runner = crate::runner::Runner::new(Waker::noop());
        runner.set_workspace(None, &[project.clone()], Path::new("dotnet"), None);
        assert_eq!(runner.aspire_hosts, [project.clone()]);
        assert!(
            matches!(&runner.configs[0].kind, crate::runner::ConfigKind::App { project: configured, .. } if configured == &project)
        );
    }
}
