//! A Debug Adapter Protocol client over a child process's stdio.
//!
//! Usable from the UI thread without blocking, like `rc_lsp::rpc`: every request returns a
//! [`Pending`] the UI polls each frame (the response wakes the UI through the [`Waker`]).
//! Events and reverse requests arrive as [`Incoming`] messages through an
//! `rc_core::inbox::Outbox`. The adapter runs in its own process group so
//! [`DapClient::kill`] takes the debuggee it launched down with it.

use crate::protocol::*;
use parking_lot::Mutex;
use rc_core::Waker;
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

/// A failed request: the adapter's message (`success: false`) or a transport problem.
#[derive(Clone, Debug, PartialEq)]
pub struct DapError {
    pub message: String,
    /// The `body.error` of the response, when the adapter sent one.
    pub details: Option<Value>,
}

impl DapError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        DapError { message: message.into(), details: None }
    }

    fn stopped() -> Self {
        DapError::new("the debug adapter stopped")
    }
}

impl std::fmt::Display for DapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for DapError {}

/// A message from the adapter that is not a response to one of our requests.
#[derive(Clone, Debug)]
pub enum Incoming {
    Event { event: String, body: Value },
    /// A reverse request (`runInTerminal`, `startDebugging`…): answer it with [`DapClient::reply`].
    Request { seq: i64, command: String, arguments: Value },
    /// A line the adapter wrote to stderr (its logs).
    Stderr(String),
    /// The adapter process exited (or its stdout closed).
    Exited(Option<i32>),
}

type Responder = oneshot::Sender<Result<Value, DapError>>;

struct Shared {
    pending: Mutex<HashMap<i64, Responder>>,
    out: mpsc::UnboundedSender<Vec<u8>>,
    seq: AtomicI64,
    waker: Waker,
}

impl Shared {
    fn next_seq(&self) -> i64 {
        self.seq.fetch_add(1, Ordering::Relaxed)
    }
}

/// Handle to a running adapter. Cheap to clone.
#[derive(Clone)]
pub struct DapClient {
    shared: Arc<Shared>,
    child: Arc<Mutex<Option<Child>>>,
    pid: u32,
}

/// The response of a request, polled without blocking. Dropping it discards the answer.
pub struct Pending<T> {
    rx: Option<oneshot::Receiver<Result<Value, DapError>>>,
    seq: i64,
    client: DapClient,
    pub started: Instant,
    parse: fn(Value) -> Result<T, DapError>,
}

impl<T> Pending<T> {
    /// The response if it arrived: `None` while waiting.
    pub fn poll(&mut self) -> Option<Result<T, DapError>> {
        let rx = self.rx.as_mut()?;
        match rx.try_recv() {
            Ok(r) => {
                self.rx = None;
                Some(r.and_then(self.parse))
            }
            Err(oneshot::error::TryRecvError::Empty) => None,
            Err(oneshot::error::TryRecvError::Closed) => {
                self.rx = None;
                Some(Err(DapError::stopped()))
            }
        }
    }

    /// Waits for the response (tests and background tasks only — never the UI thread).
    pub async fn wait(mut self) -> Result<T, DapError> {
        let rx = self.rx.take().expect("already taken");
        match rx.await {
            Ok(r) => r.and_then(self.parse),
            Err(_) => Err(DapError::stopped()),
        }
    }

    /// Asks the adapter to drop the request (`cancel`); the response is not delivered.
    pub fn cancel(mut self) {
        if self.rx.take().is_some() && self.client.forget(self.seq) {
            self.client.send_request_untracked("cancel", json!({"requestId": self.seq}));
        }
    }

    pub fn is_done(&self) -> bool {
        self.rx.is_none()
    }

    /// The request's `seq`.
    pub fn seq(&self) -> i64 {
        self.seq
    }
}

impl<T> Drop for Pending<T> {
    fn drop(&mut self) {
        if self.rx.is_some() {
            self.client.forget(self.seq);
        }
    }
}

fn parse<T: serde::de::DeserializeOwned>(v: Value) -> Result<T, DapError> {
    // Adapters answer body-less requests without `body`: treat it as an empty object.
    let v = if v.is_null() { json!({}) } else { v };
    serde_json::from_value(v).map_err(|e| DapError::new(format!("unexpected response: {e}")))
}

impl DapClient {
    /// Starts `program` and talks DAP over its stdin/stdout. Must be called inside the tokio
    /// runtime (`rc_core::Runtime::handle().enter()` or from a task). `waker` is called when
    /// a response arrives (events wake through `incoming`).
    pub fn spawn(program: &Path, args: &[String], cwd: Option<&Path>, env: &[(String, String)], incoming: Outbox<Incoming>, waker: Waker) -> std::io::Result<DapClient> {
        let mut cmd = Command::new(program);
        cmd.args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        if let Some(cwd) = cwd {
            cmd.current_dir(cwd);
        }
        for (k, v) in env {
            cmd.env(k, v);
        }
        #[cfg(unix)]
        cmd.process_group(0);
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        let mut child = cmd.spawn()?;
        let pid = child.id().unwrap_or(0);
        let stdin = child.stdin.take().expect("piped");
        let stdout = child.stdout.take().expect("piped");
        let stderr = child.stderr.take().expect("piped");
        let (out_tx, out_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let shared = Arc::new(Shared { pending: Mutex::new(HashMap::new()), out: out_tx, seq: AtomicI64::new(1), waker });
        tokio::spawn(write_loop(stdin, out_rx));
        let child = Arc::new(Mutex::new(Some(child)));
        let reader_shared = shared.clone();
        let reader_incoming = incoming.clone();
        let exit_child = child.clone();
        tokio::spawn(async move {
            read_loop(stdout, &reader_shared, &reader_incoming).await;
            for (_, tx) in reader_shared.pending.lock().drain() {
                let _ = tx.send(Err(DapError::stopped()));
            }
            // Give the process a moment to be reaped so the exit code is known.
            let mut code = None;
            for _ in 0..20 {
                let status = exit_child.lock().as_mut().map(|c| c.try_wait());
                match status {
                    Some(Ok(Some(s))) => {
                        code = s.code();
                        break;
                    }
                    Some(Ok(None)) => tokio::time::sleep(std::time::Duration::from_millis(25)).await,
                    _ => break,
                }
            }
            reader_incoming.send(Incoming::Exited(code));
        });
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                incoming.send(Incoming::Stderr(line));
            }
        });
        Ok(DapClient { shared, child, pid })
    }

    /// Process id of the adapter.
    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Sends a request; the response body is parsed with `serde` into `T`.
    pub fn request<T: serde::de::DeserializeOwned>(&self, command: &str, arguments: Value) -> Pending<T> {
        let seq = self.shared.next_seq();
        let (tx, rx) = oneshot::channel();
        self.shared.pending.lock().insert(seq, tx);
        self.write(json!({"seq": seq, "type": "request", "command": command, "arguments": arguments}));
        Pending { rx: Some(rx), seq, client: self.clone(), started: Instant::now(), parse: parse::<T> }
    }

    fn send_request_untracked(&self, command: &str, arguments: Value) {
        let seq = self.shared.next_seq();
        self.write(json!({"seq": seq, "type": "request", "command": command, "arguments": arguments}));
    }

    fn write(&self, msg: Value) {
        let _ = self.shared.out.send(frame(&msg.to_string()));
    }

    fn forget(&self, seq: i64) -> bool {
        self.shared.pending.lock().remove(&seq).is_some()
    }

    /// Answers a reverse request.
    pub fn reply(&self, request_seq: i64, command: &str, result: Result<Value, String>) {
        let seq = self.shared.next_seq();
        let msg = match result {
            Ok(body) => json!({"seq": seq, "type": "response", "request_seq": request_seq, "command": command, "success": true, "body": body}),
            Err(message) => json!({"seq": seq, "type": "response", "request_seq": request_seq, "command": command, "success": false, "message": message}),
        };
        self.write(msg);
    }

    /// Kills the adapter and everything in its process group (the debuggee it launched).
    pub fn kill(&self) {
        rc_dotnet::kill_tree(self.pid);
        if let Some(mut c) = self.child.lock().take() {
            let _ = c.start_kill();
        }
    }

    /// Whether the adapter process has exited.
    pub fn has_exited(&self) -> bool {
        match self.child.lock().as_mut() {
            Some(c) => !matches!(c.try_wait(), Ok(None)),
            None => true,
        }
    }

    pub fn pending_count(&self) -> usize {
        self.shared.pending.lock().len()
    }

    // ---- Typed requests ---------------------------------------------------------------------

    pub fn initialize(&self, args: Value) -> Pending<Capabilities> {
        self.request("initialize", args)
    }

    /// `launch` with adapter-specific arguments (see [`crate::launch::launch_arguments`]).
    pub fn launch(&self, args: Value) -> Pending<Value> {
        self.request("launch", args)
    }

    pub fn attach(&self, args: Value) -> Pending<Value> {
        self.request("attach", args)
    }

    pub fn set_breakpoints(&self, path: &Path, breakpoints: &[SourceBreakpoint]) -> Pending<SetBreakpointsResponse> {
        self.request("setBreakpoints", args::set_breakpoints(path, breakpoints))
    }

    pub fn set_exception_breakpoints(&self, filters: &[String]) -> Pending<Value> {
        self.request("setExceptionBreakpoints", json!({"filters": filters}))
    }

    pub fn configuration_done(&self) -> Pending<Value> {
        self.request("configurationDone", json!({}))
    }

    pub fn threads(&self) -> Pending<ThreadsResponse> {
        self.request("threads", json!({}))
    }

    /// Frames `start..start+levels` of a thread (`levels` 0 = all).
    pub fn stack_trace(&self, thread_id: i64, start: i64, levels: i64) -> Pending<StackTraceResponse> {
        self.request("stackTrace", args::stack_trace(thread_id, start, levels))
    }

    pub fn scopes(&self, frame_id: i64) -> Pending<ScopesResponse> {
        self.request("scopes", json!({"frameId": frame_id}))
    }

    /// Children of a variable; `start`/`count` page through indexed children.
    pub fn variables(&self, reference: i64, start: Option<i64>, count: Option<i64>) -> Pending<VariablesResponse> {
        self.request("variables", args::variables(reference, start, count))
    }

    pub fn evaluate(&self, expression: &str, frame_id: Option<i64>, context: EvalContext) -> Pending<EvaluateResponse> {
        self.request("evaluate", args::evaluate(expression, frame_id, context))
    }

    pub fn set_variable(&self, reference: i64, name: &str, value: &str) -> Pending<SetVariableResponse> {
        self.request("setVariable", json!({"variablesReference": reference, "name": name, "value": value}))
    }

    pub fn continue_(&self, thread_id: i64) -> Pending<Value> {
        self.request("continue", json!({"threadId": thread_id}))
    }

    pub fn next(&self, thread_id: i64) -> Pending<Value> {
        self.request("next", json!({"threadId": thread_id}))
    }

    pub fn step_in(&self, thread_id: i64) -> Pending<Value> {
        self.request("stepIn", json!({"threadId": thread_id}))
    }

    pub fn step_out(&self, thread_id: i64) -> Pending<Value> {
        self.request("stepOut", json!({"threadId": thread_id}))
    }

    pub fn pause(&self, thread_id: i64) -> Pending<Value> {
        self.request("pause", json!({"threadId": thread_id}))
    }

    pub fn disconnect(&self, terminate_debuggee: bool) -> Pending<Value> {
        self.request("disconnect", json!({"restart": false, "terminateDebuggee": terminate_debuggee}))
    }

    pub fn terminate(&self) -> Pending<Value> {
        self.request("terminate", json!({}))
    }

    pub fn exception_info(&self, thread_id: i64) -> Pending<ExceptionInfo> {
        self.request("exceptionInfo", json!({"threadId": thread_id}))
    }
}

/// Argument builders shared by the typed requests and the session.
pub(crate) mod args {
    use super::*;

    pub fn set_breakpoints(path: &Path, breakpoints: &[SourceBreakpoint]) -> Value {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
        json!({
            "source": {"name": name, "path": path},
            "breakpoints": breakpoints,
            "lines": breakpoints.iter().map(|b| b.line).collect::<Vec<_>>(),
            "sourceModified": false,
        })
    }

    pub fn stack_trace(thread_id: i64, start: i64, levels: i64) -> Value {
        json!({"threadId": thread_id, "startFrame": start, "levels": levels})
    }

    pub fn variables(reference: i64, start: Option<i64>, count: Option<i64>) -> Value {
        let mut a = json!({"variablesReference": reference});
        if let Some(s) = start {
            a["start"] = json!(s);
        }
        if let Some(c) = count {
            a["count"] = json!(c);
        }
        a
    }

    pub fn evaluate(expression: &str, frame_id: Option<i64>, context: EvalContext) -> Value {
        let mut a = json!({"expression": expression, "context": context.as_str()});
        if let Some(f) = frame_id {
            a["frameId"] = json!(f);
        }
        a
    }
}

async fn write_loop(mut stdin: ChildStdin, mut rx: mpsc::UnboundedReceiver<Vec<u8>>) {
    while let Some(msg) = rx.recv().await {
        if stdin.write_all(&msg).await.is_err() || stdin.flush().await.is_err() {
            return;
        }
    }
}

async fn read_loop(mut stdout: tokio::process::ChildStdout, shared: &Shared, incoming: &Outbox<Incoming>) {
    let mut decoder = FrameDecoder::new();
    let mut chunk = vec![0u8; 64 * 1024];
    loop {
        match stdout.read(&mut chunk).await {
            Ok(0) | Err(_) => return,
            Ok(n) => decoder.push(&chunk[..n]),
        }
        while let Some(msg) = decoder.next_message() {
            match msg {
                Ok(msg) => dispatch(msg, shared, incoming),
                Err(BadFrame(why)) => {
                    incoming.send(Incoming::Stderr(format!("invalid DAP message: {why}")));
                }
            }
        }
    }
}

fn dispatch(msg: Value, shared: &Shared, incoming: &Outbox<Incoming>) {
    match msg.get("type").and_then(Value::as_str) {
        Some("response") => {
            let Some(seq) = msg.get("request_seq").and_then(Value::as_i64) else { return };
            let Some(tx) = shared.pending.lock().remove(&seq) else { return };
            let ok = msg.get("success").and_then(Value::as_bool).unwrap_or(false);
            let result = if ok {
                Ok(msg.get("body").cloned().unwrap_or(Value::Null))
            } else {
                let details = msg.get("body").and_then(|b| b.get("error")).cloned();
                // The readable text is in `body.error.format` (netcoredbg puts it in `message`).
                let message = details
                    .as_ref()
                    .and_then(|d| d.get("format"))
                    .and_then(Value::as_str)
                    .or_else(|| msg.get("message").and_then(Value::as_str))
                    .unwrap_or("request failed")
                    .to_string();
                Err(DapError { message, details })
            };
            let _ = tx.send(result);
            shared.waker.wake();
        }
        Some("event") => {
            let event = msg.get("event").and_then(Value::as_str).unwrap_or("").to_string();
            incoming.send(Incoming::Event { event, body: msg.get("body").cloned().unwrap_or(Value::Null) });
        }
        Some("request") => {
            incoming.send(Incoming::Request {
                seq: msg.get("seq").and_then(Value::as_i64).unwrap_or(0),
                command: msg.get("command").and_then(Value::as_str).unwrap_or("").to_string(),
                arguments: msg.get("arguments").cloned().unwrap_or(Value::Null),
            });
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rc_core::{Inbox, Runtime};

    #[test]
    fn requests_get_responses_and_events_arrive() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("echo-adapter.py");
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
        if line.lower().startswith('content-length:'): n = int(line.split(':')[1])
    return json.loads(sys.stdin.buffer.read(n))
def send(obj):
    b = json.dumps(obj).encode()
    sys.stdout.buffer.write(b'Content-Length: %d\r\n\r\n' % len(b) + b); sys.stdout.buffer.flush()
seq = 100
while True:
    m = read()
    seq += 1
    if m.get('type') == 'response':
        send({'seq': seq, 'type': 'event', 'event': 'gotReply', 'body': m})
        continue
    send({'seq': seq, 'type': 'event', 'event': 'saw', 'body': {'command': m['command']}})
    if m['command'] == 'fail':
        send({'seq': seq, 'type': 'response', 'request_seq': m['seq'], 'command': 'fail', 'success': False, 'message': 'nope', 'body': {'error': {'id': 1, 'format': 'it failed'}}})
    elif m['command'] == 'ask':
        send({'seq': seq, 'type': 'request', 'command': 'runInTerminal', 'arguments': {'args': ['x']}})
        send({'seq': seq + 1, 'type': 'response', 'request_seq': m['seq'], 'command': 'ask', 'success': True})
    else:
        send({'seq': seq, 'type': 'response', 'request_seq': m['seq'], 'command': m['command'], 'success': True, 'body': {'threads': [{'id': 7, 'name': 'Main'}], 'echo': m['arguments']}})
"#,
        )
        .unwrap();
        let inbox: Inbox<Incoming> = Inbox::new(Waker::noop());
        let _guard = Runtime::handle().enter();
        let client = DapClient::spawn(Path::new("python3"), &["-u".into(), script.display().to_string()], Some(dir.path()), &[], inbox.sender(), Waker::noop()).unwrap();

        let threads = Runtime::block_on(client.threads().wait()).unwrap();
        assert_eq!(threads.threads, vec![Thread { id: 7, name: "Main".into() }]);
        let echo = Runtime::block_on(client.evaluate("a + b", Some(3), EvalContext::Watch).wait()).unwrap();
        assert_eq!(echo, EvaluateResponse::default());
        let raw: Value = Runtime::block_on(client.request::<Value>("echo", json!({"x": 1})).wait()).unwrap();
        assert_eq!(raw["echo"], json!({"x": 1}));
        let err = Runtime::block_on(client.request::<Value>("fail", json!({})).wait()).unwrap_err();
        assert_eq!(err.message, "it failed");
        // A body-less success parses into an empty object.
        Runtime::block_on(client.request::<Value>("ask", json!({})).wait()).unwrap();

        let mut saw = Vec::new();
        let mut reverse = None;
        while let Some(m) = inbox.recv_timeout(std::time::Duration::from_secs(5)) {
            match m {
                Incoming::Event { event, body } if event == "saw" => saw.push(body["command"].as_str().unwrap().to_string()),
                Incoming::Request { seq, command, .. } => {
                    reverse = Some(command.clone());
                    client.reply(seq, &command, Err("not supported".into()));
                }
                Incoming::Event { event, body } if event == "gotReply" => {
                    assert_eq!(body["success"], false);
                    assert_eq!(body["command"], "runInTerminal");
                    break;
                }
                _ => {}
            }
        }
        assert_eq!(saw, ["threads", "evaluate", "echo", "fail", "ask"]);
        assert_eq!(reverse.as_deref(), Some("runInTerminal"));
        assert_eq!(client.pending_count(), 0);
        client.kill();
    }
}
