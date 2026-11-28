//! Process streams: a process on the agent's machine whose stdin, stdout and stderr travel
//! over the one agent connection, next to every other request. The agent side is
//! [`run`]; the client side is [`RemoteProcess`], whose pipes are ordinary tokio
//! `AsyncRead`/`AsyncWrite`s, so a language server or a debug adapter can talk over them as
//! if it were a local child process.

use crate::{Client, Operation, ProcessTree, Response, Value, expand_home, no_window, reply};
use anyhow::Context;
use base64::{Engine, engine::general_purpose::STANDARD};
use std::{
    pin::Pin,
    process::Stdio,
    task::{Context as TaskContext, Poll},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf},
    process::Command,
    sync::{mpsc, oneshot},
    task::JoinHandle,
};

/// Largest output chunk per answer.
const CHUNK: usize = 32 * 1024;

pub(crate) struct Spec {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: String,
    pub env: Vec<(String, String)>,
    pub path: Vec<String>,
}

/// Aborts the output readers when the process task ends or is cancelled.
struct Readers(Vec<JoinHandle<()>>);

impl Drop for Readers {
    fn drop(&mut self) {
        for task in &self.0 {
            task.abort();
        }
    }
}

/// Runs a spawned process for request `id`: answers `started`, its output, then `exited`
/// (the last answer, marked complete). Aborting the task kills the process group.
pub(crate) async fn run(id: u64, spec: Spec, mut input: mpsc::UnboundedReceiver<Option<Vec<u8>>>, out: mpsc::UnboundedSender<(Response, bool)>) {
    let result = async {
        let program = expand_home(&spec.program);
        let mut cmd = Command::new(&program);
        cmd.args(spec.args.iter().map(|a| expand_home(a)))
            .current_dir(expand_home(&spec.cwd))
            .envs(spec.env.iter().map(|(k, v)| (k.clone(), expand_home(v))))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if !spec.path.is_empty() {
            let current = std::env::var_os("PATH").unwrap_or_default();
            let dirs = spec.path.iter().map(|d| std::path::PathBuf::from(expand_home(d))).chain(std::env::split_paths(&current));
            if let Ok(joined) = std::env::join_paths(dirs) {
                cmd.env("PATH", joined);
            }
        }
        #[cfg(unix)]
        cmd.process_group(0);
        let mut child = no_window(cmd).spawn().with_context(|| format!("could not start {program}"))?;
        let _tree = ProcessTree { pid: child.id(), armed: true };
        let _ = out.send((reply(id, Ok(Value::Started { pid: child.id() })), false));
        let mut stdin = child.stdin.take().context("missing stdin")?;
        let writer = tokio::spawn(async move {
            while let Some(Some(bytes)) = input.recv().await {
                if stdin.write_all(&bytes).await.is_err() || stdin.flush().await.is_err() {
                    break;
                }
            }
            // Dropping stdin closes it.
        });
        let mut readers = Readers(vec![writer]);
        let pipe = |fd: u8, mut pipe: Box<dyn AsyncRead + Send + Unpin>| {
            let out = out.clone();
            tokio::spawn(async move {
                let mut buffer = vec![0u8; CHUNK];
                while let Ok(count) = pipe.read(&mut buffer).await {
                    if count == 0 {
                        return;
                    }
                    let data = STANDARD.encode(&buffer[..count]);
                    if out.send((reply(id, Ok(Value::Output { fd, data })), false)).is_err() {
                        return;
                    }
                }
            })
        };
        let stdout = pipe(1, Box::new(child.stdout.take().context("missing stdout")?));
        let stderr = pipe(2, Box::new(child.stderr.take().context("missing stderr")?));
        let status = child.wait().await?;
        // Like `run`: background processes may keep the pipes open; read what is left for a
        // moment, then stop.
        let _ = tokio::time::timeout(Duration::from_millis(400), async {
            let _ = stdout.await;
            let _ = stderr.await;
        })
        .await;
        readers.0.clear();
        Ok::<_, anyhow::Error>(Value::Exited { code: status.code() })
    }
    .await;
    let _ = out.send((reply(id, result.map_err(|e| format!("{e:#}"))), true));
}

/// Reads the chunks a channel delivers.
pub(crate) struct ChunkReader {
    rx: mpsc::UnboundedReceiver<Vec<u8>>,
    chunk: Vec<u8>,
    at: usize,
}

impl AsyncRead for ChunkReader {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>, buf: &mut ReadBuf<'_>) -> Poll<std::io::Result<()>> {
        while self.at >= self.chunk.len() {
            match self.rx.poll_recv(cx) {
                Poll::Ready(Some(chunk)) => {
                    self.chunk = chunk;
                    self.at = 0;
                }
                // End of the stream: EOF.
                Poll::Ready(None) => return Poll::Ready(Ok(())),
                Poll::Pending => return Poll::Pending,
            }
        }
        let count = buf.remaining().min(self.chunk.len() - self.at);
        let at = self.at;
        buf.put_slice(&self.chunk[at..at + count]);
        self.at += count;
        Poll::Ready(Ok(()))
    }
}

/// Writes go to the remote process's stdin; shutting down (or dropping) closes it.
pub struct RemoteStdin {
    tx: Option<mpsc::UnboundedSender<Option<Vec<u8>>>>,
}

impl AsyncWrite for RemoteStdin {
    fn poll_write(self: Pin<&mut Self>, _: &mut TaskContext<'_>, buf: &[u8]) -> Poll<std::io::Result<usize>> {
        match &self.tx {
            Some(tx) if tx.send(Some(buf.to_vec())).is_ok() => Poll::Ready(Ok(buf.len())),
            _ => Poll::Ready(Err(std::io::ErrorKind::BrokenPipe.into())),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, _: &mut TaskContext<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, _: &mut TaskContext<'_>) -> Poll<std::io::Result<()>> {
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(None);
        }
        Poll::Ready(Ok(()))
    }
}

impl Drop for RemoteStdin {
    fn drop(&mut self) {
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(None);
        }
    }
}

/// Kills a remote process (and its process group); cheap to clone, usable from any thread.
#[derive(Clone)]
pub struct Killer {
    client: Client,
    id: u64,
    runtime: tokio::runtime::Handle,
}

impl Killer {
    pub fn kill(&self) {
        let (client, id) = (self.client.clone(), self.id);
        self.runtime.spawn(async move {
            let _ = client.send_only(Operation::Cancel { target_id: id }).await;
        });
    }
}

/// A process running on the remote machine, from [`Client::spawn`]. Dropping it kills it.
pub struct RemoteProcess {
    pub pid: Option<u32>,
    pub stdin: Option<RemoteStdin>,
    pub stdout: Option<Box<dyn AsyncRead + Send + Unpin>>,
    pub stderr: Option<Box<dyn AsyncRead + Send + Unpin>>,
    exit: Option<oneshot::Receiver<Option<i32>>>,
    killer: Killer,
    pump: JoinHandle<()>,
}

impl RemoteProcess {
    pub(crate) fn new(client: Client, id: u64, pid: Option<u32>, mut answers: mpsc::UnboundedReceiver<Response>) -> Self {
        let runtime = tokio::runtime::Handle::current();
        let (out_tx, out_rx) = mpsc::unbounded_channel();
        let (err_tx, err_rx) = mpsc::unbounded_channel();
        let (exit_tx, exit_rx) = oneshot::channel();
        let pump = tokio::spawn(async move {
            let mut code = None;
            while let Some(answer) = answers.recv().await {
                match answer.result {
                    Ok(Value::Output { fd, data }) => {
                        let Ok(bytes) = STANDARD.decode(data) else { continue };
                        let _ = if fd == 2 { err_tx.send(bytes) } else { out_tx.send(bytes) };
                    }
                    Ok(Value::Exited { code: exit }) => {
                        code = exit;
                        break;
                    }
                    _ => break,
                }
            }
            // Dropping the senders ends both readers.
            drop((out_tx, err_tx));
            let _ = exit_tx.send(code);
        });
        let (in_tx, mut in_rx) = mpsc::unbounded_channel::<Option<Vec<u8>>>();
        let input_client = client.clone();
        tokio::spawn(async move {
            while let Some(message) = in_rx.recv().await {
                let operation = match message {
                    Some(bytes) => Operation::Input { target_id: id, data: STANDARD.encode(bytes) },
                    None => Operation::CloseInput { target_id: id },
                };
                let close = matches!(operation, Operation::CloseInput { .. });
                if input_client.send_only(operation).await.is_err() || close {
                    return;
                }
            }
        });
        RemoteProcess {
            pid,
            stdin: Some(RemoteStdin { tx: Some(in_tx) }),
            stdout: Some(Box::new(ChunkReader { rx: out_rx, chunk: Vec::new(), at: 0 })),
            stderr: Some(Box::new(ChunkReader { rx: err_rx, chunk: Vec::new(), at: 0 })),
            exit: Some(exit_rx),
            killer: Killer { client, id, runtime },
            pump,
        }
    }

    pub fn killer(&self) -> Killer {
        self.killer.clone()
    }

    /// Resolves with the exit code once the process ended (`None` when it was killed or the
    /// connection broke). Can be taken once.
    pub fn take_exit(&mut self) -> Option<oneshot::Receiver<Option<i32>>> {
        self.exit.take()
    }

    /// Waits for the end; `None` as for [`RemoteProcess::take_exit`].
    pub async fn wait(&mut self) -> Option<i32> {
        match self.exit.take() {
            Some(exit) => exit.await.ok().flatten(),
            None => None,
        }
    }

    /// Lets the process run on after this handle is dropped (its pipes still work).
    pub fn detach(mut self) {
        self.pump = tokio::spawn(async {});
        self.killer.id = 0;
    }
}

impl Drop for RemoteProcess {
    fn drop(&mut self) {
        if self.killer.id != 0 && !self.pump.is_finished() {
            self.killer.kill();
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use crate::{Client, Operation, serve};
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

    fn agent() -> (Client, tokio::task::JoinHandle<anyhow::Result<()>>) {
        let (client, server) = tokio::io::duplex(64 * 1024);
        let (read, write) = tokio::io::split(client);
        let (server_read, server_write) = tokio::io::split(server);
        let task = tokio::spawn(serve(server_read, server_write));
        (Client::new(read, write), task)
    }

    fn spawn(program: &str, args: &[&str]) -> Operation {
        Operation::Spawn {
            program: program.into(),
            args: args.iter().map(|a| a.to_string()).collect(),
            cwd: std::env::temp_dir().display().to_string(),
            env: vec![("DOTCRAFT_TEST".into(), "yes".into())],
            path: vec![],
        }
    }

    /// stdin in, stdout and stderr out, the exit code at the end; a large output arrives
    /// whole, and other requests keep working meanwhile.
    #[tokio::test]
    async fn a_spawned_process_streams_both_ways() {
        let (client, _task) = agent();
        client.handshake().await.unwrap();
        let mut cat = client.spawn(spawn("sh", &["-c", "cat; echo \"$DOTCRAFT_TEST\" >&2; exit 3"])).await.unwrap();
        let mut stdin = cat.stdin.take().unwrap();
        let payload: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
        let writer = {
            let payload = payload.clone();
            tokio::spawn(async move {
                for chunk in payload.chunks(10_000) {
                    stdin.write_all(chunk).await.unwrap();
                }
                stdin.shutdown().await.unwrap();
            })
        };
        client.request(Operation::Hello).await.unwrap();
        let mut out = Vec::new();
        cat.stdout.take().unwrap().read_to_end(&mut out).await.unwrap();
        writer.await.unwrap();
        let mut err = String::new();
        cat.stderr.take().unwrap().read_to_string(&mut err).await.unwrap();
        assert_eq!(out.len(), payload.len());
        assert_eq!(out, payload);
        assert_eq!(err, "yes\n");
        assert_eq!(cat.wait().await, Some(3));
    }

    /// Killing ends the process and its children; the connection stays usable.
    #[tokio::test]
    async fn killing_a_stream_ends_it() {
        let (client, _task) = agent();
        let mut sleeper = client.spawn(spawn("sh", &["-c", "echo ready; sleep 30"])).await.unwrap();
        let mut lines = BufReader::new(sleeper.stdout.take().unwrap()).lines();
        assert_eq!(lines.next_line().await.unwrap().as_deref(), Some("ready"));
        sleeper.killer().kill();
        let code = tokio::time::timeout(std::time::Duration::from_secs(5), sleeper.wait()).await.unwrap();
        assert_eq!(code, None);
        assert_eq!(lines.next_line().await.unwrap(), None);
        client.request(Operation::Hello).await.unwrap();
        // A program that does not exist is an answer, not a broken connection.
        let error = client.spawn(spawn("/nonexistent/dotcraft-tool", &[])).await.err().unwrap();
        assert!(!crate::is_connection_error(&error), "{error:#}");
    }

    #[tokio::test]
    async fn home_and_path_are_expanded_for_spawned_processes() {
        let (client, _task) = agent();
        let home = std::env::var("HOME").unwrap();
        let mut op = spawn("sh", &["-c", "printf '%s|' \"$1\" \"${PATH%%:*}\"", "sh", "~/x"]);
        if let Operation::Spawn { path, .. } = &mut op {
            path.push("~/bin".into());
        }
        let mut p = client.spawn(op).await.unwrap();
        let mut out = String::new();
        p.stdout.take().unwrap().read_to_string(&mut out).await.unwrap();
        let home = home.trim_end_matches('/');
        assert_eq!(out, format!("{home}/x|{home}/bin|"));
    }
}
