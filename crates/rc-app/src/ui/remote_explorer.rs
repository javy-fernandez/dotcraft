//! File › Open Remote Folder…: a folder on another machine (over SSH) or in a WSL
//! distribution, served by `dotcraft-remote-agent`. On connect DotCraft installs the agent of
//! its own version when the remote machine does not have it yet (`rc_remote::deploy`). The
//! Explorer shows the folder's tree, the editor area edits the selected file (saved through
//! the agent), builds and tests run on the remote machine, and Run and the Terminal open an
//! SSH or WSL session there. Every request runs on one worker task, never on the UI thread;
//! closing the folder saves the open file, then ends the connection, and the agent stops
//! whatever it was running.

use crate::app::DotCraftApp;
use crate::toolwindow::ToolWindow;
use crate::ui::{dialogs, kit};
use egui::{Align2, Context, FontId, Rect, Sense, Ui, pos2, vec2};
use rc_core::settings::RemoteConnection;
use rc_core::{Inbox, Runtime, Waker, inbox::Outbox};
use rc_editor::{Action, EditorState, EditorView};
use rc_kit::i18n::{tr, trf};
use rc_kit::{Ico, ico, icons, theme};
use rc_remote::deploy::{self, Release, Step};
use rc_remote::{Client, Endpoint, Entry, Operation, RemoteSession, Value, is_connection_error, shell_word};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::{mpsc, oneshot};

/// Where `cargo install` puts the agent: the hint of a custom agent path.
pub const DEFAULT_AGENT: &str = "~/.cargo/bin/dotcraft-remote-agent";
/// The agent reads and writes files up to 1 MiB.
const MAX_FILE_BYTES: usize = 1024 * 1024;

/// Editor ids of remote files, apart from the local documents' ids.
static NEXT_EDITOR_ID: AtomicU64 = AtomicU64::new(1 << 43);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Status {
    Idle,
    Connecting,
    Connected,
    /// The connection dropped; the tree stays until Reconnect or Close.
    Lost,
}

/// What a connection in progress is doing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase {
    Deploy(Step),
    Starting,
}

fn phase_text(phase: Phase) -> &'static str {
    match phase {
        Phase::Deploy(Step::Checking) => tr("Checking the agent on the remote machine…"),
        Phase::Deploy(Step::DownloadingOnRemote) => tr("The remote machine is downloading the agent…"),
        Phase::Deploy(Step::Downloading) => tr("Downloading the agent…"),
        Phase::Deploy(Step::Uploading) => tr("Copying the agent to the remote machine…"),
        Phase::Starting => tr("Starting the agent…"),
    }
}

enum Request {
    List(String),
    Read(String),
    /// Saves a file: its path, bytes and the buffer version they come from.
    Write(String, Vec<u8>, u64),
    /// A `dotnet` command in the folder, with a project or solution as target; firing the
    /// sender cancels it.
    Command(CommandKind, Option<String>, oneshot::Receiver<()>),
    /// Finishes the writes sent before, then closes the connection.
    Shutdown,
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
    Progress(Phase),
    /// The agent could not be installed: the dialog offers the manual install.
    DeployFailed(String),
    Connected(String, Vec<Entry>),
    Listed(String, Vec<Entry>),
    ListFailed(String, String),
    Read(String, Result<Vec<u8>, String>),
    Saved(String, u64, Result<(), String>),
    CommandDone(CommandKind, Result<Option<CommandResult>, String>),
    /// The connection could not be made, or broke.
    Lost(String),
}

/// The file open in the editor area.
pub struct RemoteFile {
    pub path: String,
    /// The editor once the file arrived, or why it cannot be edited; `None` while it loads.
    pub body: Option<Result<EditorState, String>>,
    bom: bool,
    saving: bool,
    save_error: Option<String>,
}

impl RemoteFile {
    fn modified(&self) -> bool {
        matches!(&self.body, Some(Ok(editor)) if editor.buffer.is_modified())
    }
}

pub struct RemoteExplorer {
    /// The connection dialog's fields.
    pub form: RemoteConnection,
    form_error: Option<String>,
    /// The last connection failed while installing the agent: show how to install it by hand.
    deploy_failed: bool,
    /// The connection in use (the form as it was when Connect was pressed).
    connection: Option<RemoteConnection>,
    /// The open remote folder, once its first listing arrived.
    root: Option<String>,
    status: Status,
    phase: Option<Phase>,
    error: Option<String>,
    cache: HashMap<String, Vec<Entry>>,
    loading: HashSet<String>,
    expanded: HashSet<String>,
    file: Option<RemoteFile>,
    running: Option<CommandKind>,
    cancel_command: Option<oneshot::Sender<()>>,
    sdk_versions: Option<Vec<String>>,
    output: Option<CommandOutput>,
    /// The editor area shows the command output instead of the file.
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
            deploy_failed: false,
            connection: None,
            root: None,
            status: Status::Idle,
            phase: None,
            error: None,
            cache: HashMap::new(),
            loading: HashSet::new(),
            expanded: HashSet::new(),
            file: None,
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

    /// Closes the remote folder. Unsaved edits of the open file are saved first: the worker
    /// finishes that write, then drops the session, which ends ssh or wsl.exe; the agent sees
    /// its input close and kills the processes it started.
    pub fn disconnect(&mut self) {
        let modified = self.file.as_ref().is_some_and(RemoteFile::modified);
        if modified && self.status == Status::Connected {
            self.save();
            if let Some(tx) = self.requests.take() {
                let _ = tx.send(Request::Shutdown);
            }
            // Let it finish on its own.
            self.worker = None;
        }
        self.stop_worker();
        self.connection = None;
        self.root = None;
        self.status = Status::Idle;
        self.phase = None;
        self.error = None;
        self.cache.clear();
        self.loading.clear();
        self.expanded.clear();
        self.file = None;
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

    /// Connects again after the connection dropped, keeping the expanded folders and the
    /// open file (with its unsaved edits).
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
        self.deploy_failed = false;
    }

    fn start(&mut self, endpoint: Endpoint, folder: String) {
        self.stop_worker();
        self.status = Status::Connecting;
        self.phase = None;
        self.form_error = None;
        self.deploy_failed = false;
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
                Event::Progress(phase) => self.phase = Some(phase),
                Event::DeployFailed(error) => {
                    self.lose(error);
                    self.deploy_failed = self.root.is_none();
                }
                Event::Connected(folder, entries) => {
                    opened = self.root.is_none();
                    self.status = Status::Connected;
                    self.phase = None;
                    self.expanded.insert(folder.clone());
                    self.cache.insert(folder.clone(), sorted(entries));
                    self.root = Some(folder);
                    self.start_command(CommandKind::Sdks);
                    // A file that was loading when the connection dropped.
                    if let Some(path) = self.file.as_ref().filter(|f| matches!(f.body, None | Some(Err(_)))).map(|f| f.path.clone()) {
                        self.file = None;
                        self.select_file(&path);
                    }
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
                Event::Read(path, bytes) => {
                    if let Some(file) = self.file.as_mut().filter(|f| f.path == path) {
                        match bytes.and_then(|bytes| file_text(&bytes)) {
                            Ok((text, bom)) => {
                                let id = NEXT_EDITOR_ID.fetch_add(1, Ordering::Relaxed);
                                let mut editor = EditorState::new(id, Some(path.into()), &text);
                                editor.parse_blocking();
                                file.body = Some(Ok(editor));
                                file.bom = bom;
                            }
                            Err(error) => file.body = Some(Err(error)),
                        }
                    }
                }
                Event::Saved(path, version, result) => match self.file.as_mut().filter(|f| f.path == path) {
                    Some(file) => {
                        file.saving = false;
                        match result {
                            Ok(()) => {
                                file.save_error = None;
                                if let Some(Ok(editor)) = &mut file.body
                                    && editor.buffer.version() == version
                                {
                                    editor.buffer.mark_saved();
                                }
                            }
                            Err(error) => file.save_error = Some(error),
                        }
                    }
                    None => {
                        if let Err(error) = result {
                            self.error = Some(trf("Could not save {file}: {error}", &[("file", &file_name(&path)), ("error", &error)]));
                        }
                    }
                },
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
                Event::Lost(error) => self.lose(error),
            }
        }
        opened
    }

    fn lose(&mut self, error: String) {
        self.requests = None;
        self.worker = None;
        self.phase = None;
        self.loading.clear();
        self.running = None;
        self.cancel_command = None;
        // A command in flight died with the connection.
        if let Some(output) = self.output.as_mut().filter(|o| o.result.is_none()) {
            output.result = Some(Err(tr("Connection lost").into()));
        }
        if let Some(file) = self.file.as_mut() {
            if file.body.is_none() {
                file.body = Some(Err(tr("Connection lost").into()));
            }
            if file.saving {
                file.saving = false;
                file.save_error = Some(tr("Connection lost").into());
            }
        }
        if self.root.is_some() {
            self.status = Status::Lost;
            self.error = Some(error);
        } else {
            self.disconnect();
            self.form_error = Some(error);
        }
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

    /// Opens a file in the editor area; the one open before is saved when it was edited
    /// (like Rider, which saves when you leave a file).
    fn select_file(&mut self, path: &str) {
        if self.file.as_ref().is_some_and(|f| f.path == path && matches!(f.body, Some(Ok(_)))) {
            self.show_output = false;
            return;
        }
        if self.file.as_ref().is_some_and(RemoteFile::modified) {
            self.save();
        }
        self.show_output = false;
        self.file = Some(RemoteFile { path: path.to_string(), body: None, bom: false, saving: false, save_error: None });
        if !self.send(Request::Read(path.to_string()))
            && let Some(file) = &mut self.file
        {
            file.body = Some(Err(tr("Not connected").into()));
        }
    }

    /// Saves the open file through the agent (File › Save, ⌘S).
    pub fn save(&mut self) {
        let Some(file) = &self.file else { return };
        let Some(Ok(editor)) = &file.body else { return };
        if !editor.buffer.is_modified() || file.saving {
            return;
        }
        let mut bytes = if file.bom { vec![0xEF, 0xBB, 0xBF] } else { Vec::new() };
        bytes.extend_from_slice(editor.buffer.to_disk_string().as_bytes());
        let (path, version) = (file.path.clone(), editor.buffer.version());
        let error = if bytes.len() > MAX_FILE_BYTES {
            Some(tr("Remote files are saved up to 1 MiB").to_string())
        } else if !self.send(Request::Write(path, bytes, version)) {
            Some(tr("Not connected").to_string())
        } else {
            None
        };
        if let Some(file) = &mut self.file {
            file.saving = error.is_none();
            file.save_error = error;
        }
    }

    /// Runs a `dotnet` command in the folder: on the selected project or solution, or on
    /// whatever `dotnet` finds in the folder.
    fn start_command(&mut self, kind: CommandKind) {
        if self.running.is_some() {
            return;
        }
        let target = self.selected_target();
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

    /// The open file when it is a project or solution.
    fn selected_target(&self) -> Option<String> {
        self.file.as_ref().map(|f| f.path.clone()).filter(|path| is_dotnet_target(path))
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

    /// An interactive shell on the remote machine in the open folder, for the Terminal tool
    /// window: the program to start here, its arguments and the tab title.
    pub fn terminal_command(&self) -> Option<(String, Vec<String>, String)> {
        let (connection, root) = (self.connection.as_ref()?, self.root.as_ref()?);
        let (program, args) = session_command(connection, root, None);
        Some((program, args, connection.host.trim().to_string()))
    }

    /// `dotnet run` on the remote machine (the open project, or the one in the folder), in a
    /// terminal so its output streams and it reads input; closing the tab stops it.
    pub fn run_command(&self) -> Option<(String, Vec<String>, String)> {
        let (connection, root) = (self.connection.as_ref()?, self.root.as_ref()?);
        let project = self.selected_target().filter(|path| !is_solution(path));
        let mut words = vec![connection.dotnet.trim().to_string(), "run".to_string()];
        if let Some(project) = &project {
            words.extend(["--project".to_string(), project.clone()]);
        }
        let name = project.as_deref().map_or_else(|| file_name(root).to_string(), |p| file_stem(p).to_string());
        let (program, args) = session_command(connection, root, Some(&words));
        Some((program, args, name))
    }
}

fn endpoint(c: &RemoteConnection) -> Option<Endpoint> {
    let host = c.host.trim().to_string();
    // The automatic agent lives in a folder of this version, put there on connect.
    let agent = if c.custom_agent { c.agent.trim().to_string() } else { Release::current().agent_path() };
    let endpoint = if c.wsl { Endpoint::Wsl { distribution: host, agent } } else { Endpoint::Ssh { host, agent } };
    endpoint.command().ok().map(|_| endpoint)
}

/// The program and arguments that open an interactive session on the remote machine in
/// `folder`: `words` run there, or the user's login shell. Unlike the agent's connection
/// this one has a terminal, so ssh may ask for a password or a host key.
fn session_command(c: &RemoteConnection, folder: &str, words: Option<&[String]>) -> (String, Vec<String>) {
    let run = match words {
        Some(words) => words.iter().map(|w| shell_word(w)).collect::<Vec<_>>().join(" "),
        None => "\"${SHELL:-/bin/sh}\" -l".to_string(),
    };
    let line = format!("cd {} && exec {run}", shell_word(folder));
    let host = c.host.trim().to_string();
    if c.wsl {
        ("wsl.exe".into(), vec!["--distribution".into(), host, "--exec".into(), "sh".into(), "-c".into(), line])
    } else {
        ("ssh".into(), vec!["-t".into(), "--".into(), host, format!("sh -c {}", shell_word(&line))])
    }
}

/// Folders first, then by name, like the local Explorer.
fn sorted(mut entries: Vec<Entry>) -> Vec<Entry> {
    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    entries
}

/// Downloads release assets here, for the agent install.
struct HttpFetch;

impl deploy::Fetch for HttpFetch {
    async fn get(&self, url: &str) -> anyhow::Result<Vec<u8>> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .user_agent(concat!("DotCraft/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(client.get(url).send().await?.error_for_status()?.bytes().await?.to_vec())
    }
}

/// The worker: installs the agent when needed, then one connection; requests run side by
/// side (browse while a build runs). Aborting it drops the session and every request in
/// flight.
async fn serve(endpoint: Endpoint, connection: RemoteConnection, root: String, mut requests: mpsc::UnboundedReceiver<Request>, out: Outbox<Event>) {
    if !connection.custom_agent {
        let progress = |step: Step| {
            out.send(Event::Progress(Phase::Deploy(step)));
        };
        if let Err(error) = deploy::ensure_agent(&endpoint, &HttpFetch, &Release::current(), &progress).await {
            // ssh could not log in, say: a connection problem, which installing by hand
            // would not solve.
            out.send(match error.downcast_ref::<deploy::Unreachable>() {
                Some(unreachable) => Event::Lost(unreachable.to_string()),
                None => Event::DeployFailed(format!("{error:#}")),
            });
            return;
        }
    }
    out.send(Event::Progress(Phase::Starting));
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
            request = requests.recv() => match request {
                None | Some(Request::Shutdown) => return,
                // In order, so a shutdown right after a save waits for it.
                Some(Request::Write(path, bytes, version)) => {
                    let result = write(&client, &path, bytes).await;
                    if let Some(error) = result.as_ref().err().filter(|e| is_connection_error(e)) {
                        out.send(Event::Lost(describe(&session, error)));
                        return;
                    }
                    out.send(Event::Saved(path, version, result.map_err(|e| e.to_string())));
                }
                Some(request) => {
                    let (client, root, dotnet) = (client.clone(), root.clone(), dotnet.clone());
                    tasks.spawn(async move { handle(&client, &root, &dotnet, request).await });
                }
            },
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
            Ok(bytes) => (Event::Read(path, Ok(bytes)), None),
            Err(error) => (Event::Read(path, Err(error.to_string())), Some(error)),
        },
        Request::Command(kind, target, cancel) => match remote_command(client, root, dotnet, kind, target, cancel).await {
            Ok(result) => (Event::CommandDone(kind, Ok(result)), None),
            Err(error) => (Event::CommandDone(kind, Err(error.to_string())), Some(error)),
        },
        Request::Write(..) | Request::Shutdown => unreachable!("handled by the worker loop"),
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

async fn read(client: &Client, path: &str) -> anyhow::Result<Vec<u8>> {
    let Value::Data { data } = client.request(Operation::Read { path: path.to_string() }).await? else {
        anyhow::bail!("unexpected remote file response");
    };
    rc_remote::decode_data(&data)
}

async fn write(client: &Client, path: &str, bytes: Vec<u8>) -> anyhow::Result<()> {
    let data = rc_remote::encode_data(&bytes);
    match client.request(Operation::Write { path: path.to_string(), data }).await? {
        Value::Written => Ok(()),
        _ => anyhow::bail!("unexpected remote write response"),
    }
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

fn is_solution(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    [".sln", ".slnx", ".slnf"].iter().any(|ext| lower.ends_with(ext))
}

fn is_dotnet_target(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    is_solution(path) || [".csproj", ".fsproj", ".vbproj"].iter().any(|ext| lower.ends_with(ext))
}

/// The versions in `dotnet --list-sdks` output (`8.0.428 [/usr/share/dotnet/sdk]`).
fn sdk_versions(stdout: &str) -> Vec<String> {
    stdout.lines().filter_map(|line| line.split_whitespace().next()).map(str::to_string).collect()
}

/// A file's text and whether it starts with a UTF-8 BOM, like local files; binary files are
/// not edited.
fn file_text(bytes: &[u8]) -> Result<(String, bool), String> {
    let (bytes, bom) = match bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        Some(rest) => (rest, true),
        None => (bytes, false),
    };
    if bytes.iter().take(8000).any(|&b| b == 0) {
        return Err(tr("This is a binary file and cannot be edited").to_string());
    }
    Ok((String::from_utf8_lossy(bytes).into_owned(), bom))
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

fn file_stem(path: &str) -> &str {
    let name = file_name(path);
    name.rsplit_once('.').map_or(name, |(stem, _)| stem)
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
    let version = Release::current().version;
    let (mut connect, mut cancel) = (false, false);
    let ((), escape) = dialogs::modal(ctx, "remote_connect", width, |ui| {
        dialogs::title(ui, tr("Open Remote Folder"));
        let transports = vec!["SSH".to_string(), "WSL".to_string()];
        let agents = vec![tr("Automatic").to_string(), tr("Custom path").to_string()];
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
            ui.label(tr("Remote folder"));
            ui.add_enabled(!connecting, egui::TextEdit::singleline(&mut remote.form.folder).hint_text("/home/me/src/App").desired_width(field_width));
            ui.end_row();
            ui.label(tr(".NET path"));
            ui.add_enabled(!connecting, egui::TextEdit::singleline(&mut remote.form.dotnet).hint_text("dotnet").desired_width(field_width));
            ui.end_row();
            ui.label(tr("Agent"));
            if let Some(i) = kit::dropdown(ui, "remote_agent", &agents[remote.form.custom_agent as usize], &agents, !connecting) {
                remote.form.custom_agent = i == 1;
            }
            ui.end_row();
            if remote.form.custom_agent {
                ui.label(tr("Agent path"));
                ui.add_enabled(!connecting, egui::TextEdit::singleline(&mut remote.form.agent).hint_text(DEFAULT_AGENT).desired_width(field_width));
                ui.end_row();
            }
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
        if remote.form.custom_agent {
            kit::note(ui, tr("Install the agent on that machine with:"));
            ui.label(egui::RichText::new(deploy::manual_install_command(&version)).monospace().size(11.5));
        } else {
            kit::note(ui, &trf("DotCraft installs its agent ({version}) in ~/.dotcraft/remote on that machine when it is missing: the machine downloads it from the DotCraft releases on GitHub, or DotCraft copies it over this connection.", &[("version", &version)]));
        }
        if let Some(error) = &remote.form_error {
            ui.add_space(6.0);
            kit::message(ui, error, true);
            if remote.deploy_failed && !remote.form.custom_agent {
                ui.add_space(4.0);
                kit::note(ui, tr("Install the agent by hand on that machine, then choose Custom path:"));
                ui.label(egui::RichText::new(deploy::manual_install_command(&version)).monospace().size(11.5));
            }
        }
        ui.add_space(14.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            connect = ui.add_enabled_ui(valid && !connecting, |ui| theme::primary_button(ui, tr("Connect"))).inner.clicked();
            cancel = theme::secondary_button(ui, tr("Cancel")).clicked();
            if connecting {
                ui.spinner();
                kit::note(ui, remote.phase.map_or(tr("Connecting…"), phase_text));
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
    let mut run = false;
    if connected {
        let busy = remote.running.is_some();
        kit::toolbar(ui, |ui| {
            if kit::button(ui, Some(ico::BUILD), tr("Build"), !busy).clicked() {
                remote.start_command(CommandKind::Build);
            }
            if kit::button(ui, Some(ico::RUN_ALL), tr("Test"), !busy).clicked() {
                remote.start_command(CommandKind::Test);
            }
            run = kit::button(ui, Some(ico::RUN), tr("Run"), true).clicked();
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
            let text = remote.phase.map_or(tr("Connecting…"), phase_text);
            kit::toolbar(ui, |ui| {
                ui.spinner();
                kit::note(ui, text);
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
    if run && let Some((program, args, name)) = app.remote.run_command() {
        app.open_remote_session(program, args, name, true);
    }
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
            let selected = remote.file.as_ref().is_some_and(|f| f.path == path);
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

/// The editor area while a remote folder is open and no local file is: the open remote
/// file or the output of the last `dotnet` command. `editor_actions` come from the menus.
pub fn editor(app: &mut DotCraftApp, ui: &mut Ui, editor_actions: Vec<Action>) {
    let remote = &mut app.remote;
    let file_title = remote.file.as_ref().map_or_else(
        || tr("Editor").to_string(),
        |f| format!("{}{}", file_name(&f.path), if f.modified() { " *" } else { "" }),
    );
    let file_status = remote.file.as_ref().map(|f| if f.saving { format!("{} · {}", f.path, tr("Saving…")) } else { f.path.clone() }).unwrap_or_default();
    if remote.output.is_some() {
        let file_icon = remote.file.as_ref().map_or(ico::FILE_TEXT, |f| file_icon(&f.path));
        let tabs = [(file_icon, file_title.as_str(), !remote.show_output), (ico::TW_BUILD, tr(".NET Output"), remote.show_output)];
        let status = if remote.show_output { remote.output.as_ref().map(|o| o.command.clone()).unwrap_or_default() } else { file_status };
        if let Some(tab) = kit::header(ui, "", &tabs, &status, &[]).tab {
            remote.show_output = tab == 1;
        }
    } else if remote.file.is_some() {
        kit::header(ui, &file_title, &[], &file_status, &[]);
    }
    if remote.show_output && let Some(output) = &remote.output {
        command_output(ui, output, app.settings.editor_font_size);
        return;
    }
    let p = rc_kit::pal(ui);
    let Some(file) = &mut remote.file else {
        let rect = ui.max_rect();
        ui.painter().text(rect.center(), Align2::CENTER_CENTER, tr("Select a file in the Explorer to open it"), FontId::proportional(13.0), p.text_muted);
        ui.allocate_rect(rect, Sense::hover());
        return;
    };
    if let Some(error) = &file.save_error {
        kit::content(ui, |ui| kit::message(ui, &trf("Could not save {file}: {error}", &[("file", &file_name(&file.path)), ("error", error)]), true));
    }
    match &mut file.body {
        None => loading(ui, tr("Loading…")),
        Some(Err(error)) => {
            kit::content(ui, |ui| kit::message(ui, error, true));
        }
        Some(Ok(state)) => {
            let rect = ui.available_rect_before_wrap();
            let mut ed_ui = ui.new_child(egui::UiBuilder::new().max_rect(rect));
            ed_ui.set_clip_rect(rect.intersect(ui.clip_rect()));
            for action in &editor_actions {
                for other in rc_editor::run_action(&ed_ui, state, &app.editor_config, *action) {
                    if other.is_ide() {
                        app.pending.push(other);
                    }
                }
            }
            let out = EditorView::new(state, &app.editor_config, &app.keymap).show(&mut ed_ui);
            ui.allocate_rect(rect, Sense::hover());
            // Remote files are at most 1 MiB: reparse right away (incremental) after edits.
            if state.syntax.lang != rc_syntax::Lang::Plain && !state.syntax.is_current() {
                state.parse_blocking();
            }
            // Global shortcuts (⌘S saves through `run_ide_action`); language features need
            // Roslyn, which does not run on the remote machine yet.
            app.pending.extend(out.actions.into_iter().filter(|a| a.is_ide()));
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

    /// A file open in the editor, as if the agent had read it.
    pub fn file_for_test(&mut self, path: &str, text: &str) {
        self.file = Some(RemoteFile { path: path.into(), body: None, bom: false, saving: false, save_error: None });
        self.events.sender().send(Event::Read(path.into(), Ok(text.as_bytes().to_vec())));
        self.pump();
    }

    pub fn file_editor_for_test(&mut self) -> Option<&mut EditorState> {
        self.file.as_mut().and_then(|f| f.body.as_mut()).and_then(|b| b.as_mut().ok())
    }

    pub fn lose_for_test(&mut self, error: &str) {
        self.status = Status::Lost;
        self.error = Some(error.into());
    }

    pub fn form_error_for_test(&mut self, error: &str, deploy_failed: bool) {
        self.form_error = Some(error.into());
        self.deploy_failed = deploy_failed;
    }

    pub fn connecting_for_test(&mut self, step: Step) {
        self.status = Status::Connecting;
        self.phase = Some(Phase::Deploy(step));
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

    fn connection() -> RemoteConnection {
        RemoteConnection { host: "dev@build-box".into(), agent: DEFAULT_AGENT.into(), folder: "/srv/app".into(), dotnet: "dotnet".into(), ..Default::default() }
    }

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
        assert_eq!(file_stem("/home/dev/Shop.Api.csproj"), "Shop.Api");
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
    fn files_keep_their_bom_and_binary_files_are_not_edited() {
        assert_eq!(file_text(b"\xEF\xBB\xBFclass A {}\r\n").unwrap(), ("class A {}\r\n".to_string(), true));
        assert_eq!(file_text(b"x").unwrap(), ("x".to_string(), false));
        assert!(file_text(&[0x4d, 0x5a, 0x00, 0x01]).is_err());
    }

    #[test]
    fn the_dialog_only_accepts_launchable_connections() {
        let mut c = connection();
        assert!(endpoint(&c).is_some());
        c.host = "-oProxyCommand=sh".into();
        assert!(endpoint(&c).is_none());
        c.host = "dev box".into();
        assert!(endpoint(&c).is_none());
        c.wsl = true;
        c.host = "Ubuntu 24.04".into();
        assert!(endpoint(&c).is_some());
        // A custom agent path is checked; the automatic one is DotCraft's own.
        c.custom_agent = true;
        c.agent = "-x".into();
        assert!(endpoint(&c).is_none());
        c.custom_agent = false;
        let Some(Endpoint::Wsl { agent, .. }) = endpoint(&c) else { panic!("wsl endpoint") };
        assert_eq!(agent, format!("~/.dotcraft/remote/{}/dotcraft-remote-agent", env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn terminals_and_runs_open_a_session_in_the_folder() {
        let mut c = connection();
        c.folder = "/srv/my app".into();
        let (program, args) = session_command(&c, &c.folder, None);
        assert_eq!(program, "ssh");
        assert_eq!(args[..3], ["-t", "--", "dev@build-box"]);
        let words = ["~/.dotnet/dotnet".to_string(), "run".into(), "--project".into(), "/srv/my app/It's.csproj".into()];
        let (_, args) = session_command(&c, &c.folder, Some(&words));
        assert!(args[3].starts_with("sh -c "), "{}", args[3]);
        c.wsl = true;
        c.host = "Ubuntu".into();
        let (program, args) = session_command(&c, &c.folder, None);
        assert_eq!(program, "wsl.exe");
        assert_eq!(args[..5], ["--distribution", "Ubuntu", "--exec", "sh", "-c"]);
        assert_eq!(args[5], "cd '/srv/my app' && exec \"${SHELL:-/bin/sh}\" -l");
    }

    /// The SSH line goes through the remote login shell and then `sh -c`: the words must
    /// arrive unchanged, and `~/` must still reach the home folder.
    #[cfg(unix)]
    #[test]
    fn session_lines_survive_two_shells() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("my app");
        std::fs::create_dir_all(&folder).unwrap();
        let mut c = connection();
        c.folder = folder.display().to_string();
        let words = ["printf".to_string(), "%s|".into(), "a b".into(), "it's".into(), "$(id)".into(), "~/x".into()];
        let (_, args) = session_command(&c, &c.folder, Some(&words));
        let out = std::process::Command::new("sh").args(["-c", &args[3]]).env("HOME", "/home/dev").output().unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), "a b|it's|$(id)|/home/dev/x|");
        let pwd = std::process::Command::new("sh")
            .args(["-c", &session_command(&c, &c.folder, Some(&["pwd".to_string()])).1[3]])
            .output()
            .unwrap();
        assert_eq!(dunce::canonicalize(String::from_utf8_lossy(&pwd.stdout).trim()).unwrap(), dunce::canonicalize(&folder).unwrap());
    }

    #[test]
    fn run_uses_the_open_project() {
        let mut remote = RemoteExplorer::new(Waker::noop(), RemoteConnection::default());
        remote.open_for_test(connection(), vec![("/srv/app", vec![("Api", true)])]);
        let (_, args, name) = remote.run_command().unwrap();
        assert_eq!(name, "app");
        assert!(args[3].contains("run") && !args[3].contains("--project"), "{}", args[3]);
        remote.file_for_test("/srv/app/Api/Api.csproj", "<Project />");
        let (_, args, name) = remote.run_command().unwrap();
        assert_eq!(name, "Api");
        assert!(args[3].contains("--project"), "{}", args[3]);
        let (_, _, title) = remote.terminal_command().unwrap();
        assert_eq!(title, "dev@build-box");
    }

    /// Edits are saved through the agent; the buffer is clean once the write of that
    /// version is confirmed, and a failed write says why.
    #[test]
    fn saving_writes_the_edited_text() {
        let mut remote = RemoteExplorer::new(Waker::noop(), RemoteConnection::default());
        remote.open_for_test(connection(), vec![("/srv/app", vec![("Program.cs", false)])]);
        let (tx, mut rx) = mpsc::unbounded_channel();
        remote.requests = Some(tx);
        remote.file_for_test("/srv/app/Program.cs", "\u{feff}class A {}\r\n");
        let editor = remote.file_editor_for_test().unwrap();
        editor.apply_text_edits(vec![rc_text::Change::insert(0, "// x\n")]);
        remote.save();
        let Ok(Request::Write(path, bytes, version)) = rx.try_recv() else { panic!("no write") };
        assert_eq!(path, "/srv/app/Program.cs");
        assert_eq!(bytes, b"\xEF\xBB\xBF// x\r\nclass A {}\r\n");
        assert!(remote.file.as_ref().unwrap().saving);
        remote.events.sender().send(Event::Saved(path.clone(), version, Err("Permission denied (os error 13)".into())));
        remote.pump();
        let file = remote.file.as_ref().unwrap();
        assert!(file.modified() && !file.saving && file.save_error.as_deref() == Some("Permission denied (os error 13)"));
        remote.events.sender().send(Event::Saved(path, version, Ok(())));
        remote.pump();
        assert!(!remote.file.as_ref().unwrap().modified());
        // Opening another file saves nothing more: there are no edits left.
        remote.select_file("/srv/app/Other.cs");
        assert!(matches!(rx.try_recv(), Ok(Request::Read(p)) if p == "/srv/app/Other.cs"));
    }

    /// Closing the folder with unsaved edits saves them before the connection ends.
    #[test]
    fn closing_saves_unsaved_edits_first() {
        let mut remote = RemoteExplorer::new(Waker::noop(), RemoteConnection::default());
        remote.open_for_test(connection(), vec![("/srv/app", vec![("a.txt", false)])]);
        let (tx, mut rx) = mpsc::unbounded_channel();
        remote.requests = Some(tx);
        remote.file_for_test("/srv/app/a.txt", "a");
        remote.file_editor_for_test().unwrap().apply_text_edits(vec![rc_text::Change::insert(1, "b")]);
        remote.disconnect();
        assert!(matches!(rx.try_recv(), Ok(Request::Write(_, bytes, _)) if bytes == b"ab"));
        assert!(matches!(rx.try_recv(), Ok(Request::Shutdown)));
        assert!(!remote.is_open());
    }

    /// A dropped connection keeps the tree; a failed first connection goes back to the
    /// dialog, which offers the manual install when the agent could not be installed.
    #[test]
    fn lost_connections_keep_an_open_folder() {
        let mut remote = RemoteExplorer::new(Waker::noop(), RemoteConnection::default());
        remote.status = Status::Connecting;
        remote.events.sender().send(Event::Lost("Permission denied (publickey).".into()));
        assert!(!remote.pump());
        assert!(!remote.is_open());
        assert_eq!(remote.form_error.as_deref(), Some("Permission denied (publickey)."));
        assert!(!remote.deploy_failed);
        remote.status = Status::Connecting;
        remote.events.sender().send(Event::Progress(Phase::Deploy(Step::Uploading)));
        remote.pump();
        assert_eq!(remote.phase, Some(Phase::Deploy(Step::Uploading)));
        remote.events.sender().send(Event::DeployFailed("there is no DotCraft agent for Linux armv7l".into()));
        remote.pump();
        assert!(remote.deploy_failed && remote.phase.is_none());
        remote.connection = Some(connection());
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
