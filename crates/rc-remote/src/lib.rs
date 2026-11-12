//! Versioned JSON-line transport for a remote DotCraft workspace agent.
//! The agent runs with the remote user's filesystem and process permissions.

use anyhow::{Context, Result, bail};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    process::Stdio,
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::mpsc,
    task::JoinHandle,
};

pub const PROTOCOL_VERSION: u32 = 1;
const MAX_BYTES: usize = 1024 * 1024;
const MAX_LINE_BYTES: usize = 2 * 1024 * 1024;

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
        path: PathBuf,
    },
    Read {
        path: PathBuf,
    },
    Write {
        path: PathBuf,
        data: String,
    },
    Run {
        program: String,
        args: Vec<String>,
        cwd: PathBuf,
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

impl Endpoint {
    pub fn command(&self) -> Result<Command> {
        match self {
            Endpoint::Ssh { host, agent } => {
                if host.is_empty()
                    || host.starts_with('-')
                    || host.chars().any(char::is_whitespace)
                    || agent.is_empty()
                {
                    bail!("invalid SSH host or agent");
                }
                // ssh executes its remote command through a shell; quote the executable as one word.
                let quoted = format!("'{}'", agent.replace('\'', "'\\''"));
                let mut cmd = Command::new("ssh");
                cmd.args(["-T", "--", host, &quoted]);
                Ok(cmd)
            }
            Endpoint::Wsl {
                distribution,
                agent,
            } => {
                if distribution.is_empty() || agent.is_empty() {
                    bail!("invalid WSL distribution or agent");
                }
                let mut cmd = Command::new("wsl.exe");
                cmd.args(["--distribution", distribution, "--exec", agent]);
                Ok(cmd)
            }
        }
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
            line = read_line_bounded(&mut reader) => {
                let Some(line) = line? else { break };
                let req: Request = match serde_json::from_slice(&line) {
                    Ok(req) => req,
                    Err(_) => continue,
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
) -> Result<Option<Vec<u8>>> {
    let mut line = Vec::new();
    loop {
        let chunk = reader.fill_buf().await?;
        if chunk.is_empty() {
            if line.is_empty() {
                return Ok(None);
            }
            bail!("truncated remote request");
        }
        let count = chunk
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(chunk.len(), |index| index + 1);
        if line.len() + count > MAX_LINE_BYTES {
            bail!("remote request exceeds 2 MiB line limit");
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
            let mut dir = tokio::fs::read_dir(path).await?;
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
            let file = tokio::fs::File::open(path).await?;
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
            tokio::fs::write(path, bytes).await?;
            Ok(Value::Written)
        }
        Operation::Run {
            program,
            args,
            cwd,
            env,
            timeout_ms,
        } => {
            if program.is_empty() || timeout_ms == 0 || timeout_ms > 120_000 {
                bail!("invalid process request");
            }
            let mut cmd = Command::new(program);
            cmd.args(args)
                .current_dir(cwd)
                .envs(env)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true);
            let mut child = cmd.spawn()?;
            let stdout = child.stdout.take().context("missing stdout")?;
            let stderr = child.stderr.take().context("missing stderr")?;
            let output = tokio::time::timeout(Duration::from_millis(timeout_ms), async {
                let (out, err, status) =
                    tokio::try_join!(capture_limited(stdout), capture_limited(stderr), async {
                        Ok::<_, anyhow::Error>(child.wait().await?)
                    })?;
                Result::<_>::Ok((out, err, status.code()))
            })
            .await;
            let output = match output {
                Ok(Ok(output)) => output,
                Ok(Err(error)) => {
                    let _ = child.kill().await;
                    return Err(error);
                }
                Err(_) => {
                    let _ = child.kill().await;
                    bail!("process timed out");
                }
            };
            Ok(Value::Exit {
                code: output.2,
                stdout: STANDARD.encode(output.0),
                stderr: STANDARD.encode(output.1),
            })
        }
        Operation::Cancel { .. } => bail!("cancel is handled by the server"),
    }
}

async fn capture_limited<R: AsyncRead + Unpin>(mut reader: R) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    let mut buffer = [0u8; 8192];
    loop {
        let count = reader.read(&mut buffer).await?;
        if count == 0 {
            return Ok(output);
        }
        if output.len() + count > MAX_BYTES {
            bail!("process output exceeds 1 MiB limit");
        }
        output.extend_from_slice(&buffer[..count]);
    }
}

/// A sequential client for the current transport. Parallel callers may use separate connections.
pub struct Client<R, W> {
    read: tokio::io::Lines<BufReader<R>>,
    write: W,
    next_id: u64,
}

/// Owns one SSH or WSL agent process and its checked protocol connection.
pub struct RemoteSession {
    pub client: Client<ChildStdout, ChildStdin>,
    _child: Child,
}

impl RemoteSession {
    pub async fn connect(endpoint: &Endpoint) -> Result<Self> {
        let mut child = endpoint.spawn()?;
        let stdout = child
            .stdout
            .take()
            .context("remote agent stdout is unavailable")?;
        let stdin = child
            .stdin
            .take()
            .context("remote agent stdin is unavailable")?;
        let mut client = Client::new(stdout, stdin);
        tokio::time::timeout(Duration::from_secs(10), client.handshake())
            .await
            .context("remote agent handshake timed out")??;
        Ok(Self {
            client,
            _child: child,
        })
    }
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
        let mut client = Client::new(client_read, client_write);
        client.handshake().await.unwrap();
        let path = dir.path().join("example.txt");
        client
            .request(Operation::Write {
                path: path.clone(),
                data: STANDARD.encode("hello"),
            })
            .await
            .unwrap();
        let Value::Data { data } = client
            .request(Operation::Read { path: path.clone() })
            .await
            .unwrap()
        else {
            panic!("read response")
        };
        assert_eq!(STANDARD.decode(data).unwrap(), b"hello");
        let Value::Entries { entries } = client
            .request(Operation::List {
                path: dir.path().to_path_buf(),
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
                cwd: dir.path().to_path_buf(),
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
        assert_eq!(
            args,
            ["-T", "--", "example.org", "'/home/a b/agent'\\''v1'"]
        );
        assert!(
            Endpoint::Ssh {
                host: "-bad".into(),
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
                cwd: std::env::temp_dir(),
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

impl<R: AsyncRead + Unpin, W: AsyncWrite + Unpin> Client<R, W> {
    pub fn new(read: R, write: W) -> Self {
        Self {
            read: BufReader::new(read).lines(),
            write,
            next_id: 1,
        }
    }

    pub async fn request(&mut self, operation: Operation) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        let request = Request {
            version: PROTOCOL_VERSION,
            id,
            operation,
        };
        self.write
            .write_all(serde_json::to_string(&request)?.as_bytes())
            .await?;
        self.write.write_all(b"\n").await?;
        self.write.flush().await?;
        let line = self
            .read
            .next_line()
            .await?
            .context("remote agent disconnected")?;
        let response: Response = serde_json::from_str(&line)?;
        if response.version != PROTOCOL_VERSION || response.id != id {
            bail!("unexpected remote response");
        }
        response.result.map_err(anyhow::Error::msg)
    }

    pub async fn handshake(&mut self) -> Result<()> {
        match self.request(Operation::Hello).await? {
            Value::Hello {
                version: PROTOCOL_VERSION,
            } => Ok(()),
            _ => bail!("remote agent protocol mismatch"),
        }
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
        let error = read_line_bounded(&mut BufReader::new(reader))
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
            cwd: std::env::temp_dir(),
            env: vec![],
            timeout_ms: 5_000,
        })
        .await
        .unwrap_err();
        assert!(error.to_string().contains("output exceeds"));
        assert!(start.elapsed() < Duration::from_secs(4));
    }
}
