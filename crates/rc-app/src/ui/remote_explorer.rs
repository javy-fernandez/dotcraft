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
    Command(
        CommandKind,
        Option<String>,
        tokio::sync::oneshot::Receiver<()>,
    ),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CommandKind {
    Info,
    Sdks,
    Build,
    Test,
}
struct CommandResult {
    code: Option<i32>,
    stdout: String,
    output: String,
}
enum Event {
    Connected(Vec<Entry>),
    Listed(String, Vec<Entry>),
    Read(String, String, bool),
    Failed(Option<String>, String),
    CommandDone(CommandKind, Result<CommandResult, String>),
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
    pub dotnet_program: String,
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
    running: Option<CommandKind>,
    cancel_command: Option<tokio::sync::oneshot::Sender<()>>,
    sdk_versions: Vec<String>,
    command_output: Option<String>,
    command_error: bool,
    show_output: bool,
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
            dotnet_program: "dotnet".into(),
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
            running: None,
            cancel_command: None,
            sdk_versions: Vec::new(),
            command_output: None,
            command_error: false,
            show_output: false,
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
            || self.dotnet_program.trim().is_empty()
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
        self.running = None;
        self.cancel_command = None;
        self.sdk_versions.clear();
        self.command_output = None;
        self.command_error = false;
        self.show_output = false;
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
        let dotnet_program = self.dotnet_program.trim().to_string();
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
                    Request::Command(kind, target, cancel) => {
                        let result = remote_command(
                            &mut session,
                            &root,
                            &dotnet_program,
                            kind,
                            target,
                            cancel,
                        )
                        .await;
                        out.send(Event::CommandDone(kind, result));
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
                    self.start_command(CommandKind::Sdks);
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
                Event::CommandDone(kind, result) => {
                    self.running = None;
                    self.cancel_command = None;
                    match result {
                        Ok(result) => {
                            if kind == CommandKind::Sdks && result.code == Some(0) {
                                self.sdk_versions = result
                                    .stdout
                                    .lines()
                                    .map(str::trim)
                                    .filter(|line| !line.is_empty())
                                    .map(str::to_string)
                                    .collect();
                            }
                            self.command_error = result.code != Some(0);
                            self.command_output = Some(result.output);
                        }
                        Err(error) => {
                            self.command_error = true;
                            self.command_output = Some(if error == "remote request cancelled" {
                                tr("Remote command cancelled").into()
                            } else {
                                error
                            });
                        }
                    }
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
        self.show_output = false;
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

    fn start_command(&mut self, kind: CommandKind) {
        if !self.connected || self.running.is_some() {
            return;
        }
        let target = self
            .selected
            .as_deref()
            .filter(|path| is_dotnet_target(path))
            .map(str::to_string);
        let (cancel, receiver) = tokio::sync::oneshot::channel();
        let Some(requests) = &self.requests else {
            return;
        };
        if requests
            .send(Request::Command(kind, target, receiver))
            .is_ok()
        {
            if kind == CommandKind::Sdks {
                self.sdk_versions.clear();
            }
            self.running = Some(kind);
            self.cancel_command = Some(cancel);
            self.command_output = None;
            self.command_error = false;
            self.show_output = true;
        }
    }

    fn cancel_running(&mut self) {
        if let Some(cancel) = self.cancel_command.take() {
            let _ = cancel.send(());
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

async fn remote_command(
    session: &mut RemoteSession,
    root: &str,
    dotnet_program: &str,
    kind: CommandKind,
    target: Option<String>,
    cancel: tokio::sync::oneshot::Receiver<()>,
) -> Result<CommandResult, String> {
    let args = dotnet_args(kind, target);
    let timeout_ms = if matches!(kind, CommandKind::Build | CommandKind::Test) {
        120_000
    } else {
        15_000
    };
    let value = session
        .client
        .request_cancellable(
            Operation::Run {
                program: dotnet_program.into(),
                args,
                cwd: root.to_string(),
                env: vec![
                    ("DOTNET_CLI_UI_LANGUAGE".into(), "en".into()),
                    ("DOTNET_NOLOGO".into(), "1".into()),
                    ("DOTNET_CLI_TELEMETRY_OPTOUT".into(), "1".into()),
                ],
                timeout_ms,
            },
            cancel,
        )
        .await
        .map_err(|error| error.to_string())?;
    let Value::Exit {
        code,
        stdout,
        stderr,
    } = value
    else {
        return Err(tr("Unexpected remote process response").into());
    };
    let stdout = rc_remote::decode_data(&stdout).map_err(|error| error.to_string())?;
    let stderr = rc_remote::decode_data(&stderr).map_err(|error| error.to_string())?;
    let stdout = String::from_utf8_lossy(&stdout).into_owned();
    let mut output = stdout.clone();
    if !stderr.is_empty() {
        if !output.is_empty() {
            output.push('\n');
        }
        output.push_str(&String::from_utf8_lossy(&stderr));
    }
    if output.is_empty() {
        output = format!(
            "{}: {}",
            tr("Exit code"),
            code.map_or_else(|| "?".into(), |code| code.to_string())
        );
    }
    Ok(CommandResult {
        code,
        stdout,
        output,
    })
}

fn dotnet_args(kind: CommandKind, target: Option<String>) -> Vec<String> {
    let mut args = match kind {
        CommandKind::Info => vec!["--info".to_string()],
        CommandKind::Sdks => vec!["--list-sdks".to_string()],
        CommandKind::Build => vec!["build".to_string()],
        CommandKind::Test => vec!["test".to_string()],
    };
    if matches!(kind, CommandKind::Build | CommandKind::Test) {
        if let Some(target) = target {
            args.push(target);
        }
    }
    args
}

fn is_dotnet_target(path: &str) -> bool {
    [".sln", ".slnx", ".slnf", ".csproj", ".fsproj", ".vbproj"]
        .iter()
        .any(|extension| path.to_ascii_lowercase().ends_with(extension))
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
        ui.label(tr("Remote dotnet path"));
        ui.text_edit_singleline(&mut remote.dotnet_program);
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
    let busy = remote.running.is_some();
    ui.horizontal_wrapped(|ui| {
        if ui
            .add_enabled(!busy, egui::Button::new(tr("SDK Info")))
            .clicked()
        {
            remote.start_command(CommandKind::Info);
        }
        if ui
            .add_enabled(!busy, egui::Button::new(tr("Refresh SDKs")))
            .clicked()
        {
            remote.start_command(CommandKind::Sdks);
        }
        if ui
            .add_enabled(!busy, egui::Button::new(tr("Build")))
            .clicked()
        {
            remote.start_command(CommandKind::Build);
        }
        if ui
            .add_enabled(!busy, egui::Button::new(tr("Test")))
            .clicked()
        {
            remote.start_command(CommandKind::Test);
        }
        if busy {
            ui.spinner();
            if ui
                .add_enabled(
                    remote.cancel_command.is_some(),
                    egui::Button::new(tr("Cancel")),
                )
                .clicked()
            {
                remote.cancel_running();
            }
        }
    });
    if !remote.sdk_versions.is_empty() {
        ui.collapsing(tr("Installed SDKs"), |ui| {
            for sdk in &remote.sdk_versions {
                ui.monospace(sdk);
            }
        });
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
    ui.horizontal(|ui| {
        ui.selectable_value(&mut remote.show_output, false, tr("Preview"));
        ui.selectable_value(&mut remote.show_output, true, tr(".NET Output"));
    });
    if remote.show_output {
        if let Some(output) = &remote.command_output {
            egui::ScrollArea::both()
                .id_salt("remote_command_output")
                .show(ui, |ui| {
                    ui.colored_label(
                        if remote.command_error {
                            rc_kit::pal(ui).error
                        } else {
                            rc_kit::pal(ui).text
                        },
                        egui::RichText::new(output).monospace(),
                    );
                });
        }
    } else {
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

    #[test]
    fn build_and_test_use_selected_project_without_a_shell() {
        assert!(is_dotnet_target("/repo/tests/App.Tests.csproj"));
        assert_eq!(
            dotnet_args(CommandKind::Build, Some("/repo/App.sln".into())),
            ["build", "/repo/App.sln"]
        );
        assert_eq!(
            dotnet_args(
                CommandKind::Test,
                Some("/repo/tests/App.Tests.csproj".into())
            ),
            ["test", "/repo/tests/App.Tests.csproj"]
        );
        assert_eq!(dotnet_args(CommandKind::Sdks, None), ["--list-sdks"]);
    }
}
