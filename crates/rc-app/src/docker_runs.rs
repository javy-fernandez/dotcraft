//! Dockerfile and Compose run configurations, executed without blocking the UI.

use crate::runner::{ConfigKind, RunConfig, Runner};
use rc_core::{Inbox, Runtime, Waker};
use rc_core::inbox::Outbox;
use rc_kit::i18n::{tr, trf};
use rc_term::TermSession;
use std::path::{Path, PathBuf};
use tokio::io::{AsyncBufReadExt, BufReader};

enum Event {
    Configs(Vec<RunConfig>),
    Line(u64, String),
    Done(u64, Result<(), String>),
}

pub struct DockerRuns {
    inbox: Inbox<Event>,
    discovery: Option<tokio::task::JoinHandle<()>>,
    process: Option<tokio::task::JoinHandle<()>>,
    pub console: TermSession,
    pub active: bool,
    pub running: bool,
    pub title: String,
    pub error: Option<String>,
    current: Option<RunConfig>,
    remembered: Option<String>,
    generation: u64,
}

impl DockerRuns {
    pub fn new(waker: Waker) -> Self {
        Self {
            inbox: Inbox::new(waker.clone()), discovery: None, process: None,
            console: TermSession::output_only(160, 40, waker), active: false, running: false,
            title: String::new(), error: None, current: None, remembered: None, generation: 0,
        }
    }

    pub fn discover(&mut self, root: PathBuf, remembered: Option<String>) {
        if let Some(task) = self.discovery.take() { task.abort(); }
        self.inbox.drain();
        self.remembered = remembered;
        let out = self.inbox.sender();
        self.discovery = Some(Runtime::spawn(async move {
            if let Ok(configs) = tokio::task::spawn_blocking(move || discover(&root)).await {
                out.send(Event::Configs(configs));
            }
        }));
    }

    pub fn poll(&mut self, runner: &mut Runner) {
        for event in self.inbox.drain() {
            match event {
                Event::Configs(configs) => {
                    runner.configs.retain(|cfg| !matches!(cfg.kind, ConfigKind::Docker { .. }));
                    runner.configs.extend(configs);
                    if let Some(name) = self.remembered.take()
                        && let Some(index) = runner.configs.iter().position(|cfg| cfg.name == name)
                    {
                        runner.selected = index;
                    }
                    self.discovery.take();
                }
                Event::Line(generation, line) if generation == self.generation => self.console.feed(line.replace('\n', "\r\n").as_bytes()),
                Event::Done(generation, result) if generation == self.generation => {
                    self.running = false;
                    self.process.take();
                    match result {
                        Ok(()) => self.console.feed(format!("\r\n{}\r\n", tr("Docker run finished")).as_bytes()),
                        Err(error) => {
                            self.error = Some(error.clone());
                            self.console.feed(format!("\r\n{error}\r\n").as_bytes());
                        }
                    }
                }
                Event::Line(_, _) | Event::Done(_, _) => {}
            }
        }
    }

    pub fn start(&mut self, config: RunConfig) {
        if self.running && self.current.as_ref() == Some(&config) { return; }
        self.stop();
        let ConfigKind::Docker { file, compose } = &config.kind else { return };
        self.active = true;
        self.running = true;
        self.title = config.name.clone();
        self.error = None;
        self.current = Some(config.clone());
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.console.feed(format!("\x1b[2J\x1b[H> {}\r\n", self.title).as_bytes());
        let out = self.inbox.sender();
        let (file, compose) = (file.clone(), *compose);
        self.process = Some(Runtime::spawn(async move {
            let result = if compose { run_compose(&file, generation, &out).await } else { run_dockerfile(&file, generation, &out).await };
            out.send(Event::Done(generation, result));
        }));
    }

    pub fn rerun(&mut self) {
        if let Some(config) = self.current.clone() { self.start(config); }
    }

    pub fn current_dir(&self) -> Option<PathBuf> {
        self.current.as_ref().and_then(|config| match &config.kind {
            ConfigKind::Docker { file, .. } => file.parent().map(Path::to_path_buf),
            _ => None,
        })
    }

    pub fn stop(&mut self) {
        let was_running = self.running;
        if was_running { self.generation = self.generation.wrapping_add(1); }
        if let Some(task) = self.process.take() { task.abort(); }
        self.running = false;
        if !was_running { return; }
        if let Some(RunConfig { kind: ConfigKind::Docker { file, compose }, .. }) = &self.current {
            let file = file.clone();
            let compose = *compose;
            Runtime::spawn(async move {
                if compose {
                    let _ = tokio::process::Command::new("docker").args(["compose", "-f"]).arg(&file).args(["stop", "-t", "5"]).kill_on_drop(true).output().await;
                } else {
                    let name = container_name(&file);
                    let _ = tokio::process::Command::new("docker").args(["stop", "--time", "5", &name]).kill_on_drop(true).output().await;
                }
            });
        }
    }

    pub fn clear(&mut self) {
        self.stop();
        if let Some(task) = self.discovery.take() { task.abort(); }
        self.inbox.drain();
        self.current = None;
        self.active = false;
    }
}

impl Drop for DockerRuns {
    fn drop(&mut self) { self.clear(); }
}

fn discover(root: &Path) -> Vec<RunConfig> {
    let mut configs = Vec::new();
    let mut dirs = vec![(root.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = dirs.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            if entry.file_type().is_ok_and(|kind| kind.is_symlink()) { continue; }
            let path = entry.path();
            if path.is_dir() && depth < 4 {
                if !matches!(entry.file_name().to_str(), Some(".git" | "bin" | "obj" | "node_modules")) {
                    dirs.push((path, depth + 1));
                }
            } else if path.is_file() {
                let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
                let compose = matches!(name.as_str(), "compose.yaml" | "compose.yml" | "docker-compose.yaml" | "docker-compose.yml");
                let dockerfile = name == "dockerfile" || name.starts_with("dockerfile.") || name == "containerfile" || name.starts_with("containerfile.");
                if compose || dockerfile {
                    let relative = path.strip_prefix(root).unwrap_or(&path).display();
                    configs.push(RunConfig {
                        name: format!("{}: {relative}", if compose { tr("Compose") } else { tr("Docker") }),
                        kind: ConfigKind::Docker { file: path, compose },
                    });
                }
            }
        }
    }
    configs.sort_by(|a, b| a.name.cmp(&b.name));
    configs
}

fn container_name(file: &Path) -> String {
    let hash = file.to_string_lossy().bytes().fold(0xcbf29ce484222325u64, |acc, byte| (acc ^ u64::from(byte)).wrapping_mul(0x100000001b3));
    format!("dotcraft-{hash:016x}")
}

async fn run_dockerfile(file: &Path, generation: u64, out: &Outbox<Event>) -> Result<(), String> {
    let name = container_name(file);
    let image = format!("{name}:latest");
    let context = file.parent().unwrap_or_else(|| Path::new("."));
    stream(&["build", "-t", &image, "-f"], &[file, context], &[], generation, out).await?;
    stream(&["run", "--rm", "--name", &name, &image], &[], &[], generation, out).await
}

async fn run_compose(file: &Path, generation: u64, out: &Outbox<Event>) -> Result<(), String> {
    stream(&["compose", "-f"], &[file], &["up", "--build"], generation, out).await?;
    Ok(())
}

async fn stream(args: &[&str], paths: &[&Path], tail: &[&str], generation: u64, out: &Outbox<Event>) -> Result<(), String> {
    let mut command = tokio::process::Command::new("docker");
    command.args(args);
    for path in paths { command.arg(path); }
    command.args(tail);
    command.stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).kill_on_drop(true);
    let mut child = command.spawn().map_err(|error| trf("Could not start Docker: {error}", &[("error", &error)]))?;
    let stdout = child.stdout.take().expect("piped");
    let stderr = child.stderr.take().expect("piped");
    let output = out.clone();
    let errors = tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await { output.send(Event::Line(generation, format!("{line}\n"))); }
    });
    let mut lines = BufReader::new(stdout).lines();
    while let Ok(Some(line)) = lines.next_line().await { out.send(Event::Line(generation, format!("{line}\n"))); }
    let status = child.wait().await.map_err(|error| error.to_string())?;
    let _ = errors.await;
    if status.success() { Ok(()) } else { Err(trf("Docker exited with {status}", &[("status", &status)])) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_dockerfile_and_compose_configs_without_generated_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Dockerfile"), "FROM scratch\n").unwrap();
        std::fs::write(dir.path().join("compose.yaml"), "services: {}\n").unwrap();
        std::fs::create_dir(dir.path().join("obj")).unwrap();
        std::fs::write(dir.path().join("obj/Dockerfile"), "FROM scratch\n").unwrap();
        let configs = discover(dir.path());
        assert_eq!(configs.len(), 2);
        assert!(configs.iter().any(|cfg| matches!(cfg.kind, ConfigKind::Docker { compose: true, .. })));
        assert!(configs.iter().any(|cfg| matches!(cfg.kind, ConfigKind::Docker { compose: false, .. })));
    }
}
