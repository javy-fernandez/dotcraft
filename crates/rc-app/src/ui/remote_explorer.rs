//! Read-only remote workspace tree backed by the remote agent.

use egui::Ui;
use rc_core::{Inbox, Runtime, Waker};
use rc_kit::i18n::tr;
use rc_remote::{Endpoint, Entry, Operation, RemoteSession, Value};
use std::collections::{HashMap, HashSet};
use tokio::sync::mpsc;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    Ssh,
    Wsl,
}

enum Request {
    List(String),
    Read(String),
}
enum Event {
    Connected(Vec<Entry>),
    Listed(String, Vec<Entry>),
    Read(String, String, bool),
    Failed(Option<String>, String),
}

pub enum Action {
    Connect,
    Close,
    Cancel,
}

pub struct RemoteExplorer {
    pub show_connect: bool,
    pub transport: Transport,
    pub host: String,
    pub agent: String,
    pub root_input: String,
    root: Option<String>,
    connected: bool,
    connecting: bool,
    cache: HashMap<String, Vec<Entry>>,
    loading: HashSet<String>,
    expanded: HashSet<String>,
    selected: Option<String>,
    preview: Option<String>,
    preview_truncated: bool,
    error: Option<String>,
    requests: Option<mpsc::UnboundedSender<Request>>,
    worker: Option<tokio::task::JoinHandle<()>>,
    events: Inbox<Event>,
    waker: Waker,
}

impl RemoteExplorer {
    pub fn new(waker: Waker) -> Self {
        Self {
            show_connect: false,
            transport: Transport::Ssh,
            host: String::new(),
            agent: "dotcraft-remote-agent".into(),
            root_input: String::new(),
            root: None,
            connected: false,
            connecting: false,
            cache: HashMap::new(),
            loading: HashSet::new(),
            expanded: HashSet::new(),
            selected: None,
            preview: None,
            preview_truncated: false,
            error: None,
            requests: None,
            worker: None,
            events: Inbox::new(waker.clone()),
            waker,
        }
    }

    pub fn visible(&self) -> bool {
        self.show_connect || self.connected || self.connecting
    }
    pub fn connected(&self) -> bool {
        self.connected
    }
    pub fn can_connect(&self) -> bool {
        if remote_root(&self.root_input).is_none()
            || self.host.trim().is_empty()
            || self.agent.trim().is_empty()
        {
            return false;
        }
        let endpoint = match self.transport {
            Transport::Ssh => Endpoint::Ssh {
                host: self.host.trim().into(),
                agent: self.agent.trim().into(),
            },
            Transport::Wsl => Endpoint::Wsl {
                distribution: self.host.trim().into(),
                agent: self.agent.trim().into(),
            },
        };
        endpoint.command().is_ok()
    }

    pub fn disconnect(&mut self) {
        if let Some(worker) = self.worker.take() {
            worker.abort();
        }
        self.requests = None;
        self.root = None;
        self.connected = false;
        self.connecting = false;
        self.show_connect = false;
        self.cache.clear();
        self.loading.clear();
        self.expanded.clear();
        self.selected = None;
        self.preview = None;
        self.preview_truncated = false;
        self.error = None;
        self.events = Inbox::new(self.waker.clone());
    }

    pub fn connect(&mut self) {
        let Some(root) = remote_root(&self.root_input) else {
            self.error = Some(
                tr("Enter a host or distribution, agent path, and absolute remote folder").into(),
            );
            self.show_connect = true;
            return;
        };
        let endpoint = match self.transport {
            Transport::Ssh => Endpoint::Ssh {
                host: self.host.trim().into(),
                agent: self.agent.trim().into(),
            },
            Transport::Wsl => Endpoint::Wsl {
                distribution: self.host.trim().into(),
                agent: self.agent.trim().into(),
            },
        };
        self.disconnect();
        self.root = Some(root.clone());
        self.connecting = true;
        let (tx, mut rx) = mpsc::unbounded_channel();
        self.requests = Some(tx);
        let out = self.events.sender();
        self.worker = Some(Runtime::spawn(async move {
            let mut session = match RemoteSession::connect(&endpoint).await {
                Ok(session) => session,
                Err(error) => {
                    out.send(Event::Failed(None, error.to_string()));
                    return;
                }
            };
            match list(&mut session, &root).await {
                Ok(entries) => {
                    out.send(Event::Connected(entries));
                }
                Err(error) => {
                    out.send(Event::Failed(None, error));
                    return;
                }
            }
            while let Some(request) = rx.recv().await {
                match request {
                    Request::List(path) => match list(&mut session, &path).await {
                        Ok(entries) => {
                            out.send(Event::Listed(path, entries));
                        }
                        Err(error) => {
                            out.send(Event::Failed(Some(path), error));
                        }
                    },
                    Request::Read(path) => {
                        let result = session
                            .client
                            .request(Operation::Read { path: path.clone() })
                            .await
                            .map_err(|error| error.to_string())
                            .and_then(|value| match value {
                                Value::Data { data } => {
                                    rc_remote::decode_data(&data).map_err(|error| error.to_string())
                                }
                                _ => Err(tr("Unexpected remote file response").into()),
                            })
                            .and_then(|bytes| {
                                String::from_utf8(bytes)
                                    .map_err(|_| tr("The remote file is not UTF-8 text").into())
                            });
                        match result {
                            Ok(mut text) => {
                                let truncated = text.len() > 64 * 1024;
                                if truncated {
                                    let mut end = 64 * 1024;
                                    while !text.is_char_boundary(end) {
                                        end -= 1;
                                    }
                                    text.truncate(end);
                                }
                                out.send(Event::Read(path, text, truncated));
                            }
                            Err(error) => {
                                out.send(Event::Failed(Some(path), error));
                            }
                        }
                    }
                }
            }
        }));
    }

    pub fn pump(&mut self) {
        for event in self.events.drain() {
            match event {
                Event::Connected(entries) => {
                    if let Some(root) = &self.root {
                        self.cache.insert(root.clone(), entries);
                        self.expanded.insert(root.clone());
                    }
                    self.connecting = false;
                    self.connected = true;
                    self.show_connect = false;
                    self.error = None;
                }
                Event::Listed(path, entries) => {
                    self.loading.remove(&path);
                    self.cache.insert(path, entries);
                }
                Event::Read(path, text, truncated) => {
                    self.loading.remove(&path);
                    if self.selected.as_ref() == Some(&path) {
                        self.preview = Some(text);
                        self.preview_truncated = truncated;
                    }
                }
                Event::Failed(path, error) => {
                    if let Some(path) = path {
                        self.loading.remove(&path);
                    } else {
                        self.connecting = false;
                        self.show_connect = true;
                    }
                    self.error = Some(error);
                }
            }
        }
    }

    fn list_dir(&mut self, path: &str) {
        if self.cache.contains_key(path) || !self.loading.insert(path.to_string()) {
            return;
        }
        if let Some(tx) = &self.requests {
            let _ = tx.send(Request::List(path.to_string()));
        }
    }

    fn read_file(&mut self, path: &str) {
        self.selected = Some(path.to_string());
        self.preview = None;
        self.preview_truncated = false;
        self.error = None;
        if !self.loading.insert(path.to_string()) {
            return;
        }
        if let Some(tx) = &self.requests {
            let _ = tx.send(Request::Read(path.to_string()));
        }
    }
}

async fn list(session: &mut RemoteSession, path: &str) -> Result<Vec<Entry>, String> {
    match session
        .client
        .request(Operation::List {
            path: path.to_string(),
        })
        .await
        .map_err(|error| error.to_string())?
    {
        Value::Entries { entries } => Ok(entries),
        _ => Err(tr("Unexpected remote directory response").into()),
    }
}

pub fn show(ui: &mut Ui, remote: &mut RemoteExplorer) -> Option<Action> {
    ui.horizontal(|ui| {
        ui.heading(tr("Remote Explorer"));
    });
    ui.separator();
    if remote.show_connect || !remote.connected {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut remote.transport, Transport::Ssh, "SSH");
            ui.selectable_value(&mut remote.transport, Transport::Wsl, "WSL");
        });
        let label = if remote.transport == Transport::Ssh {
            tr("SSH host")
        } else {
            tr("WSL distribution")
        };
        ui.label(label);
        ui.text_edit_singleline(&mut remote.host);
        ui.label(tr("Remote agent path"));
        ui.text_edit_singleline(&mut remote.agent);
        ui.label(tr("Remote folder"));
        ui.text_edit_singleline(&mut remote.root_input);
        if let Some(error) = &remote.error {
            ui.colored_label(rc_kit::pal(ui).error, error);
        }
        if remote.connecting {
            ui.spinner();
        }
        if ui
            .add_enabled(
                !remote.connecting && remote.can_connect(),
                egui::Button::new(tr("Connect")),
            )
            .clicked()
        {
            return Some(Action::Connect);
        }
        if ui.button(tr("Cancel")).clicked() {
            return Some(Action::Cancel);
        }
        return None;
    }
    let mut close = false;
    ui.horizontal(|ui| {
        ui.label(remote.root_input.as_str());
        if ui.button(tr("Close")).clicked() {
            close = true;
        }
    });
    if close {
        return Some(Action::Close);
    }
    if let Some(error) = &remote.error {
        ui.colored_label(rc_kit::pal(ui).error, error);
    }
    let Some(root) = remote.root.clone() else {
        return None;
    };
    egui::ScrollArea::vertical()
        .id_salt("remote_tree")
        .max_height(ui.available_height() * 0.55)
        .show(ui, |ui| {
            tree(ui, remote, &root, 0);
        });
    ui.separator();
    if let Some(path) = &remote.selected {
        ui.label(path);
    }
    if let Some(text) = &remote.preview {
        if remote.preview_truncated {
            ui.label(tr("Preview limited to the first 64 KiB"));
        }
        egui::ScrollArea::both()
            .id_salt("remote_preview")
            .show(ui, |ui| {
                ui.monospace(text);
            });
    } else if remote.selected.is_some() && remote.error.is_none() {
        ui.spinner();
    }
    None
}

fn tree(ui: &mut Ui, remote: &mut RemoteExplorer, dir: &str, depth: usize) {
    if depth > 32 {
        return;
    }
    let entries = remote.cache.get(dir).cloned().unwrap_or_default();
    for entry in entries {
        let path = join_remote(dir, &entry.name);
        if entry.is_dir {
            let open = remote.expanded.contains(&path);
            ui.horizontal(|ui| {
                ui.add_space(depth as f32 * 12.0);
                if ui
                    .selectable_label(
                        false,
                        format!("{} {}", if open { "▾" } else { "▸" }, entry.name),
                    )
                    .clicked()
                {
                    if open {
                        remote.expanded.remove(&path);
                    } else {
                        remote.expanded.insert(path.clone());
                        remote.list_dir(&path);
                    }
                }
            });
            if remote.expanded.contains(&path) {
                tree(ui, remote, &path, depth + 1);
            }
        } else {
            ui.horizontal(|ui| {
                ui.add_space((depth + 1) as f32 * 12.0);
                if ui
                    .selectable_label(
                        remote.selected.as_deref() == Some(path.as_str()),
                        &entry.name,
                    )
                    .clicked()
                {
                    remote.read_file(&path);
                }
            });
        }
    }
    if remote.loading.contains(dir) {
        ui.spinner();
    }
}

fn remote_root(input: &str) -> Option<String> {
    let input = input.trim();
    if !input.starts_with('/') || input.contains('\0') {
        return None;
    }
    let normalized = input.trim_end_matches('/');
    Some(if normalized.is_empty() {
        "/".into()
    } else {
        normalized.into()
    })
}

fn join_remote(parent: &str, name: &str) -> String {
    if parent == "/" {
        format!("/{name}")
    } else {
        format!("{parent}/{name}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_paths_keep_posix_separators() {
        assert_eq!(remote_root(" /home/dev/ "), Some("/home/dev".into()));
        assert_eq!(join_remote("/", "repo"), "/repo");
        assert_eq!(
            join_remote("/home/dev", "App.csproj"),
            "/home/dev/App.csproj"
        );
        assert_eq!(remote_root("C:\\repo"), None);
    }
}
