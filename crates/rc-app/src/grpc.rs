//! A cancellable gRPC request client backed by grpcurl, with optional local `.proto` source.

use rc_core::{Inbox, Waker};
use rc_kit::i18n::{tr, trf};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub struct Request {
    pub endpoint: String,
    pub method: String,
    pub json: String,
    pub metadata: String,
    pub plaintext: bool,
    pub proto: Option<PathBuf>,
    pub import_path: Option<PathBuf>,
}

pub struct GrpcClient {
    pub endpoint: String,
    pub method: String,
    pub json: String,
    pub metadata: String,
    pub plaintext: bool,
    pub use_proto: bool,
    pub import_path: String,
    pub response: Option<Result<String, String>>,
    pub busy: bool,
    inbox: Inbox<(u64, Result<String, String>)>,
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
            use_proto: true,
            import_path: String::new(),
            response: None,
            busy: false,
            inbox: Inbox::new(waker),
            cancel: Arc::new(AtomicBool::new(false)),
            next_id: 0,
        }
    }

    pub fn send(&mut self, proto: Option<PathBuf>) -> Result<(), String> {
        let proto = if self.use_proto { proto } else { None };
        let import_path = if self.import_path.trim().is_empty() {
            None
        } else {
            let path = PathBuf::from(self.import_path.trim());
            Some(if path.is_relative() {
                proto
                    .as_ref()
                    .and_then(|source| source.parent())
                    .map_or(path.clone(), |parent| parent.join(path))
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
            proto,
            import_path,
        };
        let args = arguments(&request)?;
        self.cancel();
        self.cancel = Arc::new(AtomicBool::new(false));
        self.next_id += 1;
        let id = self.next_id;
        self.response = None;
        self.busy = true;
        let cancel = self.cancel.clone();
        let out = self.inbox.sender();
        std::thread::spawn(move || {
            out.send((id, run(&args, &request.json, &cancel)));
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
                self.response = Some(response);
                self.busy = false;
            }
        }
    }
}

impl Drop for GrpcClient {
    fn drop(&mut self) {
        self.cancel();
    }
}

pub fn arguments(request: &Request) -> Result<Vec<String>, String> {
    if request.endpoint.is_empty()
        || request.endpoint.contains(char::is_whitespace)
        || request.endpoint.contains('/')
    {
        return Err(tr("Enter a gRPC host and port, such as localhost:5000.").to_string());
    }
    let (service, method) = request
        .method
        .split_once('/')
        .ok_or_else(|| tr("Enter a gRPC method as package.Service/Method.").to_string())?;
    if service.is_empty() || method.is_empty() || method.contains('/') {
        return Err(tr("Enter a gRPC method as package.Service/Method.").to_string());
    }
    if request.json.len() > 1024 * 1024 {
        return Err(tr("gRPC request exceeds 1 MiB.").to_string());
    }
    serde_json::from_str::<serde_json::Value>(&request.json).map_err(|error| {
        trf(
            "Invalid request JSON: {error}",
            &[("error", &error.to_string())],
        )
    })?;
    let mut args = vec!["-max-time".into(), "60".into(), "-d".into(), "@".into()];
    if request.plaintext {
        args.push("-plaintext".into());
    }
    if let Some(proto) = &request.proto {
        let import = request.import_path.as_deref().or_else(|| proto.parent());
        let proto_arg = import
            .and_then(|root| proto.strip_prefix(root).ok())
            .unwrap_or(proto.as_path());
        args.push("-proto".into());
        args.push(proto_arg.to_string_lossy().into_owned());
        if let Some(import) = import {
            args.push("-import-path".into());
            args.push(import.to_string_lossy().into_owned());
        }
    }
    for line in request
        .metadata
        .lines()
        .filter(|line| !line.trim().is_empty())
    {
        if !line.contains(':') || line.contains(['\r', '\0']) {
            return Err(tr("Metadata must use one Key: Value entry per line.").to_string());
        }
        args.push("-H".into());
        args.push(line.trim().into());
    }
    args.push(request.endpoint.clone());
    args.push(request.method.clone());
    Ok(args)
}

fn run(args: &[String], json: &str, cancel: &AtomicBool) -> Result<String, String> {
    run_with_program(Path::new("grpcurl"), args, json, cancel)
}

fn run_with_program(
    program: &Path,
    args: &[String],
    json: &str,
    cancel: &AtomicBool,
) -> Result<String, String> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                tr("Install grpcurl to send gRPC requests.").to_string()
            } else {
                trf(
                    "Could not start grpcurl: {error}",
                    &[("error", &error.to_string())],
                )
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
    let started = Instant::now();
    let status = loop {
        if cancel.load(Ordering::Relaxed) || started.elapsed() > Duration::from_secs(65) {
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
        Ok(stdout)
    } else {
        Err(if stderr.trim().is_empty() {
            tr("gRPC request failed.").to_string()
        } else {
            stderr
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_reflection_and_proto_requests_without_a_shell() {
        let request = Request {
            endpoint: "localhost:5001".into(),
            method: "shop.Catalog/Get".into(),
            json: "{}".into(),
            metadata: "Authorization: Bearer token".into(),
            plaintext: false,
            proto: None,
            import_path: None,
        };
        let args = arguments(&request).unwrap();
        assert_eq!(
            &args[args.len() - 2..],
            ["localhost:5001", "shop.Catalog/Get"]
        );
        assert!(
            args.windows(2)
                .any(|pair| pair == ["-H", "Authorization: Bearer token"])
        );
        let with_proto = Request {
            proto: Some(PathBuf::from("/tmp/shop.proto")),
            plaintext: true,
            ..request
        };
        let args = arguments(&with_proto).unwrap();
        assert!(args.windows(2).any(|pair| pair == ["-proto", "shop.proto"]));
        assert!(args.windows(2).any(|pair| pair == ["-import-path", "/tmp"]));
        assert!(args.contains(&"-plaintext".to_string()));
    }

    #[test]
    fn rejects_bad_method_and_json() {
        let mut request = Request {
            endpoint: "localhost:5000".into(),
            method: "Bad".into(),
            json: "{}".into(),
            metadata: String::new(),
            plaintext: true,
            proto: None,
            import_path: None,
        };
        assert!(arguments(&request).is_err());
        request.method = "shop.Service/Call".into();
        request.json = "{".into();
        assert!(arguments(&request).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn sends_json_on_stdin_and_reads_the_response() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("fake-grpcurl");
        std::fs::write(&program, "#!/bin/sh\ncat\n").unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
        let result =
            run_with_program(&program, &[], "{\"id\":42}", &AtomicBool::new(false)).unwrap();
        assert_eq!(result, "{\"id\":42}");
    }
}
