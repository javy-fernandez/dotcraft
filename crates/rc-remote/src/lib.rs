//! Versioned JSON-line transport for a remote DotCraft workspace agent.
//! The agent runs with the remote user's filesystem and process permissions.

use anyhow::{Context, Result, bail};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    process::Stdio,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
pub mod deploy;

use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader},
    process::{Child, Command},
    sync::{Notify, mpsc, oneshot},
    task::JoinHandle,
};

pub const PROTOCOL_VERSION: u32 = 1;
const MAX_BYTES: usize = 1024 * 1024;
const MAX_LINE_BYTES: usize = 2 * 1024 * 1024;
/// Longest a process may run: a first restore and build of a large solution, or its tests.
pub const MAX_RUN_MS: u64 = 60 * 60 * 1000;

/// `~/x` is `$HOME/x` on the agent's machine, so settings can name `~/.dotnet/dotnet`.
fn expand_home(path: &str) -> String {
    match (path.strip_prefix("~/"), std::env::var("HOME")) {
        (Some(rest), Ok(home)) if !home.is_empty() => format!("{}/{rest}", home.trim_end_matches('/')),
        _ => path.to_string(),
    }
}

pub fn decode_data(data: &str) -> Result<Vec<u8>> {
    let bytes = STANDARD.decode(data).context("invalid base64 data")?;
    if bytes.len() > MAX_BYTES {
        bail!("data exceeds 1 MiB limit");
    }
    Ok(bytes)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Request {
    pub version: u32,
    pub id: u64,
    #[serde(flatten)]
    pub operation: Operation,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum Operation {
    Hello,
    List {
        path: String,
    },
    Read {
        path: String,
    },
    Write {
        path: String,
        data: String,
    },
    Run {
        program: String,
        args: Vec<String>,
        cwd: String,
        env: Vec<(String, String)>,
        timeout_ms: u64,
    },
    Cancel {
        target_id: u64,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Response {
    pub version: u32,
    pub id: u64,
    pub result: std::result::Result<Value, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Value {
    Hello {
        version: u32,
    },
    Entries {
        entries: Vec<Entry>,
    },
    Data {
        data: String,
    },
    Written,
    Exit {
        code: Option<i32>,
        stdout: String,
        stderr: String,
    },
    Cancelled,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    pub name: String,
    pub is_dir: bool,
}

/// SSH and WSL launchers keep the agent's stdin/stdout as the only protocol channel.
/// `agent` is a trusted executable path configured by the user on the remote host.
#[derive(Clone, Debug)]
pub enum Endpoint {
    Ssh { host: String, agent: String },
    Wsl { distribution: String, agent: String },
}

/// Quotes `word` as a single word for the POSIX shell that sshd runs the remote command
/// through. A leading `~/` stays unquoted so the remote shell expands it to the home folder;
/// everything else is single-quoted, so `$`, backticks, `;` and the like stay literal.
pub fn shell_word(word: &str) -> String {
    let (home, rest) = match word.strip_prefix("~/") {
        Some(rest) => ("~/", rest),
        None => ("", word),
    };
    format!("{home}'{}'", rest.replace('\'', "'\\''"))
}

/// Keeps Windows from flashing a console window for a process started by the GUI.
fn no_window(#[allow(unused_mut)] mut cmd: Command) -> Command {
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    cmd
}

/// A host, distribution or agent path must be one plain line: no control characters, and
/// no leading `-` that ssh or wsl.exe could read as an option.
fn plain_argument(value: &str) -> bool {
    !value.is_empty() && !value.starts_with('-') && !value.chars().any(char::is_control)
}

impl Endpoint {
    /// ssh (with its options and the host) or wsl.exe (with the distribution), ready for the
    /// remote command.
    fn launcher(&self) -> Result<Command> {
        match self {
            Endpoint::Ssh { host, .. } => {
                if !plain_argument(host) || host.chars().any(char::is_whitespace) {
                    bail!("invalid SSH host or agent");
                }
                let mut cmd = Command::new("ssh");
                // No prompts: DotCraft has no terminal to answer them, so password and
                // unknown-host-key prompts fail instead of hanging. Keys, agents, known_hosts
                // and host aliases all come from the user's own SSH configuration.
                cmd.args([
                    "-T",
                    "-o",
                    "BatchMode=yes",
                    "-o",
                    "ConnectTimeout=15",
                    "-o",
                    "ServerAliveInterval=15",
                    "-o",
                    "ServerAliveCountMax=3",
                    "--",
                    host,
                ]);
                Ok(cmd)
            }
            Endpoint::Wsl { distribution, .. } => {
                if !plain_argument(distribution) {
                    bail!("invalid WSL distribution or agent");
                }
                let mut cmd = Command::new("wsl.exe");
                cmd.args(["--distribution", distribution]);
                Ok(cmd)
            }
        }
    }

    pub fn command(&self) -> Result<Command> {
        let mut cmd = self.launcher()?;
        match self {
            Endpoint::Ssh { agent, .. } => {
                if !plain_argument(agent) {
                    bail!("invalid SSH host or agent");
                }
                cmd.arg(shell_word(agent));
            }
            Endpoint::Wsl { agent, .. } => {
                if !plain_argument(agent) {
                    bail!("invalid WSL distribution or agent");
                }
                // `--exec` runs the program without a shell, so `~` is never expanded:
                // start in the home folder and run the rest as a relative path instead.
                match agent.strip_prefix("~/") {
                    Some(rest) => cmd.args(["--cd", "~", "--exec", &format!("./{rest}")]),
                    None => cmd.args(["--exec", agent]),
                };
            }
        }
        Ok(no_window(cmd))
    }

    /// Runs a POSIX `sh` script on the remote machine (the agent path is not used): for
    /// installing the agent. Over SSH the script goes through the user's login shell first,
    /// quoted as one word.
    pub fn shell_command(&self, script: &str) -> Result<Command> {
        let mut cmd = self.launcher()?;
        match self {
            Endpoint::Ssh { .. } => cmd.arg(format!("sh -c {}", shell_word(script))),
            Endpoint::Wsl { .. } => cmd.args(["--exec", "sh", "-c", script]),
        };
        Ok(no_window(cmd))
    }

    pub fn spawn(&self) -> Result<Child> {
        let mut cmd = self.command()?;
        Ok(cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()?)
    }
}

/// Serve requests concurrently so a cancellation can interrupt a running process.
pub async fn serve<R, W>(read: R, mut write: W) -> Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut reader = BufReader::new(read);
    let (tx, mut rx) = mpsc::unbounded_channel::<(Response, bool)>();
    let mut running: HashMap<u64, JoinHandle<()>> = HashMap::new();
    let mut cancelled = HashSet::new();
    loop {
        tokio::select! {
            line = read_line_bounded(&mut reader, MAX_LINE_BYTES) => {
                let Some(line) = line? else { break };
                let req: Request = match serde_json::from_slice(&line) {
                    Ok(req) => req,
                    Err(error) => {
                        // Answer instead of staying silent, so a client waiting for this id
                        // (an unknown operation from a newer DotCraft, say) gets an error.
                        let id = serde_json::from_slice::<serde_json::Value>(&line)
                            .ok()
                            .and_then(|value| value.get("id")?.as_u64())
                            .unwrap_or(0);
                        tx.send((reply(id, Err(format!("invalid request: {error}"))), false))?;
                        continue;
                    }
                };
                if req.version != PROTOCOL_VERSION {
                    tx.send((reply(req.id, Err(format!("unsupported protocol version {}", req.version))), false))?;
                    continue;
                }
                if let Operation::Cancel { target_id } = &req.operation {
                    if let Some(task) = running.remove(target_id) {
                        task.abort();
                        cancelled.insert(*target_id);
                        tx.send((reply(*target_id, Ok(Value::Cancelled)), false))?;
                    }
                    tx.send((reply(req.id, Ok(Value::Cancelled)), false))?;
                    continue;
                }
                if running.contains_key(&req.id) {
                    tx.send((reply(req.id, Err("request id is already in use".into())), false))?;
                    continue;
                }
                let output = tx.clone();
                let task = tokio::spawn(async move {
                    let result = perform(req.operation).await.map_err(|e| e.to_string());
                    let _ = output.send((reply(req.id, result), true));
                });
                running.insert(req.id, task);
            }
            Some((response, completion)) = rx.recv() => {
                if cancelled.contains(&response.id) && !matches!(&response.result, Ok(Value::Cancelled)) {
                    continue;
                }
                if completion { running.remove(&response.id); }
                write.write_all(serde_json::to_string(&response)?.as_bytes()).await?;
                write.write_all(b"\n").await?;
                write.flush().await?;
            }
        }
    }
    for (_, task) in running {
        task.abort();
    }
    Ok(())
}

async fn read_line_bounded<R: AsyncRead + Unpin>(
    reader: &mut BufReader<R>,
    limit: usize,
) -> Result<Option<Vec<u8>>> {
    let mut line = Vec::new();
    loop {
        let chunk = reader.fill_buf().await?;
        if chunk.is_empty() {
            if line.is_empty() {
                return Ok(None);
            }
            bail!("truncated remote line");
        }
        let count = chunk
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(chunk.len(), |index| index + 1);
        if line.len() + count > limit {
            bail!("remote line exceeds the {} MiB line limit", limit / (1024 * 1024));
        }
        line.extend_from_slice(&chunk[..count]);
        reader.consume(count);
        if line.last() == Some(&b'\n') {
            return Ok(Some(line));
        }
    }
}

fn reply(id: u64, result: std::result::Result<Value, String>) -> Response {
    Response {
        version: PROTOCOL_VERSION,
        id,
        result,
    }
}

async fn perform(operation: Operation) -> Result<Value> {
    match operation {
        Operation::Hello => Ok(Value::Hello {
            version: PROTOCOL_VERSION,
        }),
        Operation::List { path } => {
            let mut dir = tokio::fs::read_dir(PathBuf::from(path)).await?;
            let mut entries = Vec::new();
            while let Some(item) = dir.next_entry().await? {
                entries.push(Entry {
                    name: item.file_name().to_string_lossy().into_owned(),
                    is_dir: item.file_type().await?.is_dir(),
                });
                if entries.len() > 10_000 {
                    bail!("directory has too many entries");
                }
            }
            entries.sort_by(|a, b| a.name.cmp(&b.name));
            Ok(Value::Entries { entries })
        }
        Operation::Read { path } => {
            let file = tokio::fs::File::open(PathBuf::from(path)).await?;
            let mut bytes = Vec::new();
            file.take(MAX_BYTES as u64 + 1)
                .read_to_end(&mut bytes)
                .await?;
            if bytes.len() > MAX_BYTES {
                bail!("file exceeds 1 MiB response limit");
            }
            Ok(Value::Data {
                data: STANDARD.encode(bytes),
            })
        }
        Operation::Write { path, data } => {
            let bytes = STANDARD.decode(data).context("invalid base64 data")?;
            if bytes.len() > MAX_BYTES {
                bail!("write exceeds 1 MiB limit");
            }
            tokio::fs::write(PathBuf::from(path), bytes).await?;
            Ok(Value::Written)
        }
        Operation::Run {
            program,
            args,
            cwd,
            env,
            timeout_ms,
        } => {
            if program.is_empty() || timeout_ms == 0 || timeout_ms > MAX_RUN_MS {
                bail!("invalid process request");
            }
            run_process(expand_home(&program), args, expand_home(&cwd), env, timeout_ms).await
        }
        Operation::Cancel { .. } => bail!("cancel is handled by the server"),
    }
}

/// Runs a process in its own process group. Dropping the future (a cancelled request), a
/// timeout or an output overflow kills the whole group, so `dotnet test` cannot leave a
/// testhost running on the remote machine.
async fn run_process(
    program: String,
    args: Vec<String>,
    cwd: String,
    env: Vec<(String, String)>,
    timeout_ms: u64,
) -> Result<Value> {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .current_dir(PathBuf::from(cwd))
        .envs(env)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    cmd.process_group(0);
    let mut child = no_window(cmd).spawn()?;
    let mut tree = ProcessTree {
        pid: child.id(),
        armed: true,
    };
    let overflow = Arc::new(Notify::new());
    let mut out = Capture::start(child.stdout.take().context("missing stdout")?, &overflow);
    let mut err = Capture::start(child.stderr.take().context("missing stderr")?, &overflow);
    let status = tokio::select! {
        status = child.wait() => status?,
        _ = overflow.notified() => bail!("process output exceeds 1 MiB limit"),
        _ = tokio::time::sleep(Duration::from_millis(timeout_ms)) => bail!("process timed out"),
    };
    // The process exited on its own: build servers and MSBuild nodes it started may live on
    // by design, and may keep its pipes open. Read what is left for a moment, then stop.
    tree.armed = false;
    let _ = tokio::time::timeout(Duration::from_millis(400), async {
        let _ = (&mut out.task).await;
        let _ = (&mut err.task).await;
    })
    .await;
    Ok(Value::Exit {
        code: status.code(),
        stdout: STANDARD.encode(out.take()?),
        stderr: STANDARD.encode(err.take()?),
    })
}

/// Kills the process group of a child when dropped while still armed.
struct ProcessTree {
    #[cfg_attr(not(unix), allow(dead_code))]
    pid: Option<u32>,
    armed: bool,
}

impl Drop for ProcessTree {
    fn drop(&mut self) {
        #[cfg(unix)]
        if self.armed
            && let Some(pid) = self.pid
        {
            // SAFETY: plain syscall; the group was created for this child, which has not
            // been reaped yet, so its id cannot have been reused.
            unsafe {
                libc::killpg(pid as libc::pid_t, libc::SIGKILL);
            }
        }
        // Elsewhere `kill_on_drop` stops the direct child.
    }
}

/// Reads one output stream into a buffer capped at `MAX_BYTES`, signalling `overflow`
/// when the process writes more.
struct Capture {
    data: Arc<Mutex<Vec<u8>>>,
    overflowed: Arc<AtomicBool>,
    task: JoinHandle<()>,
}

impl Capture {
    fn start<R: AsyncRead + Unpin + Send + 'static>(mut reader: R, overflow: &Arc<Notify>) -> Self {
        let data = Arc::new(Mutex::new(Vec::new()));
        let overflowed = Arc::new(AtomicBool::new(false));
        let (buffer, flag, overflow) = (data.clone(), overflowed.clone(), overflow.clone());
        let task = tokio::spawn(async move {
            let mut chunk = [0u8; 8192];
            while let Ok(count) = reader.read(&mut chunk).await {
                if count == 0 {
                    return;
                }
                let mut buffer = buffer.lock().unwrap_or_else(|e| e.into_inner());
                if buffer.len() + count > MAX_BYTES {
                    flag.store(true, Ordering::SeqCst);
                    overflow.notify_one();
                    return;
                }
                buffer.extend_from_slice(&chunk[..count]);
            }
        });
        Self {
            data,
            overflowed,
            task,
        }
    }

    fn take(&self) -> Result<Vec<u8>> {
        if self.overflowed.load(Ordering::SeqCst) {
            bail!("process output exceeds 1 MiB limit");
        }
        Ok(std::mem::take(
            &mut self.data.lock().unwrap_or_else(|e| e.into_inner()),
        ))
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Longest response line the client accepts: a process result carries two base64 streams
/// of up to 1 MiB each.
const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;

type Waiters = Arc<Mutex<Option<HashMap<u64, oneshot::Sender<Response>>>>>;

/// A client for the agent. Requests can run side by side (browse while a build runs): a
/// reader task hands each response to the request with its id. Clones share the connection.
#[derive(Clone)]
pub struct Client {
    inner: Arc<ClientInner>,
}

struct ClientInner {
    write: tokio::sync::Mutex<std::pin::Pin<Box<dyn AsyncWrite + Send>>>,
    /// `None` once the connection is gone: every waiting request then fails.
    waiters: Waiters,
    next_id: std::sync::atomic::AtomicU64,
    reader: JoinHandle<()>,
}

impl Drop for ClientInner {
    fn drop(&mut self) {
        self.reader.abort();
    }
}

/// Forgets a request whose caller stopped waiting, so its late response is dropped.
struct Waiting<'a> {
    waiters: &'a Waiters,
    id: u64,
}

impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        if let Some(map) = lock(self.waiters).as_mut() {
            map.remove(&self.id);
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

fn disconnected<E>(_: E) -> anyhow::Error {
    anyhow::anyhow!("remote agent disconnected")
}

fn outcome(response: Response) -> Result<Value> {
    response
        .result
        .map_err(|message| anyhow::Error::new(RemoteError(message)))
}

/// Routes responses to their requests until the stream ends or breaks.
async fn route_responses<R: AsyncRead + Unpin>(read: R, waiters: Waiters) {
    let mut read = BufReader::new(read);
    while let Ok(Some(line)) = read_line_bounded(&mut read, MAX_RESPONSE_BYTES).await {
        // A line that is not a response of this protocol means the stream is not ours.
        let Ok(response) = serde_json::from_slice::<Response>(&line) else {
            break;
        };
        if response.version != PROTOCOL_VERSION {
            break;
        }
        let waiter = lock(&waiters).as_mut().and_then(|map| map.remove(&response.id));
        if let Some(waiter) = waiter {
            let _ = waiter.send(response);
        }
    }
    lock(&waiters).take();
}

impl Client {
    /// Starts the response reader on the current tokio runtime.
    pub fn new<R, W>(read: R, write: W) -> Self
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Send + 'static,
    {
        let waiters: Waiters = Arc::new(Mutex::new(Some(HashMap::new())));
        let reader = tokio::spawn(route_responses(read, waiters.clone()));
        Self {
            inner: Arc::new(ClientInner {
                write: tokio::sync::Mutex::new(Box::pin(write)),
                waiters,
                next_id: std::sync::atomic::AtomicU64::new(1),
                reader,
            }),
        }
    }

    async fn send(&self, operation: Operation) -> Result<(u64, oneshot::Receiver<Response>)> {
        let id = self
            .inner
            .next_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        lock(&self.inner.waiters)
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("remote agent disconnected"))?
            .insert(id, tx);
        let mut line = serde_json::to_vec(&Request {
            version: PROTOCOL_VERSION,
            id,
            operation,
        })?;
        line.push(b'\n');
        let mut write = self.inner.write.lock().await;
        let sent = async {
            write.write_all(&line).await?;
            write.flush().await
        }
        .await;
        if sent.is_err() {
            if let Some(map) = lock(&self.inner.waiters).as_mut() {
                map.remove(&id);
            }
            bail!("remote agent disconnected");
        }
        Ok((id, rx))
    }

    pub async fn request(&self, operation: Operation) -> Result<Value> {
        let (id, rx) = self.send(operation).await?;
        let _waiting = Waiting {
            waiters: &self.inner.waiters,
            id,
        };
        outcome(rx.await.map_err(disconnected)?)
    }

    /// Like [`Client::request`], but firing `cancel` asks the agent to stop the request (and
    /// kill its processes). Returns `None` when it was cancelled; a request that finished
    /// before the cancellation reached the agent returns its result.
    pub async fn request_cancellable(
        &self,
        operation: Operation,
        cancel: oneshot::Receiver<()>,
    ) -> Result<Option<Value>> {
        let (id, mut rx) = self.send(operation).await?;
        let _waiting = Waiting {
            waiters: &self.inner.waiters,
            id,
        };
        tokio::select! {
            response = &mut rx => return outcome(response.map_err(disconnected)?).map(Some),
            fired = cancel => {
                if fired.is_err() {
                    // Nobody can cancel any more: just wait.
                    return outcome(rx.await.map_err(disconnected)?).map(Some);
                }
            }
        }
        // Nobody waits for the acknowledgement: the reader drops it.
        let (cancel_id, _) = self.send(Operation::Cancel { target_id: id }).await?;
        if let Some(map) = lock(&self.inner.waiters).as_mut() {
            map.remove(&cancel_id);
        }
        match rx.await.map_err(disconnected)? {
            Response {
                result: Ok(Value::Cancelled),
                ..
            } => Ok(None),
            response => outcome(response).map(Some),
        }
    }

    pub async fn handshake(&self) -> Result<()> {
        match self.request(Operation::Hello).await? {
            Value::Hello {
                version: PROTOCOL_VERSION,
            } => Ok(()),
            _ => bail!("remote agent protocol mismatch"),
        }
    }
}

/// An error the agent answered for one request. The connection itself is still usable;
/// any other error from [`Client`] means the connection is broken.
#[derive(Debug)]
pub struct RemoteError(pub String);

impl std::fmt::Display for RemoteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for RemoteError {}

/// Whether `error` came from the connection (agent gone, ssh dropped, stream out of sync)
/// rather than from a request the agent answered with an error.
pub fn is_connection_error(error: &anyhow::Error) -> bool {
    error.downcast_ref::<RemoteError>().is_none()
}

/// Bytes of the launcher's stderr kept to explain a failed or dropped connection.
const STDERR_TAIL_BYTES: usize = 4096;

/// Owns one SSH or WSL agent process and its checked protocol connection. Dropping it
/// kills the launcher, which closes the agent's stdin; the agent then stops its processes.
pub struct RemoteSession {
    pub client: Client,
    stderr: Arc<Mutex<Vec<u8>>>,
    stderr_task: Option<JoinHandle<()>>,
    _child: Child,
}

impl RemoteSession {
    pub async fn connect(endpoint: &Endpoint) -> Result<Self> {
        let mut child = endpoint.spawn().context("could not start the remote launcher")?;
        let stdout = child
            .stdout
            .take()
            .context("remote agent stdout is unavailable")?;
        let stdin = child
            .stdin
            .take()
            .context("remote agent stdin is unavailable")?;
        // Always drain stderr: ssh reports authentication and host key failures there, and
        // a full pipe would block it.
        let stderr = Arc::new(Mutex::new(Vec::new()));
        let mut stderr_task = child
            .stderr
            .take()
            .map(|pipe| tokio::spawn(keep_tail(pipe, stderr.clone())));
        let client = Client::new(stdout, stdin);
        let error = match tokio::time::timeout(Duration::from_secs(30), client.handshake()).await
        {
            Ok(Ok(())) => {
                return Ok(Self {
                    client,
                    stderr,
                    stderr_task,
                    _child: child,
                });
            }
            Ok(Err(error)) => error,
            Err(_) => anyhow::anyhow!("remote agent handshake timed out"),
        };
        let _ = child.start_kill();
        // ssh usually exits on its own here; give it a moment to finish its message.
        if let Some(task) = stderr_task.as_mut() {
            let _ = tokio::time::timeout(Duration::from_secs(1), task).await;
        }
        let detail = tail_text(&stderr);
        Err(if detail.is_empty() {
            error
        } else {
            anyhow::anyhow!("{error}\n{detail}")
        })
    }

    /// The last lines ssh or wsl.exe wrote to stderr, to explain a dropped connection.
    pub fn stderr_tail(&self) -> String {
        tail_text(&self.stderr)
    }
}

impl Drop for RemoteSession {
    fn drop(&mut self) {
        if let Some(task) = &self.stderr_task {
            task.abort();
        }
    }
}

async fn keep_tail<R: AsyncRead + Unpin>(mut pipe: R, tail: Arc<Mutex<Vec<u8>>>) {
    let mut chunk = [0u8; 1024];
    while let Ok(count) = pipe.read(&mut chunk).await {
        if count == 0 {
            return;
        }
        let mut tail = tail.lock().unwrap_or_else(|e| e.into_inner());
        tail.extend_from_slice(&chunk[..count]);
        let excess = tail.len().saturating_sub(STDERR_TAIL_BYTES);
        tail.drain(..excess);
    }
}

fn tail_text(tail: &Mutex<Vec<u8>>) -> String {
    String::from_utf8_lossy(&tail.lock().unwrap_or_else(|e| e.into_inner()))
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[tokio::test]
    async fn agent_round_trips_files_and_processes() {
        let dir = tempfile::tempdir().unwrap();
        let (client, server) = tokio::io::duplex(64 * 1024);
        let (client_read, client_write) = tokio::io::split(client);
        let (server_read, server_write) = tokio::io::split(server);
        let task = tokio::spawn(serve(server_read, server_write));
        let client = Client::new(client_read, client_write);
        client.handshake().await.unwrap();
        let path = dir.path().join("example.txt");
        client
            .request(Operation::Write {
                path: path.display().to_string(),
                data: STANDARD.encode("hello"),
            })
            .await
            .unwrap();
        let Value::Data { data } = client
            .request(Operation::Read {
                path: path.display().to_string(),
            })
            .await
            .unwrap()
        else {
            panic!("read response")
        };
        assert_eq!(STANDARD.decode(data).unwrap(), b"hello");
        let Value::Entries { entries } = client
            .request(Operation::List {
                path: dir.path().display().to_string(),
            })
            .await
            .unwrap()
        else {
            panic!("list response")
        };
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            ["example.txt"]
        );
        let Value::Exit { code, stdout, .. } = client
            .request(Operation::Run {
                program: "sh".into(),
                args: vec!["-c".into(), "printf remote".into()],
                cwd: dir.path().display().to_string(),
                env: vec![],
                timeout_ms: 5_000,
            })
            .await
            .unwrap()
        else {
            panic!("run response")
        };
        assert_eq!(code, Some(0));
        assert_eq!(STANDARD.decode(stdout).unwrap(), b"remote");
        drop(client);
        task.await.unwrap().unwrap();
    }

    #[test]
    fn ssh_launcher_quotes_agent_path() {
        let command = Endpoint::Ssh {
            host: "example.org".into(),
            agent: "/home/a b/agent'v1".into(),
        }
        .command()
        .unwrap();
        let args: Vec<_> = command
            .as_std()
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args[0], "-T");
        assert!(args.contains(&"BatchMode=yes".to_string()));
        assert_eq!(
            args[args.len() - 3..],
            ["--", "example.org", "'/home/a b/agent'\\''v1'"]
        );
        for (host, agent) in [
            ("-oProxyCommand=evil", "agent"),
            ("bad host", "agent"),
            ("host\n", "agent"),
            ("", "agent"),
            ("host", ""),
            ("host", "-agent"),
            ("host", "agent\nrm -rf ~"),
        ] {
            let endpoint = Endpoint::Ssh {
                host: host.into(),
                agent: agent.into(),
            };
            assert!(endpoint.command().is_err(), "{host:?} {agent:?}");
        }
    }

    /// The quoted word goes through a real POSIX shell and must come back unchanged.
    #[cfg(unix)]
    #[test]
    fn shell_word_survives_a_real_shell() {
        for word in [
            "plain",
            "/home/a b/agent",
            "it's",
            "$(touch /tmp/dotcraft-pwned)",
            "`id`",
            "a;b && c | d > e",
            "\"double\" \\back\\slash",
            "*?[glob]",
            "${HOME}",
            "naïve/日本",
            "'",
            "",
        ] {
            let out = std::process::Command::new("sh")
                .args(["-c", &format!("printf %s {}", shell_word(word))])
                .output()
                .unwrap();
            assert_eq!(String::from_utf8(out.stdout).unwrap(), word);
        }
        let out = std::process::Command::new("sh")
            .args(["-c", &format!("printf %s {}", shell_word("~/bin/it's agent"))])
            .output()
            .unwrap();
        let home = std::env::var("HOME").unwrap();
        assert_eq!(
            String::from_utf8(out.stdout).unwrap(),
            format!("{home}/bin/it's agent")
        );
    }

    #[test]
    fn wsl_launcher_starts_home_relative_agents_in_the_home_folder() {
        let command = Endpoint::Wsl {
            distribution: "Ubuntu".into(),
            agent: "~/.cargo/bin/dotcraft-remote-agent".into(),
        }
        .command()
        .unwrap();
        let args: Vec<_> = command
            .as_std()
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            [
                "--distribution",
                "Ubuntu",
                "--cd",
                "~",
                "--exec",
                "./.cargo/bin/dotcraft-remote-agent"
            ]
        );
        assert!(
            Endpoint::Wsl {
                distribution: "--help".into(),
                agent: "agent".into()
            }
            .command()
            .is_err()
        );
    }

    #[test]
    fn wsl_launcher_keeps_distribution_and_program_separate() {
        let command = Endpoint::Wsl {
            distribution: "Ubuntu Dev".into(),
            agent: "/opt/dotcraft/bin/dotcraft-remote-agent".into(),
        }
        .command()
        .unwrap();
        let args: Vec<_> = command
            .as_std()
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            [
                "--distribution",
                "Ubuntu Dev",
                "--exec",
                "/opt/dotcraft/bin/dotcraft-remote-agent"
            ]
        );
    }

    #[tokio::test]
    async fn version_mismatch_is_rejected() {
        let (client, server) = tokio::io::duplex(4096);
        let (mut client_read, mut client_write) = tokio::io::split(client);
        let (server_read, server_write) = tokio::io::split(server);
        let task = tokio::spawn(serve(server_read, server_write));
        client_write
            .write_all(b"{\"version\":99,\"id\":7,\"operation\":\"hello\"}\n")
            .await
            .unwrap();
        let line = BufReader::new(&mut client_read)
            .lines()
            .next_line()
            .await
            .unwrap()
            .unwrap();
        let response: Response = serde_json::from_str(&line).unwrap();
        assert_eq!(response.id, 7);
        assert!(response.result.is_err());
        drop(client_read);
        drop(client_write);
        task.await.unwrap().unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn running_process_can_be_cancelled() {
        let (client, server) = tokio::io::duplex(4096);
        let (client_read, mut client_write) = tokio::io::split(client);
        let (server_read, server_write) = tokio::io::split(server);
        let task = tokio::spawn(serve(server_read, server_write));
        let mut lines = BufReader::new(client_read).lines();
        let run = Request {
            version: PROTOCOL_VERSION,
            id: 1,
            operation: Operation::Run {
                program: "sleep".into(),
                args: vec!["10".into()],
                cwd: std::env::temp_dir().display().to_string(),
                env: vec![],
                timeout_ms: 20_000,
            },
        };
        let cancel = Request {
            version: PROTOCOL_VERSION,
            id: 2,
            operation: Operation::Cancel { target_id: 1 },
        };
        for request in [run, cancel] {
            client_write
                .write_all(serde_json::to_string(&request).unwrap().as_bytes())
                .await
                .unwrap();
            client_write.write_all(b"\n").await.unwrap();
        }
        let replies = tokio::time::timeout(Duration::from_secs(2), async {
            let first: Response =
                serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
            let second: Response =
                serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
            [first, second]
        })
        .await
        .unwrap();
        assert!(
            replies
                .iter()
                .any(|reply| reply.id == 1 && matches!(&reply.result, Ok(Value::Cancelled)))
        );
        assert!(replies.iter().any(|reply| reply.id == 2));
        drop(client_write);
        drop(lines);
        task.await.unwrap().unwrap();
    }
}

#[cfg(test)]
mod limits_tests {
    use super::*;

    #[tokio::test]
    async fn rejects_oversized_request_before_parsing() {
        let (mut writer, reader) = tokio::io::duplex(MAX_LINE_BYTES + 100);
        writer
            .write_all(&vec![b'x'; MAX_LINE_BYTES + 1])
            .await
            .unwrap();
        let error = read_line_bounded(&mut BufReader::new(reader), MAX_LINE_BYTES)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("line limit"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn oversized_process_output_fails_before_timeout() {
        let start = std::time::Instant::now();
        let error = perform(Operation::Run {
            program: "python3".into(),
            args: vec![
                "-c".into(),
                "import sys; sys.stdout.buffer.write(b'x' * 1048577)".into(),
            ],
            cwd: std::env::temp_dir().display().to_string(),
            env: vec![],
            timeout_ms: 5_000,
        })
        .await
        .unwrap_err();
        assert!(error.to_string().contains("output exceeds"));
        assert!(start.elapsed() < Duration::from_secs(4));
    }
}

#[cfg(test)]
mod framing_tests {
    use super::*;
    use tokio::io::{DuplexStream, Lines, ReadHalf, WriteHalf};

    type Wire = (
        Lines<BufReader<ReadHalf<DuplexStream>>>,
        WriteHalf<DuplexStream>,
        JoinHandle<Result<()>>,
    );

    fn agent() -> Wire {
        let (client, server) = tokio::io::duplex(64 * 1024);
        let (read, write) = tokio::io::split(client);
        let (server_read, server_write) = tokio::io::split(server);
        let task = tokio::spawn(serve(server_read, server_write));
        (BufReader::new(read).lines(), write, task)
    }

    async fn response(lines: &mut Lines<BufReader<ReadHalf<DuplexStream>>>) -> Response {
        let line = tokio::time::timeout(Duration::from_secs(5), lines.next_line())
            .await
            .expect("the agent did not answer")
            .unwrap()
            .expect("the agent closed the stream");
        serde_json::from_str(&line).unwrap()
    }

    #[tokio::test]
    async fn requests_batched_in_one_write_are_all_answered() {
        let (mut lines, mut write, task) = agent();
        write
            .write_all(
                b"{\"version\":1,\"id\":1,\"operation\":\"hello\"}\n\
                  {\"version\":1,\"id\":2,\"operation\":\"hello\"}\r\n",
            )
            .await
            .unwrap();
        let mut ids = vec![response(&mut lines).await.id, response(&mut lines).await.id];
        ids.sort();
        assert_eq!(ids, [1, 2]);
        write.shutdown().await.unwrap();
        task.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn a_request_split_across_writes_is_reassembled() {
        let (mut lines, mut write, task) = agent();
        for byte in b"{\"version\":1,\"id\":3,\"operation\":\"hello\"}\n" {
            write.write_all(&[*byte]).await.unwrap();
            write.flush().await.unwrap();
        }
        let reply = response(&mut lines).await;
        assert_eq!(reply.id, 3);
        assert!(matches!(reply.result, Ok(Value::Hello { version: 1 })));
        write.shutdown().await.unwrap();
        task.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn malformed_and_unknown_requests_get_an_error_and_the_agent_keeps_serving() {
        let (mut lines, mut write, task) = agent();
        write.write_all(b"not json\n").await.unwrap();
        let reply = response(&mut lines).await;
        assert_eq!(reply.id, 0);
        assert!(reply.result.is_err());
        write
            .write_all(b"{\"version\":1,\"id\":9,\"operation\":\"teleport\"}\n")
            .await
            .unwrap();
        let reply = response(&mut lines).await;
        assert_eq!(reply.id, 9);
        assert!(reply.result.unwrap_err().contains("invalid request"));
        write
            .write_all(b"{\"version\":1,\"id\":10,\"operation\":\"hello\"}\n")
            .await
            .unwrap();
        assert!(response(&mut lines).await.result.is_ok());
        write.shutdown().await.unwrap();
        task.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn a_request_cut_off_by_the_end_of_the_stream_is_an_error() {
        let (_lines, mut write, task) = agent();
        write
            .write_all(b"{\"version\":1,\"id\":1,\"operation\":")
            .await
            .unwrap();
        write.shutdown().await.unwrap();
        let error = task.await.unwrap().unwrap_err();
        assert!(error.to_string().contains("truncated"));
    }

    #[tokio::test]
    async fn agent_errors_and_connection_errors_are_told_apart() {
        let (client, server) = tokio::io::duplex(64 * 1024);
        let (read, write) = tokio::io::split(client);
        let (server_read, server_write) = tokio::io::split(server);
        let task = tokio::spawn(serve(server_read, server_write));
        let client = Client::new(read, write);
        let missing = std::env::temp_dir().join("dotcraft-remote-missing/file.txt");
        let error = client
            .request(Operation::Read {
                path: missing.display().to_string(),
            })
            .await
            .unwrap_err();
        assert!(!is_connection_error(&error));
        task.abort();
        let _ = task.await;
        let error = client.request(Operation::Hello).await.unwrap_err();
        assert!(is_connection_error(&error));
    }
}

#[cfg(test)]
mod session_tests {
    use super::*;

    /// Needs the `ssh` client: a failed connection reports what ssh said, quickly.
    #[ignore]
    #[tokio::test]
    async fn a_failed_ssh_connection_explains_itself() {
        let start = std::time::Instant::now();
        let Err(error) = RemoteSession::connect(&Endpoint::Ssh {
            host: "dotcraft-remote-test.invalid".into(),
            agent: "dotcraft-remote-agent".into(),
        })
        .await
        else {
            panic!("connected to an invalid host")
        };
        let text = error.to_string();
        assert!(text.contains("dotcraft-remote-test.invalid"), "{text}");
        assert!(start.elapsed() < Duration::from_secs(20));
    }
}

#[cfg(all(test, unix))]
mod home_tests {
    use super::*;

    #[tokio::test]
    async fn programs_and_folders_may_start_in_the_home_folder() {
        let home = std::env::var("HOME").unwrap();
        assert_eq!(expand_home("~/.dotnet/dotnet"), format!("{}/.dotnet/dotnet", home.trim_end_matches('/')));
        assert_eq!(expand_home("/usr/bin/dotnet"), "/usr/bin/dotnet");
        assert_eq!(expand_home("dotnet"), "dotnet");
        assert_eq!(expand_home("~user/x"), "~user/x");
        let Value::Exit { stdout, .. } = perform(Operation::Run {
            program: "pwd".into(),
            args: vec![],
            cwd: "~/".into(),
            env: vec![],
            timeout_ms: 5_000,
        })
        .await
        .unwrap() else {
            panic!("run response")
        };
        let pwd = String::from_utf8(STANDARD.decode(stdout).unwrap()).unwrap();
        let canonical = |p: &str| std::fs::canonicalize(p.trim()).unwrap();
        assert_eq!(canonical(&pwd), canonical(&home));
    }
}

#[cfg(all(test, unix))]
mod process_tree_tests {
    use super::*;

    fn alive(pid: i32) -> bool {
        // SAFETY: signal 0 only checks that the process exists.
        unsafe { libc::kill(pid, 0) == 0 }
    }

    async fn wait_gone(pid: i32) -> bool {
        for _ in 0..40 {
            if !alive(pid) {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        false
    }

    async fn read_pid(path: &std::path::Path) -> i32 {
        for _ in 0..40 {
            if let Ok(text) = std::fs::read_to_string(path)
                && let Ok(pid) = text.trim().parse()
            {
                return pid;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("the script did not write its pid")
    }

    fn sleeper(dir: &std::path::Path, timeout_ms: u64) -> Operation {
        Operation::Run {
            program: "sh".into(),
            args: vec!["-c".into(), "sleep 30 & echo $! > pid; wait".into()],
            cwd: dir.display().to_string(),
            env: vec![],
            timeout_ms,
        }
    }

    #[tokio::test]
    async fn cancelling_a_run_kills_its_grandchildren() {
        let dir = tempfile::tempdir().unwrap();
        let task = tokio::spawn(perform(sleeper(dir.path(), 20_000)));
        let pid = read_pid(&dir.path().join("pid")).await;
        assert!(alive(pid));
        task.abort();
        assert!(wait_gone(pid).await, "the grandchild survived the cancel");
    }

    #[tokio::test]
    async fn a_timeout_kills_its_grandchildren() {
        let dir = tempfile::tempdir().unwrap();
        let task = tokio::spawn(perform(sleeper(dir.path(), 1_500)));
        let pid = read_pid(&dir.path().join("pid")).await;
        let error = task.await.unwrap().unwrap_err();
        assert!(error.to_string().contains("timed out"));
        assert!(wait_gone(pid).await, "the grandchild survived the timeout");
    }

    /// Like an MSBuild node, a background process keeps the pipes open after the command
    /// exits: the run still finishes as soon as the command does, and leaves it running.
    #[tokio::test]
    async fn background_processes_holding_the_pipes_do_not_stall_the_run() {
        let start = std::time::Instant::now();
        let Value::Exit { code, stdout, .. } = perform(Operation::Run {
            program: "sh".into(),
            args: vec!["-c".into(), "sleep 30 & echo $!".into()],
            cwd: std::env::temp_dir().display().to_string(),
            env: vec![],
            timeout_ms: 20_000,
        })
        .await
        .unwrap() else {
            panic!("run response")
        };
        assert!(start.elapsed() < Duration::from_secs(3));
        assert_eq!(code, Some(0));
        let pid: i32 = String::from_utf8(STANDARD.decode(stdout).unwrap())
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert!(alive(pid));
        // SAFETY: the test's own background process.
        unsafe { libc::kill(pid, libc::SIGKILL) };
    }
}
