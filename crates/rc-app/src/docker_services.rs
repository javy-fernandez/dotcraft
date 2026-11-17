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
    pub busy: bool,
    pub loaded: bool,
    inbox: Inbox<Event>,
    task: Option<tokio::task::JoinHandle<()>>,
    docker: PathBuf,
}

impl DockerServices {
    pub fn new(waker: Waker) -> Self {
        Self {
            containers: Vec::new(), selected: None, logs: String::new(), error: None,
            busy: false, loaded: false, inbox: Inbox::new(waker), task: None,
            docker: PathBuf::from("docker"),
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
                    if self.selected.as_ref().is_some_and(|id| !self.containers.iter().any(|c| &c.id == id)) {
                        self.selected = None;
                        self.logs.clear();
                    }
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
}
