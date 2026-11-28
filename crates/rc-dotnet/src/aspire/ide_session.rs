//! The IDE session endpoint of Aspire's "IDE execution" protocol
//! (`docs/specs/IDE-execution.md` in dotnet/aspire), the way Visual Studio, VS Code and Rider
//! debug the services of an app host.
//!
//! The app host is started with `DEBUG_SESSION_PORT`, `DEBUG_SESSION_TOKEN` and
//! `DEBUG_SESSION_RUN_MODE`; its orchestrator (DCP) then asks this endpoint to run each project
//! (`PUT /run_session`) instead of starting it, and stops it with `DELETE /run_session/{id}`.
//! The IDE reports the process id, the output and the exit of each run through a WebSocket
//! (`GET /run_session/notify`). Plain HTTP on loopback, authenticated with the bearer token.

use rc_core::Runtime;
use rc_core::inbox::Outbox;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;

/// Protocol versions this endpoint speaks (see the spec's "Protocol versioning").
pub const PROTOCOLS: [&str; 3] = ["2024-03-03", "2024-04-23", "2025-10-01"];

/// What DCP asks for a project (`type: project` launch configuration).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunSession {
    pub project: std::path::PathBuf,
    /// `Debug` (default) or `NoDebug`.
    pub debug: bool,
    pub launch_profile: Option<String>,
    pub disable_launch_profile: bool,
    pub configuration: Option<String>,
    /// Variables over the launch profile's.
    pub env: Vec<(String, String)>,
    /// `Some` replaces the profile's arguments (even when empty).
    pub args: Option<Vec<String>>,
}

impl RunSession {
    /// Parses a `PUT /run_session` body; `Err` is the error code and message for DCP.
    pub fn parse(body: &[u8]) -> Result<RunSession, (&'static str, String)> {
        #[derive(Deserialize)]
        struct Env {
            name: String,
            #[serde(default)]
            value: Option<String>,
        }
        #[derive(Deserialize)]
        struct Request {
            #[serde(default)]
            launch_configurations: Vec<Value>,
            #[serde(default)]
            env: Option<Vec<Env>>,
            #[serde(default)]
            args: Option<Vec<String>>,
        }
        let request: Request = serde_json::from_slice(body).map_err(|e| ("InvalidRequest", format!("The request is not valid JSON: {e}")))?;
        let config = request
            .launch_configurations
            .iter()
            .find(|c| c["type"].as_str() == Some("project"))
            .ok_or(("UnsupportedLaunchConfiguration", "DotCraft only runs project launch configurations".to_string()))?;
        let project = config["project_path"].as_str().unwrap_or_default();
        let path = std::path::PathBuf::from(project);
        if project.is_empty() || !path.is_file() {
            return Err(("ProjectNotFound", format!("The project '{project}' was not found")));
        }
        let text = |key: &str| config[key].as_str().filter(|s| !s.is_empty()).map(str::to_string);
        Ok(RunSession {
            project: path,
            debug: !config["mode"].as_str().is_some_and(|m| m.eq_ignore_ascii_case("NoDebug")),
            launch_profile: text("launch_profile"),
            disable_launch_profile: config["disable_launch_profile"].as_bool().unwrap_or(false),
            configuration: text("build_configuration"),
            env: request.env.unwrap_or_default().into_iter().map(|e| (e.name, e.value.unwrap_or_default())).collect(),
            args: request.args,
        })
    }
}

/// What the endpoint asks the IDE to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionRequest {
    Start { id: String, session: RunSession },
    Stop { id: String },
}

/// Sends run session notifications to DCP. Cheap to clone.
#[derive(Clone)]
pub struct Notifier {
    sockets: Arc<Mutex<Vec<mpsc::UnboundedSender<Frame>>>>,
    ended: Arc<Mutex<HashSet<String>>>,
}

impl Notifier {
    fn send(&self, notification: Value) {
        let text = notification.to_string();
        let mut sockets = self.sockets.lock().unwrap_or_else(|e| e.into_inner());
        sockets.retain(|s| s.send(Frame::Text(text.clone())).is_ok());
    }

    pub fn process_started(&self, id: &str, pid: u32) {
        self.send(json!({ "notification_type": "processRestarted", "session_id": id, "pid": pid }));
    }

    pub fn log(&self, id: &str, stderr: bool, line: &str) {
        self.send(json!({ "notification_type": "serviceLogs", "session_id": id, "is_std_err": stderr, "log_message": line }));
    }

    pub fn terminated(&self, id: &str, exit_code: Option<i32>) {
        self.ended.lock().unwrap_or_else(|e| e.into_inner()).insert(id.to_string());
        let mut n = json!({ "notification_type": "sessionTerminated", "session_id": id });
        if let Some(code) = exit_code {
            n["exit_code"] = json!(code as u32);
        }
        self.send(n);
    }

    pub fn message(&self, id: &str, error: bool, message: &str) {
        let mut n = json!({ "notification_type": "sessionMessage", "session_id": id, "level": if error { "error" } else { "info" }, "message": message });
        if error {
            n["code"] = json!("SessionFailed");
        }
        self.send(n);
    }
}

enum Frame {
    Text(String),
    Pong(Vec<u8>),
    Close,
}

/// The listening endpoint; stops when dropped.
pub struct IdeSessionServer {
    pub port: u16,
    pub token: String,
    notifier: Notifier,
    task: tokio::task::JoinHandle<()>,
}

impl IdeSessionServer {
    /// Listens on a free loopback port. Requests go to `out`.
    pub fn start(out: Outbox<SessionRequest>) -> std::io::Result<IdeSessionServer> {
        let std_listener = std::net::TcpListener::bind(("127.0.0.1", 0))?;
        std_listener.set_nonblocking(true)?;
        let port = std_listener.local_addr()?.port();
        let token = super::random_token();
        let notifier = Notifier { sockets: Default::default(), ended: Default::default() };
        let state = Arc::new(Shared { token: token.clone(), out, notifier: notifier.clone(), next: Mutex::new(0), sessions: Mutex::new(HashSet::new()), port });
        let task = Runtime::spawn(async move {
            let Ok(listener) = TcpListener::from_std(std_listener) else { return };
            while let Ok((stream, _)) = listener.accept().await {
                let state = state.clone();
                tokio::spawn(async move {
                    let _ = serve(stream, state).await;
                });
            }
        });
        Ok(IdeSessionServer { port, token, notifier, task })
    }

    pub fn notifier(&self) -> Notifier {
        self.notifier.clone()
    }

    /// Environment for the app host: where DCP finds this endpoint, and that services run
    /// under the debugger (`DEBUG_SESSION_RUN_MODE`).
    pub fn host_environment(&self, debug: bool) -> Vec<(String, String)> {
        let info = json!({ "protocols_supported": PROTOCOLS, "supported_launch_configurations": ["project"] });
        vec![
            ("DEBUG_SESSION_PORT".into(), format!("localhost:{}", self.port)),
            ("DEBUG_SESSION_TOKEN".into(), self.token.clone()),
            ("DEBUG_SESSION_RUN_MODE".into(), if debug { "Debug" } else { "NoDebug" }.into()),
            ("DEBUG_SESSION_INFO".into(), info.to_string()),
        ]
    }
}

impl Drop for IdeSessionServer {
    fn drop(&mut self) {
        let sockets = self.notifier.sockets.lock().map(|s| s.clone()).unwrap_or_default();
        for s in sockets {
            let _ = s.send(Frame::Close);
        }
        self.task.abort();
    }
}

struct Shared {
    token: String,
    out: Outbox<SessionRequest>,
    notifier: Notifier,
    next: Mutex<u64>,
    sessions: Mutex<HashSet<String>>,
    port: u16,
}

struct Request {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

/// Reads one HTTP/1.1 request (headers and a `Content-Length` body). `None` at end of stream.
async fn read_request(stream: &mut TcpStream, buf: &mut Vec<u8>) -> std::io::Result<Option<Request>> {
    let head_end = loop {
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i;
        }
        if buf.len() > 64 * 1024 {
            return Err(std::io::Error::other("request headers too large"));
        }
        let mut chunk = [0u8; 8192];
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            return Ok(None);
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    let mut lines = head.split("\r\n");
    let mut first = lines.next().unwrap_or("").split(' ');
    let (method, path) = (first.next().unwrap_or("").to_string(), first.next().unwrap_or("").to_string());
    let headers: Vec<(String, String)> = lines.filter_map(|l| l.split_once(':')).map(|(k, v)| (k.trim().to_string(), v.trim().to_string())).collect();
    let length: usize = headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("content-length")).and_then(|(_, v)| v.parse().ok()).unwrap_or(0);
    if length > 16 * 1024 * 1024 {
        return Err(std::io::Error::other("request body too large"));
    }
    buf.drain(..head_end + 4);
    while buf.len() < length {
        let mut chunk = [0u8; 65536];
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            return Ok(None);
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    let body: Vec<u8> = buf.drain(..length).collect();
    Ok(Some(Request { method, path, headers, body }))
}

async fn respond(stream: &mut TcpStream, status: &str, extra: &[(&str, String)], body: Option<Value>) -> std::io::Result<()> {
    let body = body.map(|b| b.to_string()).unwrap_or_default();
    let mut head = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\n", body.len());
    if !body.is_empty() {
        head.push_str("Content-Type: application/json\r\n");
    }
    for (k, v) in extra {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    head.push_str(&body);
    stream.write_all(head.as_bytes()).await?;
    stream.flush().await
}

fn error_body(code: &str, message: &str) -> Value {
    json!({ "error": { "code": code, "message": message, "details": [] } })
}

async fn serve(mut stream: TcpStream, state: Arc<Shared>) -> std::io::Result<()> {
    let mut buf = Vec::new();
    while let Some(request) = read_request(&mut stream, &mut buf).await? {
        let authorized = request.header("authorization").is_some_and(|a| constant_time_eq(a.as_bytes(), format!("Bearer {}", state.token).as_bytes()));
        if !authorized {
            respond(&mut stream, "401 Unauthorized", &[], None).await?;
            continue;
        }
        let path = request.path.split('?').next().unwrap_or("").to_string();
        match (request.method.as_str(), path.as_str()) {
            ("GET", "/info") => {
                let info = json!({ "protocols_supported": PROTOCOLS, "supported_launch_configurations": ["project"] });
                respond(&mut stream, "200 OK", &[], Some(info)).await?;
            }
            ("GET", "/run_session/notify") => {
                let Some(key) = request.header("sec-websocket-key").map(str::to_string) else {
                    respond(&mut stream, "400 Bad Request", &[], None).await?;
                    continue;
                };
                let accept = websocket_accept(&key);
                stream.write_all(format!("HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n").as_bytes()).await?;
                return websocket(stream, buf, state).await;
            }
            ("PUT", "/run_session") => match RunSession::parse(&request.body) {
                Ok(session) => {
                    let id = {
                        let mut next = state.next.lock().unwrap_or_else(|e| e.into_inner());
                        *next += 1;
                        format!("dotcraft-{}", *next)
                    };
                    state.sessions.lock().unwrap_or_else(|e| e.into_inner()).insert(id.clone());
                    let location = format!("http://localhost:{}/run_session/{id}", state.port);
                    respond(&mut stream, "201 Created", &[("Location", location)], None).await?;
                    state.out.send(SessionRequest::Start { id, session });
                }
                Err((code, message)) => respond(&mut stream, "400 Bad Request", &[], Some(error_body(code, &message))).await?,
            },
            ("DELETE", p) if p.starts_with("/run_session/") => {
                let id = p.trim_start_matches("/run_session/").to_string();
                let known = state.sessions.lock().unwrap_or_else(|e| e.into_inner()).contains(&id);
                let ended = state.notifier.ended.lock().unwrap_or_else(|e| e.into_inner()).contains(&id);
                if known && !ended {
                    respond(&mut stream, "200 OK", &[], None).await?;
                    state.out.send(SessionRequest::Stop { id });
                } else {
                    respond(&mut stream, "204 No Content", &[], None).await?;
                }
            }
            _ => respond(&mut stream, "404 Not Found", &[], None).await?,
        }
    }
    Ok(())
}

/// Serves the notification WebSocket: writes notifications, answers pings, ends on close.
async fn websocket(stream: TcpStream, mut pending: Vec<u8>, state: Arc<Shared>) -> std::io::Result<()> {
    let (mut reader, mut writer) = stream.into_split();
    let (tx, mut rx) = mpsc::unbounded_channel::<Frame>();
    state.notifier.sockets.lock().unwrap_or_else(|e| e.into_inner()).push(tx.clone());
    let write = tokio::spawn(async move {
        while let Some(frame) = rx.recv().await {
            let (opcode, payload, close) = match frame {
                Frame::Text(t) => (0x1, t.into_bytes(), false),
                Frame::Pong(p) => (0xA, p, false),
                Frame::Close => (0x8, Vec::new(), true),
            };
            if writer.write_all(&encode_frame(opcode, &payload)).await.is_err() || close {
                break;
            }
        }
        let _ = writer.shutdown().await;
    });
    loop {
        match decode_frame(&mut pending) {
            Some((opcode, payload)) => match opcode {
                0x9 => {
                    let _ = tx.send(Frame::Pong(payload));
                }
                0x8 => {
                    let _ = tx.send(Frame::Close);
                    break;
                }
                _ => {}
            },
            None => {
                let mut chunk = [0u8; 4096];
                match reader.read(&mut chunk).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => pending.extend_from_slice(&chunk[..n]),
                }
            }
        }
    }
    drop(tx);
    let _ = write.await;
    Ok(())
}

/// A server frame (never masked, never fragmented).
fn encode_frame(opcode: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = vec![0x80 | opcode];
    match payload.len() {
        n if n < 126 => out.push(n as u8),
        n if n <= u16::MAX as usize => {
            out.push(126);
            out.extend_from_slice(&(n as u16).to_be_bytes());
        }
        n => {
            out.push(127);
            out.extend_from_slice(&(n as u64).to_be_bytes());
        }
    }
    out.extend_from_slice(payload);
    out
}

/// Takes one complete client frame off `buf` (unmasked): `(opcode, payload)`.
fn decode_frame(buf: &mut Vec<u8>) -> Option<(u8, Vec<u8>)> {
    if buf.len() < 2 {
        return None;
    }
    let opcode = buf[0] & 0x0f;
    let masked = buf[1] & 0x80 != 0;
    let (len, mut at) = match buf[1] & 0x7f {
        126 => (u16::from_be_bytes([*buf.get(2)?, *buf.get(3)?]) as usize, 4),
        127 => (u64::from_be_bytes(buf.get(2..10)?.try_into().ok()?) as usize, 10),
        n => (n as usize, 2),
    };
    let mask = if masked {
        let m: [u8; 4] = buf.get(at..at + 4)?.try_into().ok()?;
        at += 4;
        Some(m)
    } else {
        None
    };
    let end = at.checked_add(len)?;
    if buf.len() < end {
        return None;
    }
    let mut payload = buf[at..end].to_vec();
    if let Some(m) = mask {
        for (i, b) in payload.iter_mut().enumerate() {
            *b ^= m[i % 4];
        }
    }
    buf.drain(..end);
    Some((opcode, payload))
}

fn websocket_accept(key: &str) -> String {
    use base64::Engine;
    let digest = sha1(format!("{key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11").as_bytes());
    base64::engine::general_purpose::STANDARD.encode(digest)
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// SHA-1, only for the WebSocket handshake (RFC 6455 requires it).
fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0];
    let mut message = data.to_vec();
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    for block in message.chunks(64) {
        let mut w = [0u32; 80];
        for (i, word) in block.chunks(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (i, wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5A827999),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                _ => (b ^ c ^ d, 0xCA62C1D6),
            };
            let t = a.rotate_left(5).wrapping_add(f).wrapping_add(e).wrapping_add(k).wrapping_add(*wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = t;
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e]) {
            *x = x.wrapping_add(y);
        }
    }
    let mut out = [0u8; 20];
    for (i, x) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&x.to_be_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::time::Duration;

    #[test]
    fn websocket_accept_matches_rfc_6455() {
        assert_eq!(websocket_accept("dGhlIHNhbXBsZSBub25jZQ=="), "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=");
        let hex: String = sha1(b"abc").iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, "a9993e364706816aba3e25717850c26c9cd0d89d");
    }

    #[test]
    fn frames_round_trip_with_client_masks() {
        let mut masked = vec![0x89, 0x80 | 3, 1, 2, 3, 4];
        masked.extend([b'a' ^ 1, b'b' ^ 2, b'c' ^ 3]);
        masked.push(0x81);
        assert_eq!(decode_frame(&mut masked), Some((0x9, b"abc".to_vec())));
        assert_eq!(masked, vec![0x81], "an incomplete frame stays");
        assert_eq!(decode_frame(&mut masked), None);
        let long = vec![7u8; 300];
        let mut encoded = encode_frame(0x1, &long);
        assert_eq!(&encoded[..4], &[0x81, 126, 1, 44]);
        assert_eq!(decode_frame(&mut encoded), Some((0x1, long)));
    }

    #[test]
    fn parses_project_run_sessions() {
        let project = tempfile::NamedTempFile::with_suffix(".csproj").unwrap();
        let body = json!({
            "launch_configurations": [{ "type": "project", "project_path": project.path(), "launch_profile": "https", "mode": "Debug", "build_configuration": "Debug" }],
            "env": [{ "name": "ASPNETCORE_URLS", "value": "http://localhost:5000" }, { "name": "EMPTY" }],
        });
        let s = RunSession::parse(body.to_string().as_bytes()).unwrap();
        assert_eq!(s.project, project.path());
        assert!(s.debug);
        assert_eq!(s.launch_profile.as_deref(), Some("https"));
        assert_eq!(s.env, vec![("ASPNETCORE_URLS".into(), "http://localhost:5000".into()), ("EMPTY".into(), String::new())]);
        assert_eq!(s.args, None);
        let missing = json!({ "launch_configurations": [{ "type": "project", "project_path": "/nonexistent/A.csproj" }] });
        assert_eq!(RunSession::parse(missing.to_string().as_bytes()).unwrap_err().0, "ProjectNotFound");
        let python = json!({ "launch_configurations": [{ "type": "python", "program_path": "app.py" }] });
        assert_eq!(RunSession::parse(python.to_string().as_bytes()).unwrap_err().0, "UnsupportedLaunchConfiguration");
    }

    fn http(port: u16, request: &str) -> String {
        let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        s.write_all(request.as_bytes()).unwrap();
        let mut out = vec![0u8; 4096];
        let n = s.read(&mut out).unwrap();
        String::from_utf8_lossy(&out[..n]).into_owned()
    }

    #[test]
    fn serves_the_protocol_over_loopback() {
        let inbox = rc_core::Inbox::new(rc_core::Waker::noop());
        let server = IdeSessionServer::start(inbox.sender()).unwrap();
        let auth = format!("Authorization: Bearer {}\r\n", server.token);
        assert!(http(server.port, "GET /info HTTP/1.1\r\nAuthorization: Bearer wrong\r\n\r\n").starts_with("HTTP/1.1 401"));
        let info = http(server.port, &format!("GET /info HTTP/1.1\r\n{auth}\r\n"));
        assert!(info.starts_with("HTTP/1.1 200") && info.contains("2024-04-23"), "{info}");

        // Subscribe to notifications, then create and stop a session.
        let mut ws = std::net::TcpStream::connect(("127.0.0.1", server.port)).unwrap();
        ws.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        ws.write_all(format!("GET /run_session/notify?api-version=2025-10-01 HTTP/1.1\r\n{auth}Upgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n").as_bytes()).unwrap();
        let mut head = vec![0u8; 1024];
        let n = ws.read(&mut head).unwrap();
        assert!(String::from_utf8_lossy(&head[..n]).contains("s3pPLMBiTxaQ9kYGzzhZRbK+xOo="));

        let project = tempfile::NamedTempFile::with_suffix(".csproj").unwrap();
        let body = json!({ "launch_configurations": [{ "type": "project", "project_path": project.path() }] }).to_string();
        let created = http(server.port, &format!("PUT /run_session?api-version=2025-10-01 HTTP/1.1\r\n{auth}Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", body.len()));
        assert!(created.starts_with("HTTP/1.1 201") && created.contains("Location: http://localhost:"), "{created}");
        let request = inbox.recv_timeout(Duration::from_secs(5)).unwrap();
        let SessionRequest::Start { id, session } = request else { panic!("{request:?}") };
        assert!(session.debug);

        // A ping is answered, a notification arrives as a text frame.
        let mut ping = vec![0x89, 0x80 | 2, 0, 0, 0, 0];
        ping.extend(b"hi");
        ws.write_all(&ping).unwrap();
        server.notifier().process_started(&id, 4242);
        let mut frames = Vec::new();
        let mut got = Vec::new();
        while got.len() < 2 {
            let mut chunk = [0u8; 1024];
            let n = ws.read(&mut chunk).unwrap();
            frames.extend_from_slice(&chunk[..n]);
            while let Some(f) = decode_frame(&mut frames) {
                got.push(f);
            }
        }
        assert!(got.contains(&(0xA, b"hi".to_vec())), "{got:?}");
        let text = got.iter().find(|(op, _)| *op == 1).map(|(_, p)| String::from_utf8_lossy(p).into_owned()).unwrap();
        let n: Value = serde_json::from_str(&text).unwrap();
        assert_eq!((n["notification_type"].as_str(), n["pid"].as_u64()), (Some("processRestarted"), Some(4242)));

        assert!(http(server.port, &format!("DELETE /run_session/{id}?api-version=2025-10-01 HTTP/1.1\r\n{auth}\r\n")).starts_with("HTTP/1.1 200"));
        assert_eq!(inbox.recv_timeout(Duration::from_secs(5)), Some(SessionRequest::Stop { id: id.clone() }));
        server.notifier().terminated(&id, Some(0));
        assert!(http(server.port, &format!("DELETE /run_session/{id} HTTP/1.1\r\n{auth}\r\n")).starts_with("HTTP/1.1 204"));
    }
}
