//! A language-server session owned by the UI thread: starts the server, runs the handshake,
//! keeps documents in sync, answers routine server requests and turns notifications into
//! [`LspEvent`]s. [`LanguageSession::pump`] is called once per frame and never blocks.

use crate::roslyn::{self, ServerCommand};
use crate::rpc::{Client, Incoming, Pending, RpcError};
use crate::sync::{OpenDocs, changes_for, path_to_uri, uri_to_path};
use lsp_types::{Diagnostic, InitializeResult, ServerCapabilities};
use rc_core::{Inbox, Waker};
use rc_text::Edit;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

#[derive(Clone, Debug, PartialEq)]
pub enum ServerState {
    /// The process is starting and `initialize` is in flight.
    Starting,
    /// Projects are loading (Roslyn's design-time builds).
    LoadingProjects,
    Ready,
    Failed(String),
    Stopped,
}

/// Something the UI should react to.
#[derive(Clone, Debug)]
pub enum LspEvent {
    StateChanged(ServerState),
    Diagnostics { path: PathBuf, version: Option<i32>, diagnostics: Vec<Diagnostic> },
    /// A background task's progress (title, message, percentage); `None` when it ended.
    Progress { token: String, title: String, message: Option<String>, percentage: Option<u32>, done: bool },
    /// `workspace/applyEdit`: apply it and answer with [`LanguageSession::reply`].
    ApplyEdit { id: Value, edit: lsp_types::WorkspaceEdit },
    /// The server asked the client to refresh semantic tokens / inlay hints / diagnostics.
    Refresh(&'static str),
    Log(String),
    ShowMessage { error: bool, text: String },
}

pub struct LanguageSession {
    pub client: Client,
    inbox: Inbox<Incoming>,
    pub state: ServerState,
    pub capabilities: Option<ServerCapabilities>,
    pub docs: OpenDocs,
    pub semantic_legend: Vec<String>,
    init: Option<Pending<InitializeResult>>,
    root: PathBuf,
    solution: Option<PathBuf>,
    projects: Vec<PathBuf>,
    progress: HashMap<String, String>,
    pub started: Instant,
    /// Recent stderr lines, for the error message when the server dies.
    stderr: Vec<String>,
}

impl LanguageSession {
    /// Starts the server for `root`, then opens `solution` (or `projects`) once initialized.
    pub fn start(cmd: &ServerCommand, root: &Path, solution: Option<&Path>, projects: Vec<PathBuf>, dotnet_root: Option<&Path>, waker: Waker) -> std::io::Result<Self> {
        let inbox = Inbox::new(waker);
        let _rt = rc_core::Runtime::handle().enter();
        let client = roslyn::start(cmd, root, dotnet_root, inbox.sender())?;
        let init = roslyn::initialize(&client, root);
        Ok(LanguageSession {
            client,
            inbox,
            state: ServerState::Starting,
            capabilities: None,
            docs: OpenDocs::default(),
            semantic_legend: Vec::new(),
            init: Some(init),
            root: root.to_path_buf(),
            solution: solution.map(Path::to_path_buf),
            projects,
            progress: HashMap::new(),
            started: Instant::now(),
            stderr: Vec::new(),
        })
    }

    pub fn is_ready(&self) -> bool {
        self.state == ServerState::Ready
    }

    /// Whether requests can be sent (the server is initialized, projects may still load).
    pub fn is_initialized(&self) -> bool {
        matches!(self.state, ServerState::Ready | ServerState::LoadingProjects)
    }

    /// Handles everything that arrived; returns the events for the UI.
    pub fn pump(&mut self) -> Vec<LspEvent> {
        let mut events = Vec::new();
        if let Some(init) = &mut self.init
            && let Some(result) = init.poll()
        {
            self.init = None;
            match result {
                Ok(res) => {
                    if let Some(st) = &res.capabilities.semantic_tokens_provider {
                        let legend = match st {
                            lsp_types::SemanticTokensServerCapabilities::SemanticTokensOptions(o) => &o.legend,
                            lsp_types::SemanticTokensServerCapabilities::SemanticTokensRegistrationOptions(o) => &o.semantic_tokens_options.legend,
                        };
                        self.semantic_legend = legend.token_types.iter().map(|t| t.as_str().to_string()).collect();
                    }
                    self.capabilities = Some(res.capabilities);
                    self.client.notify_raw("initialized", json!({}));
                    roslyn::open_solution(&self.client, self.solution.as_deref(), &self.projects);
                    self.set_state(ServerState::LoadingProjects, &mut events);
                }
                Err(e) => self.set_state(ServerState::Failed(e.to_string()), &mut events),
            }
        }
        for msg in self.inbox.drain() {
            match msg {
                Incoming::Request { id, method, params } => {
                    if let Some(answer) = roslyn::default_reply(&method, &params) {
                        if method == "workspace/semanticTokens/refresh" {
                            events.push(LspEvent::Refresh("semanticTokens"));
                        } else if method == "workspace/inlayHint/refresh" {
                            events.push(LspEvent::Refresh("inlayHint"));
                        } else if method == "workspace/diagnostic/refresh" {
                            events.push(LspEvent::Refresh("diagnostic"));
                        }
                        self.client.reply(id, Ok(answer));
                    } else if method == "workspace/applyEdit" {
                        match serde_json::from_value::<lsp_types::ApplyWorkspaceEditParams>(params) {
                            Ok(p) => events.push(LspEvent::ApplyEdit { id, edit: p.edit }),
                            Err(e) => self.client.reply(id, Err(RpcError { code: -32602, message: e.to_string() })),
                        }
                    } else {
                        self.client.reply(id, Err(RpcError { code: -32601, message: format!("{method} is not supported by DotCraft") }));
                    }
                }
                Incoming::Notification { method, params } => self.notification(&method, params, &mut events),
                Incoming::Stderr(line) => {
                    self.stderr.push(line.clone());
                    if self.stderr.len() > 50 {
                        self.stderr.remove(0);
                    }
                    events.push(LspEvent::Log(line));
                }
                Incoming::Exited(code) => {
                    if !matches!(self.state, ServerState::Stopped) {
                        let tail = self.stderr.iter().rev().take(3).rev().cloned().collect::<Vec<_>>().join("\n");
                        let why = format!("exit code {}{}", code.map_or_else(|| "?".to_string(), |c| c.to_string()), if tail.is_empty() { String::new() } else { format!(": {tail}") });
                        self.set_state(ServerState::Failed(why), &mut events);
                    }
                }
            }
        }
        events
    }

    fn set_state(&mut self, s: ServerState, events: &mut Vec<LspEvent>) {
        if self.state != s {
            self.state = s.clone();
            events.push(LspEvent::StateChanged(s));
        }
    }

    fn notification(&mut self, method: &str, params: Value, events: &mut Vec<LspEvent>) {
        match method {
            "textDocument/publishDiagnostics" => {
                if let Ok(p) = serde_json::from_value::<lsp_types::PublishDiagnosticsParams>(params)
                    && let Some(path) = uri_to_path(&p.uri)
                {
                    events.push(LspEvent::Diagnostics { path, version: p.version, diagnostics: p.diagnostics });
                }
            }
            "$/progress" => {
                let token = match params.get("token") {
                    Some(Value::String(s)) => s.clone(),
                    Some(v) => v.to_string(),
                    None => return,
                };
                let value = params.get("value").cloned().unwrap_or(Value::Null);
                let kind = value.get("kind").and_then(Value::as_str).unwrap_or("");
                let message = value.get("message").and_then(Value::as_str).map(str::to_string);
                let percentage = value.get("percentage").and_then(Value::as_u64).map(|p| p as u32);
                let title = match kind {
                    "begin" => {
                        let t = value.get("title").and_then(Value::as_str).unwrap_or("").to_string();
                        self.progress.insert(token.clone(), t.clone());
                        t
                    }
                    _ => self.progress.get(&token).cloned().unwrap_or_default(),
                };
                let done = kind == "end";
                if done {
                    self.progress.remove(&token);
                }
                events.push(LspEvent::Progress { token, title, message, percentage, done });
            }
            // Roslyn: every project of the solution finished loading.
            "workspace/projectInitializationComplete" => self.set_state(ServerState::Ready, events),
            "window/logMessage" => {
                if let Some(m) = params.get("message").and_then(Value::as_str) {
                    events.push(LspEvent::Log(m.to_string()));
                }
            }
            "window/showMessage" => {
                let error = params.get("type").and_then(Value::as_u64).is_some_and(|t| t == 1);
                if let Some(m) = params.get("message").and_then(Value::as_str) {
                    events.push(LspEvent::ShowMessage { error, text: m.to_string() });
                }
            }
            _ => {}
        }
    }

    /// Answers a request the UI handled (e.g. `workspace/applyEdit`).
    pub fn reply(&self, id: Value, result: Value) {
        self.client.reply(id, Ok(result));
    }

    // ------------------------------------------------------------------ documents

    pub fn did_open(&mut self, path: &Path, language: &str, text: &str) {
        if self.docs.is_open(path) {
            return;
        }
        let version = self.docs.open(path);
        self.client.notify_raw(
            "textDocument/didOpen",
            json!({ "textDocument": { "uri": path_to_uri(path).as_str(), "languageId": language, "version": version, "text": text } }),
        );
    }

    /// Sends the edits made since the last call, incrementally.
    pub fn did_change(&mut self, path: &Path, edits: &[Edit]) {
        if edits.is_empty() {
            return;
        }
        let Some(version) = self.docs.bump(path) else { return };
        let changes = changes_for(edits);
        self.client.notify_raw(
            "textDocument/didChange",
            json!({ "textDocument": { "uri": path_to_uri(path).as_str(), "version": version }, "contentChanges": changes }),
        );
    }

    /// Replaces the whole text (after a reload from disk).
    pub fn did_change_full(&mut self, path: &Path, text: &str) {
        let Some(version) = self.docs.bump(path) else { return };
        self.client.notify_raw(
            "textDocument/didChange",
            json!({ "textDocument": { "uri": path_to_uri(path).as_str(), "version": version }, "contentChanges": [{ "text": text }] }),
        );
    }

    pub fn did_save(&mut self, path: &Path) {
        if self.docs.is_open(path) {
            self.client.notify_raw("textDocument/didSave", json!({ "textDocument": { "uri": path_to_uri(path).as_str() } }));
        }
    }

    pub fn did_close(&mut self, path: &Path) {
        if self.docs.close(path) {
            self.client.notify_raw("textDocument/didClose", json!({ "textDocument": { "uri": path_to_uri(path).as_str() } }));
        }
    }

    /// Files created/changed/deleted on disk (so the server reloads closed files).
    pub fn did_change_watched(&self, changes: &[(PathBuf, u8)]) {
        let list: Vec<Value> = changes.iter().map(|(p, t)| json!({ "uri": path_to_uri(p).as_str(), "type": t })).collect();
        self.client.notify_raw("workspace/didChangeWatchedFiles", json!({ "changes": list }));
    }

    /// Stops the server politely (`shutdown` + `exit`), then kills it.
    pub fn stop(&mut self) {
        self.state = ServerState::Stopped;
        let client = self.client.clone();
        let pending = client.request_raw::<Value>("shutdown", Value::Null);
        rc_core::Runtime::spawn(async move {
            let _ = tokio::time::timeout(std::time::Duration::from_secs(2), pending.wait()).await;
            client.notify_raw("exit", Value::Null);
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            client.kill();
        });
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}

impl Drop for LanguageSession {
    fn drop(&mut self) {
        if self.state != ServerState::Stopped {
            self.client.kill();
        }
    }
}
