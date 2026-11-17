//! gRPC Client: one unary or server-streaming call through grpcurl, with the schema from the
//! server's reflection or from the active `.proto`. The call runs on its own thread and can
//! be cancelled; metadata values reach grpcurl through its environment, not its command line.

use rc_core::{Inbox, Waker};
use rc_kit::i18n::{tr, trf};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Longest a call may take (grpcurl's `-max-time`); the process is killed a bit later.
const MAX_TIME: Duration = Duration::from_secs(60);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Debug)]
pub struct Request {
    pub endpoint: String,
    pub method: String,
    pub json: String,
    pub metadata: String,
    pub plaintext: bool,
    /// Accept any server certificate (development certificates).
    pub insecure: bool,
    pub proto: Option<PathBuf>,
    pub import_path: Option<PathBuf>,
}

/// What grpcurl printed and how long the call took.
#[derive(Clone, Debug)]
pub struct Reply {
    pub body: String,
    pub elapsed: Duration,
}

/// A grpcurl command line and the environment holding the metadata values.
#[derive(Debug, PartialEq)]
pub struct Invocation {
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

pub struct GrpcClient {
    pub endpoint: String,
    pub method: String,
    pub json: String,
    pub metadata: String,
    pub plaintext: bool,
    pub insecure: bool,
    pub use_proto: bool,
    pub import_path: String,
    /// The request side shows the metadata instead of the JSON.
    pub show_metadata: bool,
    pub response: Option<Result<Reply, String>>,
    /// The response in a read-only editor, highlighted as JSON.
    pub body: Option<rc_editor::EditorState>,
    pub busy: bool,
    inbox: Inbox<(u64, Result<Reply, String>)>,
    cancel: Arc<AtomicBool>,
    next_id: u64,
}

impl GrpcClient {
    pub fn new(waker: Waker) -> Self {
        Self {
            endpoint: "localhost:5000".into(),
            method: String::new(),
            json: "{}".into(),
            metadata: String::new(),
            plaintext: true,
            insecure: false,
            use_proto: true,
            import_path: String::new(),
            show_metadata: false,
            response: None,
            body: None,
            busy: false,
            inbox: Inbox::new(waker),
            cancel: Arc::new(AtomicBool::new(false)),
            next_id: 0,
        }
    }

    /// Sends the request with `proto` (the active `.proto`) as schema when "Use active .proto"
    /// is on; otherwise grpcurl asks the server's reflection service.
    pub fn send(&mut self, proto: Option<PathBuf>) -> Result<(), String> {
        let proto = if self.use_proto { proto } else { None };
        let import_path = if self.import_path.trim().is_empty() {
            None
        } else {
            let path = PathBuf::from(self.import_path.trim());
            Some(if path.is_relative() {
                proto.as_ref().and_then(|source| source.parent()).map_or(path.clone(), |parent| parent.join(path))
            } else {
                path
            })
        };
        let request = Request {
            endpoint: self.endpoint.trim().to_string(),
            method: self.method.trim().to_string(),
            json: self.json.clone(),
            metadata: self.metadata.clone(),
            plaintext: self.plaintext,
            insecure: self.insecure,
            proto,
            import_path,
        };
        let invocation = arguments(&request)?;
        self.cancel();
        self.cancel = Arc::new(AtomicBool::new(false));
        self.next_id += 1;
        let id = self.next_id;
        self.response = None;
        self.body = None;
        self.busy = true;
        let cancel = self.cancel.clone();
        let out = self.inbox.sender();
        std::thread::spawn(move || {
            out.send((id, run(&grpcurl_path(), &invocation, &request.json, &cancel)));
        });
        Ok(())
    }

    pub fn cancel(&mut self) {
        if self.busy {
            self.cancel.store(true, Ordering::Relaxed);
            self.busy = false;
            self.next_id += 1;
            self.response = Some(Err(tr("Cancelled").to_string()));
        }
        self.inbox.drain();
    }

    pub fn poll(&mut self) {
        for (id, response) in self.inbox.drain() {
            if self.busy && id == self.next_id {
                self.show(response);
            }
        }
    }

    /// Shows a call's result; a reply goes into a read-only JSON editor.
    pub fn show(&mut self, response: Result<Reply, String>) {
        self.body = response.as_ref().ok().map(|reply| {
            // Ids of their own, apart from the documents' and the HTTP Client's.
            let mut editor = rc_editor::EditorState::new((1 << 42) + self.next_id, Some(PathBuf::from("response.json")), &reply.body);
            editor.read_only = true;
            if reply.body.len() < 64 * 1024 {
                editor.parse_blocking();
            }
            editor
        });
        self.response = Some(response);
        self.busy = false;
    }
}

impl Drop for GrpcClient {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// grpcurl from PATH or its usual install folders: DotCraft started from the Finder or the
/// Dock only sees a minimal PATH.
pub fn grpcurl_path() -> PathBuf {
    let exe = if cfg!(windows) { "grpcurl.exe" } else { "grpcurl" };
    let mut candidates: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).map(|d| d.join(exe)).collect()).unwrap_or_default();
    let home = dirs::home_dir();
    if cfg!(windows) {
        candidates.push(PathBuf::from(r"C:\ProgramData\chocolatey\bin\grpcurl.exe"));
        if let Some(home) = &home {
            candidates.extend([home.join(r"go\bin\grpcurl.exe"), home.join(r"scoop\shims\grpcurl.exe")]);
        }
    } else {
        candidates.extend(["/opt/homebrew/bin/grpcurl", "/usr/local/bin/grpcurl", "/usr/bin/grpcurl", "/snap/bin/grpcurl"].map(PathBuf::from));
        if let Some(home) = &home {
            candidates.extend([home.join("go/bin/grpcurl"), home.join(".local/bin/grpcurl")]);
        }
    }
    candidates.into_iter().find(|p| p.is_file()).unwrap_or_else(|| PathBuf::from(exe))
}

/// `host:port` without the `http://` / `https://` a copied URL carries; the scheme, when
/// present, decides between plaintext and TLS.
fn endpoint(text: &str) -> (String, Option<bool>) {
    let text = text.trim();
    let (rest, plaintext) = if let Some(rest) = text.strip_prefix("http://") {
        (rest, Some(true))
    } else if let Some(rest) = text.strip_prefix("https://") {
        (rest, Some(false))
    } else {
        (text, None)
    };
    (rest.trim_end_matches('/').to_string(), plaintext)
}

pub fn arguments(request: &Request) -> Result<Invocation, String> {
    let (endpoint, scheme_plaintext) = endpoint(&request.endpoint);
    if endpoint.is_empty() || endpoint.contains(char::is_whitespace) || endpoint.contains('/') {
        return Err(tr("Enter a gRPC host and port, such as localhost:5000.").to_string());
    }
    let (service, method) = request.method.split_once('/').ok_or_else(|| tr("Enter a gRPC method as package.Service/Method.").to_string())?;
    if service.is_empty() || method.is_empty() || method.contains('/') {
        return Err(tr("Enter a gRPC method as package.Service/Method.").to_string());
    }
    if request.json.len() > 1024 * 1024 {
        return Err(tr("gRPC request exceeds 1 MiB.").to_string());
    }
    serde_json::from_str::<serde_json::Value>(&request.json).map_err(|error| trf("Invalid request JSON: {error}", &[("error", &error.to_string())]))?;
    let mut args: Vec<String> = vec![
        "-max-time".into(),
        MAX_TIME.as_secs().to_string(),
        "-connect-timeout".into(),
        CONNECT_TIMEOUT.as_secs().to_string(),
        "-d".into(),
        "@".into(),
    ];
    if scheme_plaintext.unwrap_or(request.plaintext) {
        args.push("-plaintext".into());
    } else if request.insecure {
        args.push("-insecure".into());
    }
    if let Some(proto) = &request.proto {
        let import = request.import_path.as_deref().or_else(|| proto.parent());
        let proto_arg = import.and_then(|root| proto.strip_prefix(root).ok()).unwrap_or(proto.as_path());
        args.push("-proto".into());
        args.push(proto_arg.to_string_lossy().into_owned());
        if let Some(import) = import {
            args.push("-import-path".into());
            args.push(import.to_string_lossy().into_owned());
        }
    }
    // Values (tokens, API keys) go through the environment: `ps` shows only the names.
    let mut env = Vec::new();
    for line in request.metadata.lines().filter(|line| !line.trim().is_empty()) {
        let Some((key, value)) = line.split_once(':').filter(|(key, _)| !key.trim().is_empty() && !key.trim().contains(char::is_whitespace)) else {
            return Err(tr("Metadata must use one Key: Value entry per line.").to_string());
        };
        if line.contains(['\r', '\0']) {
            return Err(tr("Metadata must use one Key: Value entry per line.").to_string());
        }
        let name = format!("DOTCRAFT_GRPC_HEADER_{}", env.len());
        args.push("-H".into());
        args.push(format!("{}: ${{{name}}}", key.trim()));
        env.push((name, value.trim().to_string()));
    }
    if !env.is_empty() {
        args.push("-expand-headers".into());
    }
    args.push(endpoint);
    args.push(request.method.clone());
    Ok(Invocation { args, env })
}

fn run(program: &Path, invocation: &Invocation, json: &str, cancel: &AtomicBool) -> Result<Reply, String> {
    let started = Instant::now();
    let mut cmd = Command::new(program);
    cmd.args(&invocation.args).envs(invocation.env.iter().map(|(k, v)| (k, v))).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = cmd.spawn().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            tr("Install grpcurl to send gRPC requests: github.com/fullstorydev/grpcurl").to_string()
        } else {
            trf("Could not start grpcurl: {error}", &[("error", &error.to_string())])
        }
    })?;
    let mut stdin = child.stdin.take().expect("piped stdin");
    let body = json.as_bytes().to_vec();
    let writer = std::thread::spawn(move || stdin.write_all(&body));
    let mut stdout = child.stdout.take().expect("piped stdout");
    let mut stderr = child.stderr.take().expect("piped stderr");
    let output = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stdout.read_to_end(&mut bytes);
        bytes
    });
    let errors = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stderr.read_to_end(&mut bytes);
        bytes
    });
    let status = loop {
        if cancel.load(Ordering::Relaxed) || started.elapsed() > MAX_TIME + Duration::from_secs(5) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(tr("gRPC request was cancelled or timed out.").to_string());
        }
        match child.try_wait().map_err(|error| error.to_string())? {
            Some(status) => break status,
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    };
    let _ = writer.join();
    let stdout = String::from_utf8_lossy(&output.join().unwrap_or_default()).into_owned();
    let stderr = String::from_utf8_lossy(&errors.join().unwrap_or_default()).into_owned();
    if status.success() {
        Ok(Reply { body: stdout, elapsed: started.elapsed() })
    } else {
        Err(if stderr.trim().is_empty() { tr("gRPC request failed.").to_string() } else { stderr.trim().to_string() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> Request {
        Request {
            endpoint: "localhost:5001".into(),
            method: "shop.Catalog/Get".into(),
            json: "{}".into(),
            metadata: String::new(),
            plaintext: false,
            insecure: false,
            proto: None,
            import_path: None,
        }
    }

    #[test]
    fn builds_reflection_and_proto_requests_without_a_shell() {
        let request = Request { metadata: "Authorization: Bearer secret-token\nx-tenant:  acme ".into(), ..request() };
        let call = arguments(&request).unwrap();
        assert_eq!(&call.args[call.args.len() - 2..], ["localhost:5001", "shop.Catalog/Get"]);
        assert!(call.args.windows(2).any(|pair| pair == ["-H", "Authorization: ${DOTCRAFT_GRPC_HEADER_0}"]));
        assert!(call.args.windows(2).any(|pair| pair == ["-H", "x-tenant: ${DOTCRAFT_GRPC_HEADER_1}"]));
        assert!(call.args.contains(&"-expand-headers".to_string()));
        assert!(!call.args.iter().any(|a| a.contains("secret-token")), "{:?}", call.args);
        assert_eq!(call.env, [("DOTCRAFT_GRPC_HEADER_0".to_string(), "Bearer secret-token".to_string()), ("DOTCRAFT_GRPC_HEADER_1".to_string(), "acme".to_string())]);
        assert!(!call.args.contains(&"-plaintext".to_string()));
        let dir = std::env::temp_dir().join("protos");
        let with_proto = Request { proto: Some(dir.join("shop.proto")), plaintext: true, ..request };
        let call = arguments(&with_proto).unwrap();
        assert!(call.args.windows(2).any(|pair| pair == ["-proto", "shop.proto"]));
        assert!(call.args.windows(2).any(|pair| pair == ["-import-path".to_string(), dir.to_string_lossy().into_owned()]));
        assert!(call.args.contains(&"-plaintext".to_string()));
    }

    #[test]
    fn a_url_sets_the_transport() {
        let call = arguments(&Request { endpoint: "https://localhost:7042/".into(), plaintext: true, insecure: true, ..request() }).unwrap();
        assert!(!call.args.contains(&"-plaintext".to_string()));
        assert!(call.args.contains(&"-insecure".to_string()));
        assert_eq!(call.args[call.args.len() - 2], "localhost:7042");
        let call = arguments(&Request { endpoint: "http://localhost:5000".into(), ..request() }).unwrap();
        assert!(call.args.contains(&"-plaintext".to_string()));
    }

    #[test]
    fn rejects_bad_method_metadata_and_json() {
        let mut request = Request { method: "Bad".into(), ..request() };
        assert!(arguments(&request).is_err());
        request.method = "shop.Service/Call".into();
        request.metadata = "no colon".into();
        assert!(arguments(&request).is_err());
        request.metadata = String::new();
        request.json = "{".into();
        assert!(arguments(&request).is_err());
        request.json = "{}".into();
        request.endpoint = "localhost:5000/path".into();
        assert!(arguments(&request).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn sends_json_on_stdin_and_reads_the_response() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("fake-grpcurl");
        std::fs::write(&program, "#!/bin/sh\nprintf '%s ' \"$DOTCRAFT_GRPC_HEADER_0\"\ncat\n").unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
        let invocation = Invocation { args: Vec::new(), env: vec![("DOTCRAFT_GRPC_HEADER_0".into(), "token".into())] };
        let reply = run(&program, &invocation, "{\"id\":42}", &AtomicBool::new(false)).unwrap();
        assert_eq!(reply.body, "token {\"id\":42}");
    }

    #[cfg(unix)]
    #[test]
    fn cancelling_kills_the_process() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("slow-grpcurl");
        std::fs::write(&program, "#!/bin/sh\nexec sleep 30\n").unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            flag.store(true, Ordering::Relaxed);
        });
        let started = Instant::now();
        let invocation = Invocation { args: Vec::new(), env: Vec::new() };
        assert!(run(&program, &invocation, "{}", &cancel).is_err());
        assert!(started.elapsed() < Duration::from_secs(10));
    }
}
