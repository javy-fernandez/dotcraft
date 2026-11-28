//! Docker CLI backed container inspection for the Services tool window.

use rc_core::{Inbox, Runtime, Waker};
use rc_kit::i18n::trf;
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Container {
    pub id: String,
    pub name: String,
    pub image: String,
    pub status: String,
    pub state: String,
}

impl Container {
    fn parse(line: &str) -> Option<Self> {
        let mut parts = line.splitn(5, '\t');
        Some(Self {
            id: parts.next()?.to_owned(),
            name: parts.next()?.to_owned(),
            image: parts.next()?.to_owned(),
            status: parts.next()?.to_owned(),
            state: parts.next()?.to_owned(),
        })
    }
}

enum Event {
    Containers(Result<Vec<Container>, String>),
    Logs(String, Result<String, String>),
    Action(Result<(), String>),
}

pub struct DockerServices {
    pub containers: Vec<Container>,
    pub selected: Option<String>,
    pub logs: String,
    pub error: Option<String>,
    /// Docker cannot be used (not installed, or its daemon is not running): the CLI's
    /// message, shown on hover behind a short note.
    pub unavailable: Option<Unavailable>,
    pub busy: bool,
    /// The Docker section of the Services window is expanded.
    pub open: bool,
    pub loaded: bool,
    inbox: Inbox<Event>,
    task: Option<tokio::task::JoinHandle<()>>,
    docker: PathBuf,
}

impl DockerServices {
    pub fn new(waker: Waker) -> Self {
        Self {
            containers: Vec::new(), selected: None, logs: String::new(), error: None, unavailable: None,
            busy: false, open: true, loaded: false, inbox: Inbox::new(waker), task: None,
            docker: docker_path(),
        }
    }

    pub fn refresh(&mut self) {
        if self.busy { return; }
        self.busy = true;
        let docker = self.docker.clone();
        let out = self.inbox.sender();
        self.task = Some(Runtime::spawn(async move {
            let result = command(&docker, &["ps", "--all", "--format", "{{.ID}}\t{{.Names}}\t{{.Image}}\t{{.Status}}\t{{.State}}"], false)
                .await.map(|text| text.lines().filter_map(Container::parse).collect());
            out.send(Event::Containers(result));
        }));
    }

    pub fn select(&mut self, id: String) {
        self.selected = Some(id.clone());
        self.logs.clear();
        self.load_logs(id);
    }

    pub fn load_logs(&mut self, id: String) {
        if self.busy { return; }
        self.busy = true;
        let docker = self.docker.clone();
        let out = self.inbox.sender();
        self.task = Some(Runtime::spawn(async move {
            let result = command(&docker, &["logs", "--tail", "300", &id], true).await;
            out.send(Event::Logs(id, result));
        }));
    }

    pub fn set_running(&mut self, id: String, start: bool) {
        if self.busy { return; }
        self.busy = true;
        let docker = self.docker.clone();
        let out = self.inbox.sender();
        self.task = Some(Runtime::spawn(async move {
            let verb = if start { "start" } else { "stop" };
            out.send(Event::Action(command(&docker, &[verb, &id], false).await.map(|_| ())));
        }));
    }

    pub fn poll(&mut self) {
        for event in self.inbox.drain() {
            self.busy = false;
            self.task.take();
            match event {
                Event::Containers(Ok(containers)) => {
                    self.containers = containers;
                    self.loaded = true;
                    self.error = None;
                    self.unavailable = None;
                    if self.selected.as_ref().is_some_and(|id| !self.containers.iter().any(|c| &c.id == id)) {
                        self.selected = None;
                        self.logs.clear();
                    }
                }
                Event::Containers(Err(error)) if !self.docker.is_file() || daemon_unreachable(&error) => {
                    let installed = self.docker.is_file();
                    self.unavailable = Some(Unavailable { installed, details: error });
                    self.containers.clear();
                    self.selected = None;
                    self.error = None;
                    self.loaded = true;
                }
                Event::Containers(Err(error)) | Event::Action(Err(error)) | Event::Logs(_, Err(error)) => {
                    self.error = Some(error);
                    self.loaded = true;
                }
                Event::Logs(id, Ok(logs)) => {
                    if self.selected.as_ref() == Some(&id) { self.logs = logs; }
                    self.error = None;
                }
                Event::Action(Ok(())) => self.refresh(),
            }
        }
    }

    pub fn cancel(&mut self) {
        if let Some(task) = self.task.take() { task.abort(); }
        self.inbox.drain();
        self.busy = false;
    }
}

impl Drop for DockerServices {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() { task.abort(); }
    }
}

/// Why Docker cannot be used, for the Services window's note.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unavailable {
    /// The CLI was found (so its daemon is what is missing).
    pub installed: bool,
    /// What the CLI said.
    pub details: String,
}

/// Whether the CLI failed because no daemon answers (Docker Desktop, Colima… not started).
pub(crate) fn daemon_unreachable(error: &str) -> bool {
    let e = error.to_ascii_lowercase();
    ["cannot connect to the docker daemon", "failed to connect to the docker api", "is the docker daemon running", "error during connect", "docker.sock", "docker_engine"].iter().any(|s| e.contains(s))
}

/// The Docker CLI: on PATH, or where Docker Desktop, Homebrew and Colima install it (apps
/// opened from Finder get a PATH without /usr/local/bin).
pub(crate) fn docker_path() -> PathBuf {
    let exe = if cfg!(windows) { "docker.exe" } else { "docker" };
    let mut candidates: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).map(|d| d.join(exe)).collect()).unwrap_or_default();
    if cfg!(windows) {
        candidates.push(PathBuf::from(r"C:\Program Files\Docker\Docker\resources\bin\docker.exe"));
    } else {
        candidates.extend(["/usr/local/bin/docker", "/opt/homebrew/bin/docker", "/Applications/Docker.app/Contents/Resources/bin/docker", "/usr/bin/docker"].map(PathBuf::from));
        if let Some(home) = dirs::home_dir() {
            candidates.push(home.join(".docker/bin/docker"));
        }
    }
    candidates.into_iter().find(|p| p.is_file()).unwrap_or_else(|| PathBuf::from(exe))
}

async fn command(program: &PathBuf, args: &[&str], include_stderr: bool) -> Result<String, String> {
    let output = tokio::process::Command::new(program)
        .args(args).kill_on_drop(true).output().await
        .map_err(|error| trf("Could not start Docker: {error}", &[("error", &error)]))?;
    if output.status.success() {
        let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
        if include_stderr { text.push_str(&String::from_utf8_lossy(&output.stderr)); }
        Ok(text)
    } else {
        let message = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        Err(if message.is_empty() { trf("Docker exited with {status}", &[("status", &output.status)]) } else { message })
    }
}

#[cfg(test)]
mod tests {
    use super::Container;

    #[test]
    fn parses_docker_container_row() {
        let row = Container::parse("abc123\tweb\tmyapp:latest\tUp 10 minutes\trunning").unwrap();
        assert_eq!(row.id, "abc123");
        assert_eq!(row.name, "web");
        assert_eq!(row.state, "running");
        assert!(Container::parse("bad row").is_none());
    }

    #[test]
    fn a_stopped_daemon_is_not_an_error() {
        assert!(super::daemon_unreachable("failed to connect to the docker API at unix:///Users/me/.docker/run/docker.sock; check if the path is correct and if the daemon is running"));
        assert!(super::daemon_unreachable("Cannot connect to the Docker daemon at unix:///var/run/docker.sock. Is the docker daemon running?"));
        assert!(super::daemon_unreachable("error during connect: Get \"http://%2F%2F.%2Fpipe%2Fdocker_engine/v1.47/containers/json\""));
        assert!(!super::daemon_unreachable("Error response from daemon: No such container: web"));
        let mut services = super::DockerServices::new(rc_core::Waker::noop());
        services.docker = std::env::current_exe().unwrap();
        services.inbox.sender().send(super::Event::Containers(Err("Cannot connect to the Docker daemon at unix:///var/run/docker.sock. Is the docker daemon running?".into())));
        services.poll();
        assert!(services.error.is_none());
        assert!(services.unavailable.as_ref().is_some_and(|u| u.installed && u.details.contains("daemon")));
        services.inbox.sender().send(super::Event::Containers(Ok(Vec::new())));
        services.poll();
        assert!(services.unavailable.is_none());
    }
}
