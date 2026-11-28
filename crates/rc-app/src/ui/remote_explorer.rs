//! File › Open Remote Folder…: a folder on another machine (over SSH) or in a WSL
//! distribution, served by `dotcraft-remote-agent`, which DotCraft installs there on connect
//! (`rc_remote::deploy`). The folder is copied into a local mirror once
//! (`remote_backend`); its files open in ordinary editor tabs and every save goes back to the
//! remote machine. Roslyn and netcoredbg are installed on the remote machine and run there,
//! so code insight, builds, tests, runs and debugging all use the remote SDK. Every remote
//! request runs on the shared runtime, never on the UI thread.

use crate::app::DotCraftApp;
use crate::remote_backend::{self, Backend, Machine, Tools};
use crate::toolwindow::ToolWindow;
use crate::ui::{dialogs, kit};
use egui::{Align2, Context, FontId, Rect, Sense, Ui, pos2, vec2};
use rc_core::mirror::{Mirror, folder_name};
use rc_core::settings::RemoteConnection;
use rc_core::watch::{FileWatcher, FsEvent};
use rc_core::{Inbox, Runtime, Waker, inbox::Outbox};
use rc_kit::i18n::{tr, trf};
use rc_kit::{Ico, ico, icons, theme};
use rc_remote::deploy::{self, Release, Step};
use rc_remote::{Client, Endpoint, Entry, Killer, Operation, RemoteSession, Value, is_connection_error, shell_word};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::mpsc;

/// Where `cargo install` puts the agent: the hint of a custom agent path.
pub const DEFAULT_AGENT: &str = "~/.cargo/bin/dotcraft-remote-agent";

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

/// Requests for the connection worker.
enum Request {
    List(String),
    /// Ends the connection.
    Shutdown,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum CommandKind {
    Info,
    Build,
    Test,
}

/// C# support for the folder.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Language {
    Idle,
    /// Copying the folder, finding its solution, installing Roslyn on the remote machine.
    Preparing,
    Started,
    Failed(String),
}

enum Event {
    Progress(Phase),
    /// The agent could not be installed: the dialog offers the manual install.
    DeployFailed(String),
    Connected(String, Vec<Entry>, Client, Machine),
    Listed(String, Vec<Entry>),
    ListFailed(String, String),
    /// The folder's copy in the mirror (SHA-256 per file).
    Synced(Result<HashMap<PathBuf, String>, String>),
    /// The remote language server to start (its dll), or why there is none.
    LanguageReady(Result<String, String>),
    Fetched { remote: String, line: usize, col: usize, result: Result<(PathBuf, String), String> },
    Pushed(PathBuf, Result<String, String>),
    Sdks(Vec<String>),
    CommandStarted(u64, Killer),
    CommandText(u64, String, bool),
    CommandDone(u64, Result<Option<i32>, String>),
    /// A project built and resolved for debugging (console key, name, launch, adapter).
    DebugReady(u64, String, rc_dap::LaunchConfig, String),
    DebugFailed(u64, String),
    /// The connection could not be made, or broke.
    Lost(String),
}

/// What the rest of the app does for the remote folder.
pub enum RemoteOutput {
    /// The folder has just opened.
    Opened,
    /// Open this local copy at (line, column).
    Open(PathBuf, usize, usize),
    /// Start C# support: the mirror folder, solution and projects, and the remote server.
    Language { root: PathBuf, solution: Option<PathBuf>, projects: Vec<PathBuf>, connect: crate::insight::RemoteConnect },
    /// A console tab for a remote command (key, title), its output and its end.
    Console(u64, String),
    ConsoleText(u64, String, bool),
    ConsoleDone(u64, Option<i32>),
    /// Debug a prepared program with the remote adapter.
    Debug { name: String, launch: rc_dap::LaunchConfig, launcher: rc_dap::session::Launcher },
    Notify(String, bool),
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
    /// The Explorer's selected file (remote path).
    selected: Option<String>,
    backend: Option<Backend>,
    /// The SHA-256 of each mirror file as the remote machine has it.
    hashes: HashMap<PathBuf, String>,
    pushing: HashSet<PathBuf>,
    watcher: Option<FileWatcher>,
    fs_events: Inbox<FsEvent>,
    language: Language,
    /// A running `dotnet` command: its kind, console key and killer.
    running: Option<(CommandKind, u64, Option<Killer>)>,
    /// Preparing a debug session (its console key).
    debugging: Option<u64>,
    next_key: u64,
    sdk_versions: Option<Vec<String>>,
    /// Remote tools already in place (tests).
    pub tools: Tools,
    /// Overrides the mirror folder (tests).
    pub mirror_root: Option<PathBuf>,
    requests: Option<mpsc::UnboundedSender<Request>>,
    worker: Option<tokio::task::JoinHandle<()>>,
    events: Inbox<Event>,
    outputs: Vec<RemoteOutput>,
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
            selected: None,
            backend: None,
            hashes: HashMap::new(),
            pushing: HashSet::new(),
            watcher: None,
            fs_events: Inbox::new(waker.clone()),
            language: Language::Idle,
            running: None,
            debugging: None,
            next_key: 1,
            sdk_versions: None,
            tools: Tools::default(),
            mirror_root: None,
            requests: None,
            worker: None,
            events: Inbox::new(waker.clone()),
            outputs: Vec::new(),
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

    /// The local copies of this connection's files.
    fn mirror_for(&self, connection: &RemoteConnection) -> Mirror {
        let root = self.mirror_root.clone().unwrap_or_else(|| {
            let host = if connection.wsl { format!("wsl-{}", connection.host.trim()) } else { connection.host.trim().to_string() };
            rc_core::paths::data_dir().join("remote").join(folder_name(&host))
        });
        Mirror::new(root)
    }

    /// The mirror of the open folder.
    pub fn mirror(&self) -> Option<&Mirror> {
        self.backend.as_ref().map(|b| &b.mirror)
    }

    /// Closes the remote folder: the session ends (ssh or wsl.exe exits, the agent kills what
    /// it was running). Save and [`RemoteExplorer::flush`] first.
    pub fn disconnect(&mut self) {
        if let Some(tx) = self.requests.take() {
            let _ = tx.send(Request::Shutdown);
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
        self.selected = None;
        self.backend = None;
        self.hashes.clear();
        self.pushing.clear();
        self.watcher = None;
        self.language = Language::Idle;
        if let Some((_, _, Some(killer))) = self.running.take() {
            killer.kill();
        }
        self.debugging = None;
        self.sdk_versions = None;
        self.outputs.clear();
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
        self.debugging = None;
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

    /// Opens `folder` through an agent served in this process (the "remote machine" is this
    /// one): for tests.
    #[cfg(test)]
    pub fn connect_local_for_test(&mut self, connection: RemoteConnection) {
        self.disconnect();
        let folder = connection.folder.clone();
        self.connection = Some(connection);
        self.status = Status::Connecting;
        let client = remote_backend::test_support::local_client();
        let out = self.events.sender();
        self.worker = Some(Runtime::spawn(async move {
            match (list(&client, &folder).await, remote_backend::probe(&client).await) {
                (Ok(entries), Ok(machine)) => {
                    out.send(Event::Connected(folder, entries, client, machine));
                }
                (Err(e), _) | (_, Err(e)) => {
                    out.send(Event::Lost(e.to_string()));
                }
            }
        }));
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

    /// Runs `task` on the shared runtime with the back end; what it returns goes to the UI.
    fn spawn<F>(&self, task: impl FnOnce(Backend) -> F)
    where
        F: std::future::Future<Output = Event> + Send + 'static,
    {
        let Some(backend) = self.backend.clone() else { return };
        let out = self.events.sender();
        let future = task(backend);
        Runtime::spawn(async move {
            out.send(future.await);
        });
    }

    /// Applies what the worker and the tasks sent; returns what the app must do.
    pub fn pump(&mut self) -> Vec<RemoteOutput> {
        for event in self.events.drain() {
            self.on_event(event);
        }
        for event in self.fs_events.drain() {
            if let FsEvent::Changed(path) | FsEvent::Created(path) = event {
                self.push_if_changed(&path);
            }
        }
        std::mem::take(&mut self.outputs)
    }

    fn on_event(&mut self, event: Event) {
        match event {
            Event::Progress(phase) => self.phase = Some(phase),
            Event::DeployFailed(error) => {
                self.lose(error);
                self.deploy_failed = self.root.is_none();
            }
            Event::Connected(folder, entries, client, machine) => {
                let first = self.root.is_none();
                let connection = self.connection.clone().unwrap_or_default();
                self.backend = Some(Backend {
                    client,
                    mirror: self.mirror_for(&connection),
                    root: folder.clone(),
                    dotnet: connection.dotnet.trim().to_string(),
                    machine,
                    tools: self.tools.clone(),
                });
                self.status = Status::Connected;
                self.phase = None;
                self.expanded.insert(folder.clone());
                self.cache.insert(folder.clone(), sorted(entries));
                self.root = Some(folder);
                self.spawn(|b| async move {
                    match remote_backend::capture(&b.client, sdk_list(&b), None).await {
                        Ok(out) if out.code == Some(0) => Event::Sdks(sdk_versions(&String::from_utf8_lossy(&out.stdout))),
                        _ => Event::Sdks(Vec::new()),
                    }
                });
                if first {
                    self.outputs.push(RemoteOutput::Opened);
                    self.language = Language::Preparing;
                    self.spawn(|b| async move { Event::Synced(b.sync_folder().await.map_err(|e| format!("{e:#}"))) });
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
            Event::Synced(result) => match result {
                Ok(hashes) => {
                    self.hashes.extend(hashes);
                    self.watch_mirror();
                    self.start_language();
                }
                Err(error) => {
                    self.language = Language::Failed(error.clone());
                    self.error = Some(trf("Could not copy the folder: {error}", &[("error", &error)]));
                }
            },
            Event::LanguageReady(result) => match result {
                Ok(dll) => self.language_started(dll),
                Err(error) => self.language = Language::Failed(error),
            },
            Event::Fetched { remote, line, col, result } => match result {
                Ok((local, hash)) => {
                    self.hashes.insert(local.clone(), hash);
                    self.watch_mirror();
                    self.outputs.push(RemoteOutput::Open(local, line, col));
                }
                Err(error) => self.outputs.push(RemoteOutput::Notify(trf("Could not open {file}: {error}", &[("file", &file_name(&remote)), ("error", &error)]), true)),
            },
            Event::Pushed(local, result) => {
                self.pushing.remove(&local);
                match result {
                    Ok(hash) => {
                        self.hashes.insert(local.clone(), hash);
                        // Saved again while the push ran.
                        self.push_if_changed(&local);
                    }
                    Err(error) => {
                        let name = local.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        self.outputs.push(RemoteOutput::Notify(trf("Could not save {file}: {error}", &[("file", &name), ("error", &error)]), true));
                    }
                }
            }
            Event::Sdks(versions) => self.sdk_versions = Some(versions),
            Event::CommandStarted(key, killer) => {
                if let Some(running) = self.running.as_mut().filter(|r| r.1 == key) {
                    running.2 = Some(killer);
                }
            }
            Event::CommandText(key, text, stderr) => self.outputs.push(RemoteOutput::ConsoleText(key, text, stderr)),
            Event::CommandDone(key, result) => {
                if self.running.as_ref().is_some_and(|r| r.1 == key) {
                    self.running = None;
                }
                match result {
                    Ok(code) => self.outputs.push(RemoteOutput::ConsoleDone(key, code)),
                    Err(error) => self.outputs.push(RemoteOutput::ConsoleText(key, format!("{error}\n"), true)),
                }
            }
            Event::DebugReady(key, name, launch, exe) => {
                self.debugging = None;
                if let Some(backend) = self.backend.clone() {
                    self.outputs.push(RemoteOutput::ConsoleDone(key, Some(0)));
                    let launcher: rc_dap::session::Launcher = Arc::new(move |out, waker| {
                        let map = rc_dap::remote::PathMap { mirror: backend.mirror.clone() };
                        rc_dap::DapClient::connect(backend.dap_transport(exe.clone()), Some(Arc::new(map)), out, waker)
                    });
                    self.outputs.push(RemoteOutput::Debug { name, launch, launcher });
                }
            }
            Event::DebugFailed(key, error) => {
                self.debugging = None;
                self.outputs.push(RemoteOutput::ConsoleText(key, format!("{error}\n"), true));
                self.outputs.push(RemoteOutput::Notify(error, true));
            }
            Event::Lost(error) => self.lose(error),
        }
    }

    fn lose(&mut self, error: String) {
        self.requests = None;
        self.worker = None;
        self.phase = None;
        self.loading.clear();
        self.running = None;
        self.debugging = None;
        if self.root.is_some() {
            self.status = Status::Lost;
            self.error = Some(error);
        } else {
            self.disconnect();
            self.form_error = Some(error);
        }
    }

    /// Watches the mirror of the folder: a saved file goes back to the remote machine.
    fn watch_mirror(&mut self) {
        if self.watcher.is_some() {
            return;
        }
        let Some(root) = self.backend.as_ref().and_then(|b| b.mirror.local("/")) else { return };
        let _ = std::fs::create_dir_all(&root);
        self.watcher = FileWatcher::new(&root, self.fs_events.sender()).ok();
    }

    /// Pushes a mirror file whose bytes differ from the remote machine's.
    fn push_if_changed(&mut self, local: &Path) {
        let Some(known) = self.hashes.get(local).cloned() else { return };
        if self.pushing.contains(local) || self.status != Status::Connected {
            return;
        }
        let Ok(bytes) = std::fs::read(local) else { return };
        if deploy::sha256_hex(&bytes) == known {
            return;
        }
        self.pushing.insert(local.to_path_buf());
        let local = local.to_path_buf();
        self.spawn(move |b| async move {
            let result = b.push(&local, Some(&known)).await.map_err(|e| format!("{e:#}"));
            Event::Pushed(local, result)
        });
    }

    /// Writes every saved file that has not reached the remote machine yet, waiting for it
    /// (closing the folder).
    pub fn flush(&mut self) {
        let Some(backend) = self.backend.clone().filter(|_| self.status == Status::Connected) else { return };
        let changed: Vec<(PathBuf, String)> = self
            .hashes
            .iter()
            .filter(|(path, hash)| std::fs::read(path).is_ok_and(|bytes| deploy::sha256_hex(&bytes) != **hash))
            .map(|(p, h)| (p.clone(), h.clone()))
            .collect();
        if changed.is_empty() {
            return;
        }
        let _ = Runtime::block_on(tokio::time::timeout(std::time::Duration::from_secs(10), async move {
            for (path, hash) in changed {
                let _ = backend.push(&path, Some(&hash)).await;
            }
        }));
    }

    /// Finds the solution (or projects) in the mirror and installs Roslyn on the remote
    /// machine.
    fn start_language(&mut self) {
        let (Some(backend), Some(root)) = (&self.backend, &self.root) else { return };
        let Some(local_root) = backend.mirror.local(root) else { return };
        let (solution, projects) = find_solution(&local_root);
        if solution.is_none() && projects.is_empty() {
            self.language = Language::Idle;
            return;
        }
        self.spawn(|b| async move { Event::LanguageReady(b.ensure_roslyn().await.map_err(|e| format!("{e:#}"))) });
    }

    fn language_started(&mut self, dll: String) {
        let (Some(backend), Some(root)) = (self.backend.clone(), self.root.clone()) else { return };
        let Some(local_root) = backend.mirror.local(&root) else { return };
        let (solution, projects) = find_solution(&local_root);
        let connect: crate::insight::RemoteConnect = Arc::new(move |out| {
            let map = rc_lsp::remote::UriMap { mirror: backend.mirror.clone() };
            rc_lsp::Client::connect(backend.lsp_transport(dll.clone()), Some(Arc::new(map)), out)
        });
        self.language = Language::Started;
        self.outputs.push(RemoteOutput::Language { root: local_root, solution, projects, connect });
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

    /// Opens a remote file in an editor tab, fetched fresh.
    pub fn open(&mut self, remote: &str, line: usize, col: usize) {
        let remote = remote.to_string();
        if self.status != Status::Connected {
            return;
        }
        self.spawn(move |b| async move {
            let result = b.fetch(&remote).await.map(|(local, bytes)| (local, deploy::sha256_hex(&bytes))).map_err(|e| format!("{e:#}"));
            Event::Fetched { remote, line, col, result }
        });
    }

    /// The app opens `path`: a mirror file that is not open in an editor is fetched first
    /// (`true`: it opens when it arrives).
    pub fn intercept_open(&mut self, path: &Path, line: usize, col: usize, already_open: bool) -> bool {
        if already_open || self.status != Status::Connected {
            return false;
        }
        let Some(remote) = self.mirror().and_then(|m| m.remote(path)) else { return false };
        self.open(&remote, line, col);
        true
    }

    /// Runs a `dotnet` command in the folder, its output in a console tab: on the selected
    /// project or solution, or on whatever `dotnet` finds in the folder.
    fn start_command(&mut self, kind: CommandKind) {
        if self.running.is_some() || self.status != Status::Connected {
            return;
        }
        let target = self.selected_target();
        let args = dotnet_args(kind, target.clone());
        let dotnet = self.connection.as_ref().map(|c| c.dotnet.trim().to_string()).unwrap_or_default();
        let key = self.next_key;
        self.next_key += 1;
        self.running = Some((kind, key, None));
        self.outputs.push(RemoteOutput::Console(key, std::iter::once(dotnet).chain(args.iter().map(|a| file_name(a).to_string())).collect::<Vec<_>>().join(" ")));
        let out = self.events.sender();
        self.spawn(move |b| async move { stream_dotnet(&b, key, args, &out).await });
    }

    /// Builds the selected project on the remote machine, installs netcoredbg there if
    /// needed, and debugs it there.
    fn start_debug(&mut self) {
        if self.debugging.is_some() || self.status != Status::Connected {
            return;
        }
        let Some(project) = self.selected_target().filter(|p| !is_solution(p)) else {
            self.outputs.push(RemoteOutput::Notify(tr("Select a project in the Explorer to debug it").into(), false));
            return;
        };
        let key = self.next_key;
        self.next_key += 1;
        self.debugging = Some(key);
        let name = file_stem(&project).to_string();
        self.outputs.push(RemoteOutput::Console(key, trf("Debug {name}", &[("name", &name)])));
        let out = self.events.sender();
        self.spawn(move |b| async move {
            out.send(Event::CommandText(key, format!("{}\n", tr("Preparing the debugger on the remote machine…")), false));
            let exe = match b.ensure_netcoredbg().await {
                Ok(exe) => exe,
                Err(e) => return Event::DebugFailed(key, format!("{e:#}")),
            };
            match stream_dotnet(&b, key, vec!["build".into(), project.clone()], &out).await {
                Event::CommandDone(_, Ok(Some(0))) => {}
                Event::CommandDone(_, Err(e)) => return Event::DebugFailed(key, e),
                _ => return Event::DebugFailed(key, tr("The build failed").into()),
            }
            match b.launch_config(&project).await {
                Ok(launch) => Event::DebugReady(key, name, launch, exe),
                Err(e) => Event::DebugFailed(key, format!("{e:#}")),
            }
        });
    }

    /// The selected file when it is a project or solution.
    fn selected_target(&self) -> Option<String> {
        self.selected.clone().filter(|path| is_dotnet_target(path))
    }

    fn cancel_running(&mut self) {
        if let Some((_, _, Some(killer))) = &self.running {
            killer.kill();
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

    /// `dotnet run` on the remote machine (the selected project, or the one in the folder),
    /// in a terminal so its output streams and it reads input; closing the tab stops it.
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

/// `dotnet --list-sdks`.
fn sdk_list(b: &Backend) -> Operation {
    Operation::Spawn { program: b.dotnet.clone(), args: vec!["--list-sdks".into()], cwd: b.root.clone(), env: vec![("DOTNET_NOLOGO".into(), "1".into())], path: Vec::new() }
}

/// Runs `dotnet <args>` with its output streamed to console `key`.
async fn stream_dotnet(b: &Backend, key: u64, args: Vec<String>, out: &Outbox<Event>) -> Event {
    use tokio::io::AsyncReadExt;
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let mut process = match b.dotnet_stream(&args).await {
        Ok(process) => process,
        Err(e) => return Event::CommandDone(key, Err(format!("{e:#}"))),
    };
    out.send(Event::CommandStarted(key, process.killer()));
    let forward = |mut pipe: Box<dyn tokio::io::AsyncRead + Send + Unpin>, stderr: bool| {
        let out = out.clone();
        async move {
            let mut buffer = vec![0u8; 16 * 1024];
            while let Ok(count) = pipe.read(&mut buffer).await {
                if count == 0 {
                    return;
                }
                out.send(Event::CommandText(key, String::from_utf8_lossy(&buffer[..count]).into_owned(), stderr));
            }
        }
    };
    let (stdout, stderr) = (process.stdout.take().expect("stdout"), process.stderr.take().expect("stderr"));
    tokio::join!(forward(stdout, false), forward(stderr, true));
    Event::CommandDone(key, Ok(process.wait().await))
}

/// The solution of a folder (at its top, else one level down) or its projects.
fn find_solution(root: &Path) -> (Option<PathBuf>, Vec<PathBuf>) {
    let is_solution = |p: &Path| p.extension().is_some_and(|e| e == "sln" || e == "slnx");
    let mut dirs = vec![root.to_path_buf()];
    dirs.extend(rc_core::workspace::list_dir(root).into_iter().filter(|n| n.is_dir).map(|n| n.path));
    for dir in dirs {
        let mut found: Vec<PathBuf> = rc_core::workspace::list_dir(&dir).into_iter().map(|n| n.path).filter(|p| is_solution(p)).collect();
        found.sort();
        if let Some(sln) = found.into_iter().next() {
            return (Some(sln), Vec::new());
        }
    }
    (None, crate::app::find_projects(root, 3))
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

/// The connection worker: installs the agent when needed, connects, lists the folder and
/// probes the machine, then serves listings until the connection ends. Aborting it drops
/// the session.
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
    let connected = async {
        let entries = list(&client, &root).await?;
        let machine = remote_backend::probe(&client).await?;
        anyhow::Ok((entries, machine))
    };
    match connected.await {
        Ok((entries, machine)) => out.send(Event::Connected(root.clone(), entries, client.clone(), machine)),
        Err(error) => {
            out.send(Event::Lost(describe(&session, &error)));
            return;
        }
    };
    loop {
        tokio::select! {
            request = requests.recv() => match request {
                None | Some(Request::Shutdown) => return,
                Some(Request::List(path)) => {
                    let event = match list(&client, &path).await {
                        Ok(entries) => Event::Listed(path, entries),
                        Err(error) if is_connection_error(&error) => {
                            out.send(Event::Lost(describe(&session, &error)));
                            return;
                        }
                        Err(error) => Event::ListFailed(path, error.to_string()),
                    };
                    out.send(event);
                }
            },
            _ = client.closed() => {
                let tail = session.stderr_tail();
                out.send(Event::Lost(if tail.is_empty() { "remote agent disconnected".into() } else { format!("remote agent disconnected\n{tail}") }));
                return;
            }
        }
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

/// Arguments only, never a shell line: the agent starts `dotnet` directly.
fn dotnet_args(kind: CommandKind, target: Option<String>) -> Vec<String> {
    let mut args = match kind {
        CommandKind::Info => vec!["--info".to_string()],
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
        Some(2) => remote.refresh(),
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
            if kit::button(ui, Some(ico::DEBUG), tr("Debug"), remote.debugging.is_none()).clicked() {
                remote.start_debug();
            }
            if busy && kit::button(ui, Some(ico::STOP), tr("Stop"), remote.running.as_ref().is_some_and(|r| r.2.is_some())).clicked() {
                remote.cancel_running();
            }
            if busy || remote.debugging.is_some() {
                ui.spinner();
            }
        });
        let language = match &remote.language {
            Language::Preparing => Some((tr("Preparing C# support on the remote machine…").to_string(), false)),
            Language::Failed(error) => Some((trf("C# support is not available: {error}", &[("error", error)]), true)),
            Language::Idle | Language::Started => None,
        };
        kit::content(ui, |ui| {
            match &remote.sdk_versions {
                Some(versions) if versions.is_empty() => kit::message(ui, tr("No .NET SDK found on the remote machine"), false),
                Some(versions) => kit::note(ui, &format!("{} {}", tr(".NET SDKs:"), versions.join(", "))),
                None => {}
            }
            if let Some((text, error)) = &language {
                if *error {
                    kit::message(ui, text, true);
                } else {
                    kit::note(ui, text);
                }
            }
        });
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
    let mut open = None;
    egui::ScrollArea::both().id_salt("remote_tree").auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let expanded = remote.expanded.contains(&root);
        if tree_row(ui, 0, Some(expanded), ico::FOLDER, file_name(&root), &location, false).clicked() {
            toggle(remote, &root);
        }
        if expanded {
            tree(ui, remote, &root, 1, &mut open);
        }
    });
    if let Some(path) = open {
        // An open tab shows at once; the others are fetched fresh first.
        let local = app.remote.mirror().and_then(|m| m.local(&path));
        match local.filter(|l| app.docs.list.iter().any(|d| &d.path == l)) {
            Some(local) => app.open_file(&local),
            None => app.remote.open(&path, 0, 0),
        }
    }
    if run && let Some((program, args, name)) = app.remote.run_command() {
        app.open_remote_session(program, args, name, true);
    }
}

fn toggle(remote: &mut RemoteExplorer, dir: &str) {
    if !remote.expanded.remove(dir) {
        remote.expanded.insert(dir.to_string());
    }
}

/// The tree under `dir`; a double-clicked file (or a single click on a file) lands in `open`.
fn tree(ui: &mut Ui, remote: &mut RemoteExplorer, dir: &str, depth: usize, open: &mut Option<String>) {
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
            let expanded = remote.expanded.contains(&path);
            if tree_row(ui, depth, Some(expanded), ico::FOLDER, &entry.name, "", false).clicked() {
                toggle(remote, &path);
            }
            if expanded && depth < 64 {
                tree(ui, remote, &path, depth + 1, open);
            }
        } else {
            let selected = remote.selected.as_deref() == Some(path.as_str());
            let response = tree_row(ui, depth, None, file_icon(&entry.name), &entry.name, "", selected);
            if response.clicked() {
                remote.selected = Some(path.clone());
                // Projects and solutions are targets (Build, Debug): open them on double-click.
                if !is_dotnet_target(&path) {
                    *open = Some(path.clone());
                }
            }
            if response.double_clicked() {
                *open = Some(path);
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

/// The editor area while a remote folder is open and no file is: how to open one.
pub fn editor(_app: &mut DotCraftApp, ui: &mut Ui) {
    let p = rc_kit::pal(ui);
    let rect = ui.max_rect();
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, tr("Select a file in the Explorer to open it"), FontId::proportional(13.0), p.text_muted);
    ui.allocate_rect(rect, Sense::hover());
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

    pub fn select_for_test(&mut self, path: &str) {
        self.selected = Some(path.into());
    }

    /// The Explorer's Debug button on `project`.
    pub fn debug_for_test(&mut self, project: &str) {
        self.selected = Some(project.into());
        self.start_debug();
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

    pub fn preparing_language_for_test(&mut self) {
        self.language = Language::Preparing;
    }

    /// Whether C# support was handed to the app.
    pub fn language_started_for_test(&self) -> bool {
        self.language == Language::Started
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
        assert_eq!(dotnet_args(CommandKind::Info, Some("/repo/App.sln".into())), ["--info"]);
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
    fn run_uses_the_selected_project() {
        let mut remote = RemoteExplorer::new(Waker::noop(), RemoteConnection::default());
        remote.open_for_test(connection(), vec![("/srv/app", vec![("Api", true)])]);
        let (_, args, name) = remote.run_command().unwrap();
        assert_eq!(name, "app");
        assert!(args[3].contains("run") && !args[3].contains("--project"), "{}", args[3]);
        remote.select_for_test("/srv/app/Api/Api.csproj");
        let (_, args, name) = remote.run_command().unwrap();
        assert_eq!(name, "Api");
        assert!(args[3].contains("--project"), "{}", args[3]);
        let (_, _, title) = remote.terminal_command().unwrap();
        assert_eq!(title, "dev@build-box");
    }

    #[test]
    fn solutions_are_found_at_the_top_or_one_level_down() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src/App")).unwrap();
        std::fs::write(dir.path().join("src/App/App.csproj"), "<Project />").unwrap();
        assert_eq!(find_solution(dir.path()), (None, vec![dir.path().join("src/App/App.csproj")]));
        std::fs::write(dir.path().join("src/Shop.slnx"), "<Solution />").unwrap();
        assert_eq!(find_solution(dir.path()).0, Some(dir.path().join("src/Shop.slnx")));
        std::fs::write(dir.path().join("Top.sln"), "").unwrap();
        assert_eq!(find_solution(dir.path()).0, Some(dir.path().join("Top.sln")));
    }

    /// A dropped connection keeps the tree; a failed first connection goes back to the
    /// dialog, which offers the manual install when the agent could not be installed.
    #[test]
    fn lost_connections_keep_an_open_folder() {
        let mut remote = RemoteExplorer::new(Waker::noop(), RemoteConnection::default());
        remote.status = Status::Connecting;
        remote.events.sender().send(Event::Lost("Permission denied (publickey).".into()));
        remote.pump();
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
        remote.open_for_test(connection(), vec![("/srv", vec![("App", true)])]);
        remote.events.sender().send(Event::Lost("Broken pipe".into()));
        remote.pump();
        assert!(remote.is_open());
        assert_eq!(remote.status, Status::Lost);
        assert!(remote.cache.contains_key("/srv"));
    }
}
