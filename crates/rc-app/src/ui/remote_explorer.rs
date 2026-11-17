//! File › Open Remote Folder…: a folder on another machine (over SSH) or in a WSL
//! distribution, served by `dotcraft-remote-agent`. The Explorer shows its tree and the
//! editor area previews the selected file (read-only for now). Every request runs on one
//! worker task, never on the UI thread; closing the folder kills the connection, and the
//! agent then stops whatever it was running.

use crate::app::DotCraftApp;
use crate::toolwindow::ToolWindow;
use crate::ui::{dialogs, kit};
use egui::{Align2, Context, FontId, Rect, Sense, Ui, pos2, vec2};
use rc_core::settings::RemoteConnection;
use rc_core::{Inbox, Runtime, Waker, inbox::Outbox};
use rc_kit::i18n::{tr, trf};
use rc_kit::{Ico, ico, icons, theme};
use rc_remote::{Client, Endpoint, Entry, Operation, RemoteSession, Value, is_connection_error};
use std::collections::{HashMap, HashSet};
use tokio::sync::{mpsc, oneshot};

/// Where `cargo install` puts the agent, the default of the connection dialog.
pub const DEFAULT_AGENT: &str = "~/.cargo/bin/dotcraft-remote-agent";
/// How to get the agent onto the remote machine (needs a Rust toolchain there).
const INSTALL_COMMAND: &str = "cargo install --locked --git https://github.com/javy-fernandez/dotcraft rc-remote";
/// Largest preview shown in the editor area; the agent reads files up to 1 MiB.
const PREVIEW_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Status {
    Idle,
    Connecting,
    Connected,
    /// The connection dropped; the tree stays until Reconnect or Close.
    Lost,
}

enum Request {
    List(String),
    Read(String),
    /// A `dotnet` command in the folder, with a project or solution as target; firing the
    /// sender cancels it.
    Command(CommandKind, Option<String>, oneshot::Receiver<()>),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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

/// The last `dotnet` command: its command line and how it ended.
struct CommandOutput {
    command: String,
    /// `Some(Ok(None))` once cancelled.
    result: Option<Result<Option<CommandResult>, String>>,
}

enum Event {
    Connected(String, Vec<Entry>),
    Listed(String, Vec<Entry>),
    ListFailed(String, String),
    Read(String, Result<(String, bool), String>),
    CommandDone(CommandKind, Result<Option<CommandResult>, String>),
    /// The connection could not be made, or broke.
    Lost(String),
}

/// The file shown in the editor area: its text (and whether it was cut at
/// [`PREVIEW_BYTES`]) or why it cannot be shown; `None` while it loads.
pub struct Preview {
    pub path: String,
    pub body: Option<Result<(String, bool), String>>,
}

pub struct RemoteExplorer {
    /// The connection dialog's fields.
    pub form: RemoteConnection,
    form_error: Option<String>,
    /// The connection in use (the form as it was when Connect was pressed).
    connection: Option<RemoteConnection>,
    /// The open remote folder, once its first listing arrived.
    root: Option<String>,
    status: Status,
    error: Option<String>,
    cache: HashMap<String, Vec<Entry>>,
    loading: HashSet<String>,
    expanded: HashSet<String>,
    preview: Option<Preview>,
    running: Option<CommandKind>,
    cancel_command: Option<oneshot::Sender<()>>,
    sdk_versions: Option<Vec<String>>,
    output: Option<CommandOutput>,
    /// The editor area shows the command output instead of the preview.
    show_output: bool,
    requests: Option<mpsc::UnboundedSender<Request>>,
    worker: Option<tokio::task::JoinHandle<()>>,
    events: Inbox<Event>,
    waker: Waker,
}

impl RemoteExplorer {
    pub fn new(waker: Waker, mut form: RemoteConnection) -> Self {
        if form.agent.trim().is_empty() {
            form.agent = DEFAULT_AGENT.into();
        }
        if form.dotnet.trim().is_empty() {
            form.dotnet = "dotnet".into();
        }
        Self {
            form,
            form_error: None,
            connection: None,
            root: None,
            status: Status::Idle,
            error: None,
            cache: HashMap::new(),
            loading: HashSet::new(),
            expanded: HashSet::new(),
            preview: None,
            running: None,
            cancel_command: None,
            sdk_versions: None,
            output: None,
            show_output: false,
            requests: None,
            worker: None,
            events: Inbox::new(waker.clone()),
            waker,
        }
    }

    /// Whether a remote folder is open (even while its connection is lost).
    pub fn is_open(&self) -> bool {
        self.root.is_some()
    }

    /// The connection that opened the current folder, to remember it.
    pub fn connection(&self) -> Option<&RemoteConnection> {
        self.connection.as_ref()
    }

    fn connecting_from_dialog(&self) -> bool {
        self.status == Status::Connecting && self.root.is_none()
    }

    /// Closes the remote folder: aborting the worker drops the session, which kills ssh or
    /// wsl.exe; the agent sees its input close and kills the processes it started.
    pub fn disconnect(&mut self) {
        self.stop_worker();
        self.connection = None;
        self.root = None;
        self.status = Status::Idle;
        self.error = None;
        self.cache.clear();
        self.loading.clear();
        self.expanded.clear();
        self.preview = None;
        self.running = None;
        self.cancel_command = None;
        self.sdk_versions = None;
        self.output = None;
        self.show_output = false;
    }

    fn stop_worker(&mut self) {
        if let Some(worker) = self.worker.take() {
            worker.abort();
        }
        self.requests = None;
        // Whatever the old worker still had to say is about a connection that is gone.
        self.events = Inbox::new(self.waker.clone());
    }

    /// Connects with the dialog's fields; the folder opens when its first listing arrives.
    pub fn connect(&mut self) {
        let Some(folder) = remote_root(&self.form.folder) else {
            self.form_error = Some(tr("Enter an absolute remote folder, like /home/me/src/App").into());
            return;
        };
        let Some(endpoint) = endpoint(&self.form).filter(|_| !self.form.dotnet.trim().is_empty()) else {
            self.form_error = Some(tr("Enter a valid host or distribution and agent path").into());
            return;
        };
        self.disconnect();
        let mut connection = self.form.clone();
        connection.folder = folder.clone();
        self.connection = Some(connection);
        self.start(endpoint, folder);
    }

    /// Connects again after the connection dropped, keeping the expanded folders.
    pub fn reconnect(&mut self) {
        let (Some(connection), Some(root)) = (&self.connection, &self.root) else { return };
        let Some(endpoint) = endpoint(connection) else { return };
        let root = root.clone();
        self.cache.clear();
        self.loading.clear();
        self.running = None;
        self.cancel_command = None;
        self.start(endpoint, root);
    }

    /// Cancel in the dialog: stops a connection that has not opened a folder yet.
    pub fn cancel_connect(&mut self) {
        if self.root.is_none() {
            self.disconnect();
        }
        self.form_error = None;
    }

    fn start(&mut self, endpoint: Endpoint, folder: String) {
        self.stop_worker();
        self.status = Status::Connecting;
        self.form_error = None;
        self.error = None;
        let (tx, rx) = mpsc::unbounded_channel();
        self.requests = Some(tx);
        let connection = self.connection.clone().unwrap_or_default();
        self.worker = Some(Runtime::spawn(serve(endpoint, connection, folder, rx, self.events.sender())));
    }

    /// Lists every expanded folder again.
    pub fn refresh(&mut self) {
        if self.status == Status::Connected {
            self.cache.clear();
            self.error = None;
        }
    }

    fn collapse_all(&mut self) {
        self.expanded.retain(|path| Some(path) == self.root.as_ref());
    }

    /// Applies what the worker sent. Returns true when a remote folder has just opened.
    pub fn pump(&mut self) -> bool {
        let mut opened = false;
        for event in self.events.drain() {
            match event {
                Event::Connected(folder, entries) => {
                    opened = self.root.is_none();
                    self.status = Status::Connected;
                    self.expanded.insert(folder.clone());
                    self.cache.insert(folder.clone(), sorted(entries));
                    self.root = Some(folder);
                    self.start_command(CommandKind::Sdks);
                }
                Event::Listed(path, entries) => {
                    self.loading.remove(&path);
                    self.cache.insert(path, sorted(entries));
                }
                Event::ListFailed(path, error) => {
                    self.loading.remove(&path);
                    self.expanded.remove(&path);
                    self.error = Some(error);
                }
                Event::Read(path, body) => {
                    if let Some(preview) = self.preview.as_mut().filter(|p| p.path == path) {
                        preview.body = Some(body);
                    }
                }
                Event::CommandDone(kind, result) => {
                    self.running = None;
                    self.cancel_command = None;
                    if kind == CommandKind::Sdks {
                        self.sdk_versions = Some(match &result {
                            Ok(Some(r)) if r.code == Some(0) => sdk_versions(&r.stdout),
                            _ => Vec::new(),
                        });
                    } else if let Some(output) = &mut self.output {
                        output.result = Some(result);
                    }
                }
                Event::Lost(error) => {
                    self.requests = None;
                    self.worker = None;
                    self.loading.clear();
                    self.running = None;
                    self.cancel_command = None;
                    // A command in flight died with the connection.
                    if let Some(output) = self.output.as_mut().filter(|o| o.result.is_none()) {
                        output.result = Some(Err(tr("Connection lost").into()));
                    }
                    if let Some(preview) = self.preview.as_mut().filter(|p| p.body.is_none()) {
                        preview.body = Some(Err(tr("Connection lost").into()));
                    }
                    if self.root.is_some() {
                        self.status = Status::Lost;
                        self.error = Some(error);
                    } else {
                        self.disconnect();
                        self.form_error = Some(error);
                    }
                }
            }
        }
        opened
    }

    fn send(&mut self, request: Request) -> bool {
        self.status == Status::Connected && self.requests.as_ref().is_some_and(|tx| tx.send(request).is_ok())
    }

    /// Asks for the folders that are expanded but not listed yet (after a reconnect or a
    /// refresh too).
    fn list_missing(&mut self, dir: &str) {
        if !self.cache.contains_key(dir) && !self.loading.contains(dir) && self.send(Request::List(dir.to_string())) {
            self.loading.insert(dir.to_string());
        }
    }

    fn select_file(&mut self, path: &str) {
        if self.preview.as_ref().is_some_and(|p| p.path == path && p.body.is_some()) {
            return;
        }
        self.preview = Some(Preview { path: path.to_string(), body: None });
        if !self.send(Request::Read(path.to_string()))
            && let Some(preview) = &mut self.preview
        {
            preview.body = Some(Err(tr("Not connected").into()));
        }
    }

    /// Runs a `dotnet` command in the folder: on the selected project or solution, or on
    /// whatever `dotnet` finds in the folder.
    fn start_command(&mut self, kind: CommandKind) {
        if self.running.is_some() {
            return;
        }
        let target = self.preview.as_ref().map(|p| p.path.clone()).filter(|path| is_dotnet_target(path));
        let (cancel, receiver) = oneshot::channel();
        let dotnet = self.connection.as_ref().map(|c| c.dotnet.trim().to_string()).unwrap_or_default();
        let command = std::iter::once(dotnet).chain(dotnet_args(kind, target.clone())).collect::<Vec<_>>().join(" ");
        if self.send(Request::Command(kind, target, receiver)) {
            self.running = Some(kind);
            self.cancel_command = Some(cancel);
            if kind != CommandKind::Sdks {
                self.output = Some(CommandOutput { command, result: None });
                self.show_output = true;
            }
        }
    }

    fn cancel_running(&mut self) {
        if let Some(cancel) = self.cancel_command.take() {
            let _ = cancel.send(());
        }
    }

    /// `host:/folder`, for headers.
    fn location(&self) -> String {
        match (&self.connection, &self.root) {
            (Some(c), Some(root)) => format!("{}:{root}", c.host.trim()),
            _ => String::new(),
        }
    }
}

fn endpoint(c: &RemoteConnection) -> Option<Endpoint> {
    let (host, agent) = (c.host.trim().to_string(), c.agent.trim().to_string());
    let endpoint = if c.wsl { Endpoint::Wsl { distribution: host, agent } } else { Endpoint::Ssh { host, agent } };
    endpoint.command().ok().map(|_| endpoint)
}

/// Folders first, then by name, like the local Explorer.
fn sorted(mut entries: Vec<Entry>) -> Vec<Entry> {
    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    entries
}

/// The worker: one connection; requests run side by side (browse while a build runs).
/// Aborting it drops the session and every request in flight.
async fn serve(endpoint: Endpoint, connection: RemoteConnection, root: String, mut requests: mpsc::UnboundedReceiver<Request>, out: Outbox<Event>) {
    let session = match RemoteSession::connect(&endpoint).await {
        Ok(session) => session,
        Err(error) => {
            out.send(Event::Lost(error.to_string()));
            return;
        }
    };
    let client = session.client.clone();
    match list(&client, &root).await {
        Ok(entries) => out.send(Event::Connected(root.clone(), entries)),
        Err(error) => {
            out.send(Event::Lost(describe(&session, &error)));
            return;
        }
    };
    let dotnet = connection.dotnet.trim().to_string();
    let mut tasks = tokio::task::JoinSet::new();
    loop {
        tokio::select! {
            request = requests.recv() => {
                let Some(request) = request else { return };
                let (client, root, dotnet) = (client.clone(), root.clone(), dotnet.clone());
                tasks.spawn(async move { handle(&client, &root, &dotnet, request).await });
            }
            Some(Ok((event, error))) = tasks.join_next() => {
                // A broken connection ends the worker; the agent kills what it was running.
                if let Some(error) = error.filter(is_connection_error) {
                    out.send(Event::Lost(describe(&session, &error)));
                    return;
                }
                out.send(event);
            }
        }
    }
}

/// One request: the event for the UI, and the error behind a failure.
async fn handle(client: &Client, root: &str, dotnet: &str, request: Request) -> (Event, Option<anyhow::Error>) {
    match request {
        Request::List(path) => match list(client, &path).await {
            Ok(entries) => (Event::Listed(path, entries), None),
            Err(error) => (Event::ListFailed(path, error.to_string()), Some(error)),
        },
        Request::Read(path) => match read(client, &path).await {
            Ok(body) => (Event::Read(path, body), None),
            Err(error) => (Event::Read(path, Err(error.to_string())), Some(error)),
        },
        Request::Command(kind, target, cancel) => match remote_command(client, root, dotnet, kind, target, cancel).await {
            Ok(result) => (Event::CommandDone(kind, Ok(result)), None),
            Err(error) => (Event::CommandDone(kind, Err(error.to_string())), Some(error)),
        },
    }
}

/// A connection error with what ssh said about it.
fn describe(session: &RemoteSession, error: &anyhow::Error) -> String {
    let tail = session.stderr_tail();
    if tail.is_empty() || !is_connection_error(error) { error.to_string() } else { format!("{error}\n{tail}") }
}

async fn list(client: &Client, path: &str) -> anyhow::Result<Vec<Entry>> {
    match client.request(Operation::List { path: path.to_string() }).await? {
        Value::Entries { entries } => Ok(entries),
        _ => anyhow::bail!("unexpected remote directory response"),
    }
}

/// The preview of a file: its text, cut at [`PREVIEW_BYTES`], or why it has none.
async fn read(client: &Client, path: &str) -> anyhow::Result<Result<(String, bool), String>> {
    let Value::Data { data } = client.request(Operation::Read { path: path.to_string() }).await? else {
        anyhow::bail!("unexpected remote file response");
    };
    let bytes = rc_remote::decode_data(&data)?;
    Ok(preview_text(bytes))
}

/// Runs `dotnet`; `None` when it was cancelled.
async fn remote_command(
    client: &Client,
    root: &str,
    dotnet: &str,
    kind: CommandKind,
    target: Option<String>,
    cancel: oneshot::Receiver<()>,
) -> anyhow::Result<Option<CommandResult>> {
    // Builds and tests can take long (a first restore, a big test suite); Stop ends them.
    let timeout_ms = if matches!(kind, CommandKind::Build | CommandKind::Test) { 30 * 60 * 1000 } else { 60_000 };
    let Some(value) = client
        .request_cancellable(
            Operation::Run {
                program: dotnet.into(),
                args: dotnet_args(kind, target),
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
        .await?
    else {
        return Ok(None);
    };
    let Value::Exit { code, stdout, stderr } = value else {
        anyhow::bail!("unexpected remote process response");
    };
    let stdout = String::from_utf8_lossy(&rc_remote::decode_data(&stdout)?).into_owned();
    let stderr = String::from_utf8_lossy(&rc_remote::decode_data(&stderr)?).into_owned();
    let mut output = stdout.clone();
    if !stderr.is_empty() {
        if !output.is_empty() && !output.ends_with('\n') {
            output.push('\n');
        }
        output.push_str(&stderr);
    }
    Ok(Some(CommandResult { code, stdout, output }))
}

/// Arguments only, never a shell line: the agent starts `dotnet` directly.
fn dotnet_args(kind: CommandKind, target: Option<String>) -> Vec<String> {
    let mut args = match kind {
        CommandKind::Info => vec!["--info".to_string()],
        CommandKind::Sdks => vec!["--list-sdks".to_string()],
        CommandKind::Build => vec!["build".to_string()],
        CommandKind::Test => vec!["test".to_string()],
    };
    if matches!(kind, CommandKind::Build | CommandKind::Test)
        && let Some(target) = target
    {
        args.push(target);
    }
    args
}

fn is_dotnet_target(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    [".sln", ".slnx", ".slnf", ".csproj", ".fsproj", ".vbproj"].iter().any(|ext| lower.ends_with(ext))
}

/// The versions in `dotnet --list-sdks` output (`8.0.428 [/usr/share/dotnet/sdk]`).
fn sdk_versions(stdout: &str) -> Vec<String> {
    stdout.lines().filter_map(|line| line.split_whitespace().next()).map(str::to_string).collect()
}

fn preview_text(bytes: Vec<u8>) -> Result<(String, bool), String> {
    let mut text = String::from_utf8(bytes).map_err(|_| tr("Binary file: no preview").to_string())?;
    let truncated = text.len() > PREVIEW_BYTES;
    if truncated {
        let mut end = PREVIEW_BYTES;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    Ok((text, truncated))
}

/// An absolute POSIX folder without trailing slashes; remote paths never use the host's
/// separators, even on Windows.
fn remote_root(input: &str) -> Option<String> {
    let input = input.trim();
    if !input.starts_with('/') || input.chars().any(char::is_control) {
        return None;
    }
    let normalized = input.trim_end_matches('/');
    Some(if normalized.is_empty() { "/".into() } else { normalized.into() })
}

fn join_remote(parent: &str, name: &str) -> String {
    if parent == "/" { format!("/{name}") } else { format!("{parent}/{name}") }
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').find(|part| !part.is_empty()).unwrap_or("/")
}

// ------------------------------------------------------------------ UI

/// File › Open Remote Folder…. Returns true to close the dialog (it also closes when the
/// folder opens, see `DotCraftApp::remote_folder_opened`).
pub fn connect_dialog(app: &mut DotCraftApp, ctx: &Context) -> bool {
    let remote = &mut app.remote;
    let connecting = remote.connecting_from_dialog();
    let valid = remote_root(&remote.form.folder).is_some() && endpoint(&remote.form).is_some() && !remote.form.dotnet.trim().is_empty();
    let width = (ctx.content_rect().width() - 48.0).clamp(380.0, 620.0);
    let field_width = (width - 170.0).max(180.0);
    let (mut connect, mut cancel) = (false, false);
    let ((), escape) = dialogs::modal(ctx, "remote_connect", width, |ui| {
        dialogs::title(ui, tr("Open Remote Folder"));
        let transports = vec!["SSH".to_string(), "WSL".to_string()];
        egui::Grid::new("remote_fields").num_columns(2).spacing([12.0, 10.0]).show(ui, |ui| {
            ui.label(tr("Connection"));
            if let Some(i) = kit::dropdown(ui, "remote_transport", &transports[remote.form.wsl as usize], &transports, !connecting) {
                remote.form.wsl = i == 1;
            }
            ui.end_row();
            let (label, hint) = if remote.form.wsl { (tr("WSL distribution"), "Ubuntu") } else { (tr("SSH host"), "user@example.com") };
            ui.label(label);
            ui.add_enabled(!connecting, egui::TextEdit::singleline(&mut remote.form.host).hint_text(hint).desired_width(field_width));
            ui.end_row();
            ui.label(tr("Agent path"));
            ui.add_enabled(!connecting, egui::TextEdit::singleline(&mut remote.form.agent).hint_text(DEFAULT_AGENT).desired_width(field_width));
            ui.end_row();
            ui.label(tr("Remote folder"));
            ui.add_enabled(!connecting, egui::TextEdit::singleline(&mut remote.form.folder).hint_text("/home/me/src/App").desired_width(field_width));
            ui.end_row();
            ui.label(tr(".NET path"));
            ui.add_enabled(!connecting, egui::TextEdit::singleline(&mut remote.form.dotnet).hint_text("dotnet").desired_width(field_width));
            ui.end_row();
        });
        ui.add_space(10.0);
        ui.set_max_width(width);
        kit::note(
            ui,
            if remote.form.wsl {
                tr("DotCraft starts the agent inside the distribution with wsl.exe.")
            } else {
                tr("DotCraft starts the agent with your ssh client and its configuration: keys, ssh-agent and known_hosts. It cannot answer password or new host key prompts, so connect once from a terminal first.")
            },
        );
        ui.add_space(4.0);
        kit::note(ui, tr("Install the agent on that machine with:"));
        ui.label(egui::RichText::new(INSTALL_COMMAND).monospace().size(11.5));
        if let Some(error) = &remote.form_error {
            ui.add_space(6.0);
            kit::message(ui, error, true);
        }
        ui.add_space(14.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            connect = ui.add_enabled_ui(valid && !connecting, |ui| theme::primary_button(ui, tr("Connect"))).inner.clicked();
            cancel = theme::secondary_button(ui, tr("Cancel")).clicked();
            if connecting {
                ui.spinner();
                kit::note(ui, tr("Connecting…"));
            }
        });
    });
    if connect {
        remote.connect();
    }
    if cancel || escape {
        remote.cancel_connect();
        return true;
    }
    false
}

/// The Explorer tool window while a remote folder is open.
pub fn explorer(app: &mut DotCraftApp, ui: &mut Ui) {
    let remote = &mut app.remote;
    let connected = remote.status == Status::Connected;
    let click = kit::header(
        ui,
        tr("Explorer"),
        &[],
        "",
        &[
            (ico::HIDE, tr("Hide"), true),
            (ico::CLOSE, tr("Close Remote Folder"), true),
            (ico::REFRESH, tr("Refresh"), connected),
            (ico::COLLAPSE_ALL, tr("Collapse All"), true),
        ],
    );
    match click.action {
        Some(0) => {
            app.toggle_tool(ToolWindow::Explorer);
            return;
        }
        Some(1) => {
            app.close_project();
            return;
        }
        Some(2) => {
            remote.refresh();
            remote.start_command(CommandKind::Sdks);
        }
        Some(3) => remote.collapse_all(),
        _ => {}
    }
    if connected {
        let busy = remote.running.is_some();
        kit::toolbar(ui, |ui| {
            if kit::button(ui, Some(ico::BUILD), tr("Build"), !busy).clicked() {
                remote.start_command(CommandKind::Build);
            }
            if kit::button(ui, Some(ico::RUN_ALL), tr("Test"), !busy).clicked() {
                remote.start_command(CommandKind::Test);
            }
            if kit::button(ui, None, tr("SDK Info"), !busy).clicked() {
                remote.start_command(CommandKind::Info);
            }
            if busy && remote.running != Some(CommandKind::Sdks) && kit::button(ui, Some(ico::STOP), tr("Stop"), remote.cancel_command.is_some()).clicked() {
                remote.cancel_running();
            }
            if busy {
                ui.spinner();
            }
        });
        match &remote.sdk_versions {
            Some(versions) if versions.is_empty() => kit::content(ui, |ui| kit::message(ui, tr("No .NET SDK found on the remote machine"), false)),
            Some(versions) => kit::content(ui, |ui| kit::note(ui, &format!("{} {}", tr(".NET SDKs:"), versions.join(", ")))),
            None => {}
        }
    }
    match remote.status {
        Status::Lost => {
            kit::toolbar(ui, |ui| {
                kit::message(ui, tr("Connection lost"), true);
                if kit::button(ui, Some(ico::REFRESH), tr("Reconnect"), true).clicked() {
                    remote.reconnect();
                }
            });
        }
        Status::Connecting => {
            kit::toolbar(ui, |ui| {
                ui.spinner();
                kit::note(ui, tr("Connecting…"));
            });
        }
        Status::Idle | Status::Connected => {}
    }
    if let Some(error) = remote.error.clone() {
        kit::content(ui, |ui| kit::message(ui, &error, true));
    }
    let Some(root) = remote.root.clone() else { return };
    let location = remote.location();
    egui::ScrollArea::both().id_salt("remote_tree").auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let open = remote.expanded.contains(&root);
        if tree_row(ui, 0, Some(open), ico::FOLDER, file_name(&root), &location, false).clicked() {
            toggle(remote, &root);
        }
        if open {
            tree(ui, remote, &root, 1);
        }
    });
}

fn toggle(remote: &mut RemoteExplorer, dir: &str) {
    if !remote.expanded.remove(dir) {
        remote.expanded.insert(dir.to_string());
    }
}

fn tree(ui: &mut Ui, remote: &mut RemoteExplorer, dir: &str, depth: usize) {
    remote.list_missing(dir);
    let Some(entries) = remote.cache.get(dir).cloned() else {
        if remote.loading.contains(dir) {
            ui.horizontal(|ui| {
                ui.add_space(12.0 + depth as f32 * 18.0 + 26.0);
                ui.spinner();
            });
        }
        return;
    };
    for entry in entries {
        let path = join_remote(dir, &entry.name);
        if entry.is_dir {
            let open = remote.expanded.contains(&path);
            if tree_row(ui, depth, Some(open), ico::FOLDER, &entry.name, "", false).clicked() {
                toggle(remote, &path);
            }
            if open && depth < 64 {
                tree(ui, remote, &path, depth + 1);
            }
        } else {
            let selected = remote.preview.as_ref().is_some_and(|p| p.path == path);
            if tree_row(ui, depth, None, file_icon(&entry.name), &entry.name, "", selected).clicked() {
                remote.select_file(&path);
            }
        }
    }
}

fn file_icon(name: &str) -> Ico {
    let lower = name.to_ascii_lowercase();
    if [".sln", ".slnx", ".slnf"].iter().any(|ext| lower.ends_with(ext)) {
        ico::SOLUTION
    } else if [".csproj", ".fsproj", ".vbproj"].iter().any(|ext| lower.ends_with(ext)) {
        ico::PROJECT
    } else {
        icons::for_file(name)
    }
}

/// An Explorer row: chevron for folders (`open`), icon, name, grey secondary text.
fn tree_row(ui: &mut Ui, depth: usize, open: Option<bool>, icon: Ico, text: &str, secondary: &str, selected: bool) -> egui::Response {
    let p = rc_kit::pal(ui);
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width().max(240.0), 24.0), Sense::click());
    if selected {
        ui.painter().rect_filled(r.shrink2(vec2(6.0, 0.0)), 4.0, p.selection);
    } else if resp.hovered() {
        ui.painter().rect_filled(r.shrink2(vec2(6.0, 0.0)), 4.0, p.row_hover);
    }
    let x = r.left() + 12.0 + depth as f32 * 18.0;
    if let Some(open) = open {
        let chevron = if open { ico::CHEVRON_DOWN } else { ico::CHEVRON_RIGHT };
        icons::paint_sized(ui.painter(), Rect::from_center_size(pos2(x + 7.0, r.center().y), vec2(14.0, 14.0)), chevron, &p, true, 14.0);
    }
    icons::paint(ui.painter(), Rect::from_center_size(pos2(x + 26.0, r.center().y), vec2(16.0, 16.0)), icon, &p, true);
    let g = ui.painter().layout_no_wrap(text.to_string(), FontId::proportional(13.0), p.text);
    ui.painter().galley(pos2(x + 40.0, (r.center().y - g.size().y / 2.0).round()), g.clone(), p.text);
    if !secondary.is_empty() {
        ui.painter().text(pos2(x + 48.0 + g.size().x, r.center().y), Align2::LEFT_CENTER, secondary, FontId::proportional(12.0), p.muted);
    }
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, text));
    resp
}

/// The editor area while a remote folder is open and no local file is: the selected
/// remote file (read-only) or the output of the last `dotnet` command.
pub fn editor(app: &mut DotCraftApp, ui: &mut Ui) {
    let font_size = app.settings.editor_font_size;
    let remote = &mut app.remote;
    if remote.output.is_some() {
        let preview_name = remote.preview.as_ref().map_or(tr("Preview"), |p| file_name(&p.path)).to_string();
        let preview_icon = remote.preview.as_ref().map_or(ico::FILE_TEXT, |p| file_icon(&p.path));
        let tabs = [(preview_icon, preview_name.as_str(), !remote.show_output), (ico::TW_BUILD, tr(".NET Output"), remote.show_output)];
        let status = if remote.show_output {
            remote.output.as_ref().map(|o| o.command.clone()).unwrap_or_default()
        } else {
            remote.preview.as_ref().map(|p| format!("{} · {}", p.path, tr("Read-only"))).unwrap_or_default()
        };
        if let Some(tab) = kit::header(ui, "", &tabs, &status, &[]).tab {
            remote.show_output = tab == 1;
        }
    } else if let Some(preview) = &remote.preview {
        kit::header(ui, file_name(&preview.path), &[], &format!("{} · {}", preview.path, tr("Read-only")), &[]);
    }
    if remote.show_output && let Some(output) = &remote.output {
        command_output(ui, output, font_size);
    } else {
        preview(ui, remote.preview.as_ref(), font_size);
    }
}

fn preview(ui: &mut Ui, preview: Option<&Preview>, font_size: f32) {
    let p = rc_kit::pal(ui);
    let Some(preview) = preview else {
        let rect = ui.max_rect();
        ui.painter().text(rect.center() - vec2(0.0, 12.0), Align2::CENTER_CENTER, tr("Select a file in the Explorer to preview it"), FontId::proportional(13.0), p.text_muted);
        ui.painter().text(rect.center() + vec2(0.0, 12.0), Align2::CENTER_CENTER, tr("Remote files are read-only"), FontId::proportional(13.0), p.disabled);
        ui.allocate_rect(rect, Sense::hover());
        return;
    };
    match &preview.body {
        None => loading(ui, tr("Loading…")),
        Some(Err(error)) => {
            kit::content(ui, |ui| kit::message(ui, error, true));
        }
        Some(Ok((text, truncated))) => {
            if *truncated {
                kit::content(ui, |ui| kit::message(ui, tr("Preview limited to the first 64 KiB"), false));
            }
            monospace(ui, "remote_preview", text, font_size);
        }
    }
}

fn command_output(ui: &mut Ui, output: &CommandOutput, font_size: f32) {
    match &output.result {
        None => loading(ui, tr("Running…")),
        Some(Err(error)) => {
            kit::content(ui, |ui| kit::message(ui, error, true));
        }
        Some(Ok(None)) => {
            kit::content(ui, |ui| kit::message(ui, tr("Cancelled"), false));
        }
        Some(Ok(Some(result))) => {
            if result.code != Some(0) {
                let code = result.code.map_or_else(|| "?".to_string(), |code| code.to_string());
                kit::content(ui, |ui| kit::message(ui, &trf("Exited with code {code}", &[("code", &code)]), true));
            }
            monospace(ui, "remote_output", &result.output, font_size);
        }
    }
}

fn loading(ui: &mut Ui, text: &str) {
    kit::content(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spinner();
            kit::note(ui, text);
        });
    });
}

fn monospace(ui: &mut Ui, id: &str, text: &str, font_size: f32) {
    let p = rc_kit::pal(ui);
    egui::ScrollArea::both().id_salt(id).auto_shrink([false, false]).show(ui, |ui| {
        kit::content(ui, |ui| {
            ui.label(egui::RichText::new(text).font(FontId::monospace(font_size)).color(p.text));
        });
    });
}

#[cfg(test)]
impl RemoteExplorer {
    /// A folder as if the agent had listed it, for captures and tests.
    pub fn open_for_test(&mut self, connection: RemoteConnection, listings: Vec<(&str, Vec<(&str, bool)>)>) {
        let root = connection.folder.clone();
        self.connection = Some(connection);
        self.status = Status::Connected;
        for (dir, entries) in listings {
            let entries = entries.into_iter().map(|(name, is_dir)| Entry { name: name.into(), is_dir }).collect();
            self.expanded.insert(dir.to_string());
            self.cache.insert(dir.to_string(), sorted(entries));
        }
        self.root = Some(root);
    }

    pub fn preview_for_test(&mut self, path: &str, text: &str) {
        self.preview = Some(Preview { path: path.into(), body: Some(preview_text(text.as_bytes().to_vec())) });
    }

    pub fn lose_for_test(&mut self, error: &str) {
        self.status = Status::Lost;
        self.error = Some(error.into());
    }

    pub fn form_error_for_test(&mut self, error: &str) {
        self.form_error = Some(error.into());
    }

    pub fn sdks_for_test(&mut self, versions: &[&str]) {
        self.sdk_versions = Some(versions.iter().map(|v| v.to_string()).collect());
    }

    pub fn output_for_test(&mut self, command: &str, code: i32, output: &str) {
        self.output = Some(CommandOutput {
            command: command.into(),
            result: Some(Ok(Some(CommandResult { code: Some(code), stdout: output.into(), output: output.into() }))),
        });
        self.show_output = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_paths_keep_posix_separators() {
        assert_eq!(remote_root(" /home/dev/ "), Some("/home/dev".into()));
        assert_eq!(remote_root("///"), Some("/".into()));
        assert_eq!(join_remote("/", "repo"), "/repo");
        assert_eq!(join_remote("/home/dev", "App.csproj"), "/home/dev/App.csproj");
        assert_eq!(remote_root("C:\\repo"), None);
        assert_eq!(remote_root("relative/path"), None);
        assert_eq!(remote_root("/tmp/a\nb"), None);
        assert_eq!(file_name("/home/dev/App"), "App");
        assert_eq!(file_name("/"), "/");
    }

    #[test]
    fn build_and_test_use_selected_project_without_a_shell() {
        assert!(is_dotnet_target("/repo/tests/App.Tests.csproj"));
        assert!(!is_dotnet_target("/repo/Program.cs"));
        assert_eq!(dotnet_args(CommandKind::Build, Some("/repo/App.sln".into())), ["build", "/repo/App.sln"]);
        assert_eq!(dotnet_args(CommandKind::Test, Some("/repo/tests/App Tests.csproj".into())), ["test", "/repo/tests/App Tests.csproj"]);
        assert_eq!(dotnet_args(CommandKind::Sdks, Some("/repo/App.sln".into())), ["--list-sdks"]);
        assert_eq!(sdk_versions("8.0.428 [/usr/share/dotnet/sdk]\n10.0.100 [/usr/share/dotnet/sdk]\n"), ["8.0.428", "10.0.100"]);
    }

    #[test]
    fn folders_are_listed_first() {
        let entries = sorted(vec![
            Entry { name: "b.cs".into(), is_dir: false },
            Entry { name: "src".into(), is_dir: true },
            Entry { name: "A.cs".into(), is_dir: false },
        ]);
        let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["src", "A.cs", "b.cs"]);
    }

    #[test]
    fn previews_are_cut_on_a_character_boundary() {
        let text = "é".repeat(PREVIEW_BYTES);
        let (preview, truncated) = preview_text(text.into_bytes()).unwrap();
        assert!(truncated);
        assert!(preview.len() <= PREVIEW_BYTES);
        assert!(preview_text(vec![0xff, 0xfe]).is_err());
    }

    #[test]
    fn the_dialog_only_accepts_launchable_connections() {
        let mut c = RemoteConnection { wsl: false, host: "dev@build-box".into(), agent: DEFAULT_AGENT.into(), folder: "/srv/app".into(), dotnet: "dotnet".into() };
        assert!(endpoint(&c).is_some());
        c.host = "-oProxyCommand=sh".into();
        assert!(endpoint(&c).is_none());
        c.host = "dev box".into();
        assert!(endpoint(&c).is_none());
        c.wsl = true;
        c.host = "Ubuntu 24.04".into();
        assert!(endpoint(&c).is_some());
    }

    /// A dropped connection keeps the tree; a failed first connection goes back to the dialog.
    #[test]
    fn lost_connections_keep_an_open_folder() {
        let mut remote = RemoteExplorer::new(Waker::noop(), RemoteConnection::default());
        remote.status = Status::Connecting;
        remote.events.sender().send(Event::Lost("Permission denied (publickey).".into()));
        assert!(!remote.pump());
        assert!(!remote.is_open());
        assert_eq!(remote.form_error.as_deref(), Some("Permission denied (publickey)."));
        let c = RemoteConnection { wsl: false, host: "box".into(), agent: "agent".into(), folder: "/srv".into(), dotnet: "dotnet".into() };
        remote.connection = Some(c);
        remote.status = Status::Connecting;
        remote.events.sender().send(Event::Connected("/srv".into(), vec![Entry { name: "App".into(), is_dir: true }]));
        assert!(remote.pump());
        assert!(remote.is_open());
        remote.output = Some(CommandOutput { command: "dotnet build".into(), result: None });
        remote.events.sender().send(Event::Lost("Broken pipe".into()));
        assert!(!remote.pump());
        assert!(matches!(remote.output.as_ref().and_then(|o| o.result.as_ref()), Some(Err(_))));
        assert!(remote.is_open());
        assert_eq!(remote.status, Status::Lost);
        assert!(remote.cache.contains_key("/srv"));
    }
}
