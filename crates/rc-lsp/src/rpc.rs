//! JSON-RPC 2.0 over a child process's stdio with LSP's `Content-Length` framing.
//!
//! The client is usable from the UI thread without blocking: [`Client::request`] returns a
//! [`Pending`] the UI polls every frame (the response wakes it), dropping or cancelling it
//! sends `$/cancelRequest`. Server notifications and server→client requests arrive as
//! [`Incoming`] messages through an `rc_core::Outbox`.

use parking_lot::Mutex;
use rc_core::inbox::Outbox;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Instant;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{mpsc, oneshot};

#[derive(Clone, Debug, PartialEq)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
}

impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.message, self.code)
    }
}

impl std::error::Error for RpcError {}

/// `RequestCancelled` / `ContentModified`: the answer was not needed anymore.
pub const REQUEST_CANCELLED: i64 = -32800;
pub const CONTENT_MODIFIED: i64 = -32801;

/// A message from the server that is not a response to one of our requests.
#[derive(Clone, Debug)]
pub enum Incoming {
    Notification { method: String, params: Value },
    /// A request from the server: answer it with [`Client::reply`].
    Request { id: Value, method: String, params: Value },
    /// A line the server wrote to stderr (logs).
    Stderr(String),
    /// The server process exited (or its pipes closed).
    Exited(Option<i32>),
}

type Responder = oneshot::Sender<Result<Value, RpcError>>;

struct Shared {
    pending: Mutex<HashMap<i64, Responder>>,
    out: mpsc::UnboundedSender<String>,
    next_id: AtomicI64,
}

/// Handle to a running server. Cheap to clone.
#[derive(Clone)]
pub struct Client {
    shared: Arc<Shared>,
    child: Arc<Mutex<Option<Child>>>,
}

/// The response of a request, polled without blocking.
pub struct Pending<T> {
    rx: Option<oneshot::Receiver<Result<Value, RpcError>>>,
    id: i64,
    client: Client,
    pub started: Instant,
    parse: fn(Value) -> Result<T, RpcError>,
}

impl<T> Pending<T> {
    /// The response if it arrived: `None` while waiting.
    pub fn poll(&mut self) -> Option<Result<T, RpcError>> {
        let rx = self.rx.as_mut()?;
        match rx.try_recv() {
            Ok(r) => {
                self.rx = None;
                Some(r.and_then(self.parse))
            }
            Err(oneshot::error::TryRecvError::Empty) => None,
            Err(oneshot::error::TryRecvError::Closed) => {
                self.rx = None;
                Some(Err(RpcError { code: -1, message: "the language server stopped".into() }))
            }
        }
    }

    /// Waits for the response (tests and background tasks only — never the UI thread).
    pub async fn wait(mut self) -> Result<T, RpcError> {
        let rx = self.rx.take().expect("already taken");
        match rx.await {
            Ok(r) => r.and_then(self.parse),
            Err(_) => Err(RpcError { code: -1, message: "the language server stopped".into() }),
        }
    }

    /// Asks the server to stop working on it; the response will not be delivered.
    pub fn cancel(mut self) {
        if self.rx.take().is_some() {
            self.client.cancel(self.id);
        }
    }

    pub fn is_done(&self) -> bool {
        self.rx.is_none()
    }

    pub fn id(&self) -> i64 {
        self.id
    }
}

impl<T> Drop for Pending<T> {
    fn drop(&mut self) {
        if self.rx.is_some() {
            self.client.cancel(self.id);
        }
    }
}

/// Frames a JSON body with its `Content-Length` header.
pub fn frame(body: &str) -> String {
    format!("Content-Length: {}\r\n\r\n{body}", body.len())
}

impl Client {
    /// Starts `program` and talks LSP over its stdin/stdout. Must be called inside the tokio
    /// runtime (`rc_core::Runtime::handle().enter()` or from a task).
    pub fn spawn(program: &Path, args: &[String], cwd: &Path, env: &[(String, String)], incoming: Outbox<Incoming>) -> std::io::Result<Client> {
        let mut cmd = Command::new(program);
        cmd.args(args)
            .current_dir(cwd)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        for (k, v) in env {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn()?;
        let stdin = child.stdin.take().expect("piped");
        let stdout = child.stdout.take().expect("piped");
        let stderr = child.stderr.take().expect("piped");
        let (out_tx, out_rx) = mpsc::unbounded_channel::<String>();
        let shared = Arc::new(Shared { pending: Mutex::new(HashMap::new()), out: out_tx, next_id: AtomicI64::new(1) });
        tokio::spawn(write_loop(stdin, out_rx));
        let reader_shared = shared.clone();
        let reader_incoming = incoming.clone();
        let child = Arc::new(Mutex::new(Some(child)));
        let exit_child = child.clone();
        tokio::spawn(async move {
            read_loop(stdout, reader_shared.clone(), reader_incoming.clone()).await;
            // Pipes closed: fail every pending request and report the exit code.
            for (_, tx) in reader_shared.pending.lock().drain() {
                let _ = tx.send(Err(RpcError { code: -1, message: "the language server stopped".into() }));
            }
            let code = exit_child.lock().as_mut().and_then(|c| c.try_wait().ok().flatten()).and_then(|s| s.code());
            reader_incoming.send(Incoming::Exited(code));
        });
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                incoming.send(Incoming::Stderr(line));
            }
        });
        Ok(Client { shared, child })
    }

    /// Sends a request; the response is parsed with `serde` into `T`.
    pub fn request_raw<T: serde::de::DeserializeOwned>(&self, method: &str, params: Value) -> Pending<T> {
        let id = self.shared.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.shared.pending.lock().insert(id, tx);
        let body = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string();
        let _ = self.shared.out.send(frame(&body));
        fn parse<T: serde::de::DeserializeOwned>(v: Value) -> Result<T, RpcError> {
            serde_json::from_value(v).map_err(|e| RpcError { code: -2, message: format!("unexpected response: {e}") })
        }
        Pending { rx: Some(rx), id, client: self.clone(), started: Instant::now(), parse: parse::<T> }
    }

    /// A typed LSP request.
    pub fn request<R: lsp_types::request::Request>(&self, params: R::Params) -> Pending<R::Result>
    where
        R::Result: serde::de::DeserializeOwned,
    {
        self.request_raw(R::METHOD, serde_json::to_value(params).unwrap_or(Value::Null))
    }

    pub fn notify_raw(&self, method: &str, params: Value) {
        let body = json!({"jsonrpc": "2.0", "method": method, "params": params}).to_string();
        let _ = self.shared.out.send(frame(&body));
    }

    pub fn notify<N: lsp_types::notification::Notification>(&self, params: N::Params) {
        self.notify_raw(N::METHOD, serde_json::to_value(params).unwrap_or(Value::Null));
    }

    /// Answers a server→client request.
    pub fn reply(&self, id: Value, result: Result<Value, RpcError>) {
        let body = match result {
            Ok(v) => json!({"jsonrpc": "2.0", "id": id, "result": v}),
            Err(e) => json!({"jsonrpc": "2.0", "id": id, "error": {"code": e.code, "message": e.message}}),
        };
        let _ = self.shared.out.send(frame(&body.to_string()));
    }

    fn cancel(&self, id: i64) {
        if self.shared.pending.lock().remove(&id).is_some() {
            self.notify_raw("$/cancelRequest", json!({"id": id}));
        }
    }

    /// Kills the server process.
    pub fn kill(&self) {
        if let Some(mut c) = self.child.lock().take() {
            let _ = c.start_kill();
        }
    }

    pub fn pending_count(&self) -> usize {
        self.shared.pending.lock().len()
    }
}

async fn write_loop(mut stdin: ChildStdin, mut rx: mpsc::UnboundedReceiver<String>) {
    while let Some(msg) = rx.recv().await {
        if stdin.write_all(msg.as_bytes()).await.is_err() || stdin.flush().await.is_err() {
            return;
        }
    }
}

/// Reads framed messages until the pipe closes.
async fn read_loop(stdout: tokio::process::ChildStdout, shared: Arc<Shared>, incoming: Outbox<Incoming>) {
    let mut reader = BufReader::new(stdout);
    loop {
        let mut len: Option<usize> = None;
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line).await {
                Ok(0) | Err(_) => return,
                Ok(_) => {}
            }
            let line = line.trim_end();
            if line.is_empty() {
                break;
            }
            if let Some(v) = line.strip_prefix("Content-Length:") {
                len = v.trim().parse().ok();
            }
        }
        let Some(len) = len else { continue };
        let mut body = vec![0u8; len];
        if reader.read_exact(&mut body).await.is_err() {
            return;
        }
        let Ok(msg) = serde_json::from_slice::<Value>(&body) else { continue };
        dispatch(msg, &shared, &incoming);
    }
}

fn dispatch(msg: Value, shared: &Shared, incoming: &Outbox<Incoming>) {
    let method = msg.get("method").and_then(Value::as_str).map(str::to_string);
    let id = msg.get("id").cloned();
    match (method, id) {
        (Some(method), Some(id)) => {
            incoming.send(Incoming::Request { id, method, params: msg.get("params").cloned().unwrap_or(Value::Null) });
        }
        (Some(method), None) => {
            incoming.send(Incoming::Notification { method, params: msg.get("params").cloned().unwrap_or(Value::Null) });
        }
        (None, Some(id)) => {
            let Some(id) = id.as_i64() else { return };
            let Some(tx) = shared.pending.lock().remove(&id) else { return };
            let result = if let Some(err) = msg.get("error") {
                Err(RpcError {
                    code: err.get("code").and_then(Value::as_i64).unwrap_or(-1),
                    message: err.get("message").and_then(Value::as_str).unwrap_or("").to_string(),
                })
            } else {
                Ok(msg.get("result").cloned().unwrap_or(Value::Null))
            };
            let _ = tx.send(result);
        }
        (None, None) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rc_core::{Inbox, Runtime, Waker};

    /// A tiny "server": a shell script that answers one request with the same id.
    #[test]
    fn requests_get_their_responses_and_notifications_arrive() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("echo-server.py");
        std::fs::write(
            &script,
            r#"import sys, json
def read():
    n = 0
    while True:
        line = sys.stdin.buffer.readline()
        if not line: sys.exit(0)
        line = line.decode().strip()
        if not line: break
        if line.startswith('Content-Length:'): n = int(line.split(':')[1])
    return json.loads(sys.stdin.buffer.read(n))
def send(obj):
    b = json.dumps(obj).encode()
    sys.stdout.buffer.write(b'Content-Length: %d\r\n\r\n' % len(b) + b); sys.stdout.buffer.flush()
while True:
    m = read()
    if 'id' in m:
        send({'jsonrpc': '2.0', 'method': 'log', 'params': {'saw': m['method']}})
        send({'jsonrpc': '2.0', 'id': m['id'], 'result': {'echo': m['params']}})
"#,
        )
        .unwrap();
        let inbox: Inbox<Incoming> = Inbox::new(Waker::noop());
        let out = inbox.sender();
        let _guard = Runtime::handle().enter();
        let client = Client::spawn(Path::new("python3"), &["-u".into(), script.display().to_string()], dir.path(), &[], out).unwrap();
        let pending = client.request_raw::<Value>("test/echo", json!({"x": 1}));
        let result = Runtime::block_on(pending.wait()).unwrap();
        assert_eq!(result, json!({"echo": {"x": 1}}));
        let note = inbox.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        assert!(matches!(note, Incoming::Notification { method, .. } if method == "log"));
        client.kill();
    }
}
