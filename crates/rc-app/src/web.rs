//! Web tools: the HTTP Client (`.http` files: ▶ in the gutter sends a request, the response
//! shows in its tool window) and the Endpoints window (routes of controllers and minimal APIs).

use rc_core::Waker;
use rc_dotnet::endpoints::Endpoint;
use rc_dotnet::http_file::{self, HttpResponse};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};

pub fn is_http_file(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("http") || e.eq_ignore_ascii_case("rest"))
}

/// The last request sent and its response.
pub struct HttpClient {
    pub env: Option<String>,
    pub request: Option<String>,
    pub response: Option<Result<HttpResponse, String>>,
    pub body: Option<rc_editor::EditorState>,
    pub show_headers: bool,
    pub environments: Vec<String>,
    pub environments_loading: bool,
    env_path: Option<PathBuf>,
    env_pending: Option<Receiver<Vec<String>>>,
    env_task: Option<tokio::task::AbortHandle>,
    pending: Option<Receiver<Result<(String, HttpResponse), String>>>,
    task: Option<tokio::task::AbortHandle>,
    waker: Waker,
    next_id: u64,
}

impl HttpClient {
    pub fn new(waker: Waker) -> Self {
        Self { env: None, request: None, response: None, body: None, show_headers: false,
            environments: Vec::new(), environments_loading: false, env_path: None, env_pending: None,
            env_task: None, pending: None, task: None, waker, next_id: 1 << 40 }
    }
    pub fn busy(&self) -> bool { self.pending.is_some() }
    pub fn cancel(&mut self) {
        if let Some(task) = self.task.take() { task.abort(); }
        if self.pending.take().is_some() { self.response = Some(Err(rc_kit::i18n::tr("Cancelled").into())); }
    }
    /// Changing files invalidates both cached names and selection. Refresh preserves a valid selection.
    pub fn discover_environments(&mut self, path: Option<&Path>, refresh: bool) {
        if !refresh && self.env_path.as_deref() == path { return; }
        if let Some(task) = self.env_task.take() { task.abort(); }
        self.env_pending = None;
        self.environments_loading = false;
        if self.env_path.as_deref() != path { self.env = None; self.environments.clear(); }
        self.env_path = path.map(Path::to_path_buf);
        let Some(path) = path.map(Path::to_path_buf) else { return };
        let (tx, rx) = channel();
        let waker = self.waker.clone();
        let task = rc_core::Runtime::spawn(async move {
            if let Ok(names) = tokio::task::spawn_blocking(move || http_file::environments(&path)).await {
                let _ = tx.send(names); waker.wake();
            }
        });
        self.env_task = Some(task.abort_handle()); self.env_pending = Some(rx); self.environments_loading = true;
    }
    /// Resolution (environment and body files) runs off the UI thread. Aborting prevents a
    /// completed blocking resolution from sending a request, and drops in-flight sockets.
    pub fn send(&mut self, path: &Path, text: &str, line: usize) -> Result<(), String> {
        if text.len() > 1024 * 1024 { return Err(rc_kit::i18n::tr("HTTP input exceeds 1 MiB").into()); }
        // The gutter can send while this tool window is hidden, before its next render.
        self.discover_environments(Some(path), false);
        self.cancel();
        self.request = Some(rc_kit::i18n::tr("Preparing request…").into()); self.response = None; self.body = None;
        let (path, text, env) = (path.to_path_buf(), text.to_owned(), self.env.clone());
        let (tx, rx) = channel(); let waker = self.waker.clone();
        let task = rc_core::Runtime::spawn(async move {
            let result = async {
                let req = tokio::task::spawn_blocking(move || {
                    let file = http_file::parse(&text);
                    let index = file.request_at_line(line).ok_or_else(|| anyhow::anyhow!(rc_kit::i18n::tr("No request at this line")))?;
                    http_file::resolve(&file, index, env.as_deref(), &path)
                }).await??;
                let label = format!("{} {}", req.method, req.url);
                let response = http_file::send(&req, std::time::Duration::from_secs(60)).await?;
                Ok::<_, anyhow::Error>((label, response))
            }.await.map_err(|e| format!("{e:#}"));
            let _ = tx.send(result); waker.wake();
        });
        self.task = Some(task.abort_handle()); self.pending = Some(rx); Ok(())
    }
    pub fn poll(&mut self) -> bool {
        if let Some(rx) = &self.env_pending {
            match rx.try_recv() {
                Ok(names) => {
                    self.environments = names;
                    if self.env.as_ref().is_some_and(|selected| !self.environments.contains(selected)) { self.env = None; }
                    self.env_pending = None; self.env_task = None; self.environments_loading = false;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => { self.env_pending = None; self.env_task = None; self.environments_loading = false; }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
        let Some(rx) = &self.pending else { return false };
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(std::sync::mpsc::TryRecvError::Empty) => return false,
            Err(_) => Err(rc_kit::i18n::tr("HTTP worker stopped").into()),
        };
        self.pending = None; self.task = None;
        self.response = Some(match result {
            Ok((label, response)) => { self.request = Some(label); Ok(response) }
            Err(error) => Err(error),
        });
        if let Some(Ok(resp)) = &self.response {
            let json = resp.content_type.as_deref().is_some_and(|c| c.contains("json"));
            let name = if json { "response.json" } else if resp.content_type.as_deref().is_some_and(|c| c.contains("html")) { "response.html" } else { "response.txt" };
            self.next_id += 1;
            let mut editor = rc_editor::EditorState::new(self.next_id, Some(PathBuf::from(name)), &http_file::pretty_body(resp));
            editor.read_only = true;
            // Large responses stay unparsed; syntax parsing is bounded on the UI thread.
            if resp.body.len() < 64 * 1024 { editor.parse_blocking(); }
            self.body = Some(editor);
        }
        true
    }
}
impl Drop for HttpClient {
    fn drop(&mut self) { self.cancel(); if let Some(task) = self.env_task.take() { task.abort(); } }
}

/// A project's name, file and endpoints.
pub type ProjectEndpoints = (String, PathBuf, Vec<Endpoint>);

/// Endpoints of the solution's projects, scanned in the background.
pub struct Endpoints {
    pub projects: Vec<ProjectEndpoints>,
    pub scanning: bool,
    pub filter: String,
    pub method: String,
    pub group_by_method: bool,
    pub conflicts: Vec<(String, Endpoint, Endpoint)>,
    pub only_conflicts: bool,
    pub import_path: String,
    pub output_path: String,
    pub importing: bool,
    pub import_result: Option<Result<(PathBuf, usize), String>>,
    scan_cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    import_cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    pending_import: Option<Receiver<Result<(PathBuf, usize), String>>>,
    pending: Option<Receiver<(Vec<ProjectEndpoints>, Vec<(String, Endpoint, Endpoint)>)>>,
}

impl Endpoints {
    pub fn new() -> Self {
        Self { projects: Vec::new(), scanning: false, filter: String::new(), method: String::new(), group_by_method: false, conflicts: Vec::new(), only_conflicts: false,
            import_path: String::new(), output_path: String::new(), importing: false, import_result: None,
            scan_cancel: None, import_cancel: None, pending_import: None, pending: None }
    }
    pub fn cancel_scan(&mut self) {
        if let Some(cancel) = self.scan_cancel.take() { cancel.store(true, std::sync::atomic::Ordering::Relaxed); }
        self.pending = None; self.scanning = false;
    }
    pub fn refresh(&mut self, projects: Vec<(String, PathBuf)>, waker: Waker) {
        self.cancel_scan();
        let (tx, rx) = channel();
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.scan_cancel = Some(cancel.clone());
        std::thread::spawn(move || {
            let mut found = Vec::new(); let mut conflicts = Vec::new();
            let mut total = 0;
            for (name, path) in projects.into_iter().take(512) {
                if cancel.load(std::sync::atomic::Ordering::Relaxed) { return; }
                let Some(dir) = path.parent() else { continue };
                let mut endpoints = rc_dotnet::endpoints::scan_project_cancellable(dir, &cancel);
                endpoints.truncate(10000 - total); total += endpoints.len();
                for issue in rc_dotnet::endpoints::route_issues(&endpoints) {
                    if conflicts.len() >= 1000 { break; }
                    conflicts.push((name.clone(), endpoints[issue.first].clone(), endpoints[issue.second].clone()));
                }
                if !endpoints.is_empty() { found.push((name, path, endpoints)); }
                if total >= 10000 { break; }
            }
            if !cancel.load(std::sync::atomic::Ordering::Relaxed) { let _ = tx.send((found, conflicts)); waker.wake(); }
        });
        self.pending = Some(rx); self.scanning = true;
    }
    pub fn cancel_import(&mut self) {
        if let Some(cancel) = &self.import_cancel { cancel.store(true, std::sync::atomic::Ordering::Relaxed); }
        // Keep the slot until the worker acknowledges cancellation: a replacement append
        // must not race an earlier worker reading and rewriting the same request file.
        self.import_result = None;
    }
    pub fn import_openapi(&mut self, waker: Waker) {
        if self.importing { return; }
        let source = PathBuf::from(self.import_path.trim()); let output = PathBuf::from(self.output_path.trim());
        self.start_file_task(waker, move |cancel| {
            if !is_http_file(&output) { return Err(rc_kit::i18n::tr("Output must use .http or .rest").into()); }
            let bytes = http_file::read_bounded_file(&source, 2 * 1024 * 1024).map_err(|e| e.to_string())?;
            let source = String::from_utf8(bytes).map_err(|e| e.to_string())?;
            let text = rc_dotnet::openapi::requests(&source).map_err(|e| e.to_string())?;
            if cancel.load(std::sync::atomic::Ordering::Relaxed) { return Err(rc_kit::i18n::tr("Cancelled").into()); }
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&output).map_err(|e| e.to_string())?;
            if let Err(error) = file.write_all(text.as_bytes()) { drop(file); let _ = std::fs::remove_file(&output); return Err(error.to_string()); }
            Ok((output, 0))
        });
    }
    pub fn generate(&mut self, project: PathBuf, endpoint: Endpoint, waker: Waker) {
        if self.importing { return; }
        self.start_file_task(waker, move |cancel| {
            if cancel.load(std::sync::atomic::Ordering::Relaxed) { return Err(rc_kit::i18n::tr("Cancelled").into()); }
            generate_request_cancellable(&project, &endpoint, &cancel).map_err(|e| e.to_string())
        });
    }
    fn start_file_task(&mut self, waker: Waker, work: impl FnOnce(std::sync::Arc<std::sync::atomic::AtomicBool>) -> Result<(PathBuf, usize), String> + Send + 'static) {
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)); self.import_cancel = Some(cancel.clone());
        let (tx, rx) = channel(); self.pending_import = Some(rx); self.importing = true; self.import_result = None;
        std::thread::spawn(move || { let result = work(cancel); let _ = tx.send(result); waker.wake(); });
    }
    pub fn poll(&mut self) {
        if let Some(rx) = &self.pending {
            match rx.try_recv() {
                Ok((projects, conflicts)) => { self.projects = projects; self.conflicts = conflicts; self.pending = None; self.scan_cancel = None; self.scanning = false; }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => { self.pending = None; self.scan_cancel = None; self.scanning = false; self.import_result = Some(Err(rc_kit::i18n::tr("Endpoint worker stopped").into())); }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
        if let Some(rx) = &self.pending_import {
            match rx.try_recv() {
                Ok(result) => {
                    if !self.import_cancel.as_ref().is_some_and(|cancel| cancel.load(std::sync::atomic::Ordering::Relaxed)) { self.import_result = Some(result); }
                    self.importing = false; self.pending_import = None; self.import_cancel = None;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => { self.pending_import = None; self.import_cancel = None; self.importing = false; self.import_result = Some(Err(rc_kit::i18n::tr("Endpoint worker stopped").into())); }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
    }
}
impl Drop for Endpoints { fn drop(&mut self) { self.cancel_scan(); self.cancel_import(); } }

pub fn endpoint_matches(endpoint: &Endpoint, query: &str, method: &str) -> bool {
    if !method.is_empty() && endpoint.method != method { return false; }
    let haystack = format!("{} {} {} {}", endpoint.method, endpoint.route, endpoint.handler, endpoint.file.display()).to_lowercase();
    query.split_whitespace().all(|word| haystack.contains(&word.to_lowercase()))
}

/// "Generate Request in HTTP Client": appends a request for `ep` to `<Project>.http` next to the
/// project (Visual Studio's convention), with a `@host` from launchSettings.json; returns the
/// file and the line of the new request.
#[cfg(test)]
fn generate_request(project: &Path, ep: &Endpoint) -> std::io::Result<(PathBuf, usize)> {
    generate_request_cancellable(project, ep, &std::sync::atomic::AtomicBool::new(false))
}

fn generate_request_cancellable(project: &Path, ep: &Endpoint, cancel: &std::sync::atomic::AtomicBool) -> std::io::Result<(PathBuf, usize)> {
    if cancel.load(std::sync::atomic::Ordering::Relaxed) { return Err(std::io::Error::new(std::io::ErrorKind::Interrupted, rc_kit::i18n::tr("Cancelled"))); }
    let dir = project.parent().unwrap_or(Path::new("."));
    let name = project.file_stem().map_or_else(|| "requests".to_string(), |s| s.to_string_lossy().into_owned());
    let file = dir.join(format!("{name}.http"));
    let mut text = match http_file::read_bounded_file(&file, 4 * 1024 * 1024) {
        Ok(bytes) => String::from_utf8(bytes).map_err(std::io::Error::other)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error),
    };
    // Visual Studio's template: `@Shop.Api_HostAddress`.
    let host_var = format!("{name}_HostAddress");
    if !text.contains(&format!("@{host_var}")) {
        let host = http_file::read_bounded_file(&rc_dotnet::LaunchSettings::path_for(project), 1024 * 1024)
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .and_then(|text| rc_dotnet::LaunchSettings::parse(&text).ok())
            .and_then(|s| s.profiles.iter().flat_map(|p| p.urls()).find(|u| u.starts_with("http://")).or_else(|| s.profiles.iter().flat_map(|p| p.urls()).next()))
            .unwrap_or_else(|| "http://localhost:5000".into());
        text = format!("@{host_var} = {host}\n\n{text}");
    }
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    if !text.ends_with("\n\n") {
        text.push('\n');
    }
    // Route parameters get a sample value; constraints are dropped ({id:int} → 1).
    let mut route = String::new();
    let mut chars = ep.route.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '{' {
            let mut inner = String::new();
            for d in chars.by_ref() {
                if d == '}' {
                    break;
                }
                inner.push(d);
            }
            let param = inner.split([':', '=', '?']).next().unwrap_or("").to_lowercase();
            route.push_str(if param.contains("id") || inner.contains(":int") { "1" } else { "value" });
        } else {
            route.push(c);
        }
    }
    let method = if ep.method == "ANY" { "GET" } else { ep.method.as_str() };
    let line = text.lines().count() + 1;
    text.push_str(&format!("### {} {}\n{method} {{{{{host_var}}}}}{route}\nAccept: application/json\n", ep.method, ep.route));
    if matches!(method, "POST" | "PUT" | "PATCH") {
        text.push_str("Content-Type: application/json\n\n{\n}\n");
    }
    if cancel.load(std::sync::atomic::Ordering::Relaxed) { return Err(std::io::Error::new(std::io::ErrorKind::Interrupted, rc_kit::i18n::tr("Cancelled"))); }
    std::fs::write(&file, text)?;
    Ok((file, line))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wait(mut done: impl FnMut() -> bool) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !done() { assert!(std::time::Instant::now() < deadline, "worker timed out"); std::thread::sleep(std::time::Duration::from_millis(5)); }
    }

    #[test]
    fn http_cancel_and_drop_close_active_connections() {
        use std::io::Read;
        for drop_client in [false, true] {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let address = listener.local_addr().unwrap();
            let (started_tx, started_rx) = channel(); let (closed_tx, closed_rx) = channel();
            let server = std::thread::spawn(move || {
                let mut connection = None;
                wait(|| match listener.accept() { Ok((stream, _)) => { connection = Some(stream); true }, Err(_) => false });
                let mut stream = connection.unwrap();
                stream.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
                let mut data = [0; 4096];
                assert!(stream.read(&mut data).unwrap() > 0);
                started_tx.send(()).unwrap();
                closed_tx.send(matches!(stream.read(&mut data), Ok(0))).unwrap();
            });
            let mut client = HttpClient::new(Waker::noop());
            client.send(Path::new("/tmp/request.http"), &format!("GET http://{address}/slow"), 0).unwrap();
            started_rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
            if drop_client { drop(client); } else {
                client.cancel(); assert!(!client.busy()); assert!(client.response.as_ref().unwrap().is_err());
                assert!(!client.poll(), "a cancelled request cannot later replace its result");
            }
            assert!(closed_rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap());
            server.join().unwrap();
        }
    }

    #[test]
    fn http_response_limit_returns_error_instead_of_allocating_unbounded_body() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap(); listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let mut connection = None;
            wait(|| match listener.accept() { Ok((stream, _)) => { connection = Some(stream); true }, Err(_) => false });
            let mut stream = connection.unwrap(); stream.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
            stream.set_write_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
            assert!(stream.read(&mut [0; 4096]).unwrap() > 0);
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 9437184\r\nConnection: close\r\n\r\n").unwrap();
            for _ in 0..9 { if stream.write_all(&vec![b'x'; 1024 * 1024]).is_err() { break; } }
        });
        let mut client = HttpClient::new(Waker::noop());
        client.send(Path::new("/tmp/large.http"), &format!("GET http://{address}/large"), 0).unwrap();
        wait(|| { client.poll(); !client.busy() });
        assert!(client.response.as_ref().unwrap().as_ref().unwrap_err().contains("8 MiB")); assert!(client.body.is_none());
        server.join().unwrap();
    }

    #[test]
    fn http_environment_refresh_and_file_body_are_resolved_by_worker() {
        use std::io::{Read, Write};
        let dir = tempfile::tempdir().unwrap(); let path = dir.path().join("requests.http");
        let env_file = dir.path().join(http_file::ENV_FILE);
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        std::fs::write(&env_file, serde_json::json!({"dev":{"host":format!("http://{address}")}}).to_string()).unwrap();
        std::fs::write(dir.path().join("body.json"), "{\"from\":\"file\"}").unwrap();
        let (request_tx, request_rx) = channel();
        let server = std::thread::spawn(move || {
            let mut connection = None;
            wait(|| match listener.accept() { Ok((stream, _)) => { connection = Some(stream); true }, Err(_) => false });
            let mut stream = connection.unwrap(); stream.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
            let mut data = Vec::new(); let mut chunk = [0; 1024];
            while !String::from_utf8_lossy(&data).contains("{\"from\":\"file\"}") {
                let len = stream.read(&mut chunk).unwrap(); assert!(len > 0); data.extend_from_slice(&chunk[..len]);
            }
            request_tx.send(String::from_utf8(data).unwrap()).unwrap();
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 11\r\nConnection: close\r\n\r\n{\"ok\":true}").unwrap();
        });
        let mut client = HttpClient::new(Waker::noop());
        client.discover_environments(Some(&path), false);
        wait(|| { client.poll(); !client.environments_loading });
        assert_eq!(client.environments, ["dev"]); client.env = Some("dev".into());
        client.send(&path, "POST {{host}}/body\nContent-Type: application/json\n\n< body.json", 0).unwrap();
        wait(|| { client.poll(); !client.busy() });
        let request = request_rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        assert!(request.starts_with("POST /body HTTP/1.1")); assert!(request.ends_with("{\"from\":\"file\"}"));
        assert_eq!(client.response.as_ref().unwrap().as_ref().unwrap().status, 200);
        assert!(client.body.as_ref().unwrap().read_only);
        server.join().unwrap();
        std::fs::write(&env_file, r#"{"prod":{"host":"http://localhost"}}"#).unwrap();
        client.discover_environments(Some(&path), true);
        wait(|| { client.poll(); !client.environments_loading });
        assert_eq!(client.environments, ["prod"]); assert!(client.env.is_none());
        client.env = Some("prod".into());
        client.discover_environments(None, false);
        assert!(client.environments.is_empty()); assert!(client.env.is_none());
    }

    #[test]
    fn endpoints_search_import_and_cancel_have_reviewable_results() {
        let dir = tempfile::tempdir().unwrap(); let source = dir.path().join("api.json"); let output = dir.path().join("imported.http");
        std::fs::write(&source, r#"{"openapi":"3.0.3","servers":[{"url":"http://localhost:5000"}],"paths":{"/orders":{"get":{}}}}"#).unwrap();
        let mut endpoints = Endpoints::new(); endpoints.import_path = source.display().to_string(); endpoints.output_path = output.display().to_string();
        endpoints.import_openapi(Waker::noop()); wait(|| { endpoints.poll(); !endpoints.importing });
        assert!(endpoints.import_result.take().unwrap().is_ok());
        let original = std::fs::read_to_string(&output).unwrap(); assert_eq!(http_file::parse(&original).requests.len(), 1);
        endpoints.import_openapi(Waker::noop()); wait(|| { endpoints.poll(); !endpoints.importing });
        assert!(endpoints.import_result.take().unwrap().is_err()); assert_eq!(std::fs::read_to_string(&output).unwrap(), original);
        let endpoint = Endpoint { method: "GET".into(), route: "/orders/{id}".into(), file: dir.path().join("Orders.cs"), line: 0, handler: "Orders.Get".into(), kind: rc_dotnet::endpoints::EndpointKind::MinimalApi };
        assert!(endpoint_matches(&endpoint, "ORDER get .CS", "GET")); assert!(!endpoint_matches(&endpoint, "missing", "")); assert!(!endpoint_matches(&endpoint, "orders", "POST"));
        let (started_tx, started_rx) = channel(); let (release_tx, release_rx) = channel(); let task_output = output.clone();
        endpoints.start_file_task(Waker::noop(), move |cancel| { started_tx.send(()).unwrap(); release_rx.recv().unwrap(); assert!(cancel.load(std::sync::atomic::Ordering::Relaxed)); Ok((task_output, 0)) });
        started_rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap(); endpoints.cancel_import();
        assert!(endpoints.importing, "a cancelled writer retains its slot until acknowledgement");
        let cancelled_output = dir.path().join("must-not-exist.http"); endpoints.output_path = cancelled_output.display().to_string();
        endpoints.import_openapi(Waker::noop()); assert!(!cancelled_output.exists());
        release_tx.send(()).unwrap(); wait(|| { endpoints.poll(); !endpoints.importing }); assert!(endpoints.import_result.is_none()); assert!(!cancelled_output.exists());
        let cancelled = std::sync::atomic::AtomicBool::new(true);
        let project = dir.path().join("Api.csproj");
        assert_eq!(generate_request_cancellable(&project, &endpoint, &cancelled).unwrap_err().kind(), std::io::ErrorKind::Interrupted);
        assert!(!dir.path().join("Api.http").exists());
    }

    #[test]
    fn sending_from_another_file_clears_a_hidden_windows_environment() {
        let dir = tempfile::tempdir().unwrap(); let first = dir.path().join("first.http"); let second = dir.path().join("second.http");
        let mut client = HttpClient::new(Waker::noop());
        client.discover_environments(Some(&first), false); client.env = Some("production".into());
        client.send(&second, "GET {{missing}}/items", 0).unwrap();
        assert!(client.env.is_none()); assert_eq!(client.env_path.as_deref(), Some(second.as_path()));
        client.cancel();
    }

    #[test]
    fn generated_requests_parse_and_resolve() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("Shop.Api.csproj");
        std::fs::write(&project, "<Project Sdk=\"Microsoft.NET.Sdk.Web\" />").unwrap();
        let ep = Endpoint { method: "GET".into(), route: "/orders/{id:int}".into(), file: project.clone(), line: 0, handler: "OrdersController.Get".into(), kind: rc_dotnet::endpoints::EndpointKind::Controller };
        let (file, line) = generate_request(&project, &ep).unwrap();
        let post = Endpoint { method: "POST".into(), route: "/orders".into(), ..ep.clone() };
        generate_request(&project, &post).unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        let parsed = http_file::parse(&text);
        assert_eq!(parsed.requests.len(), 2, "{text}");
        let i = parsed.request_at_line(line).unwrap();
        let r = http_file::resolve(&parsed, i, None, &file).unwrap();
        assert_eq!((r.method.as_str(), r.url.as_str()), ("GET", "http://localhost:5000/orders/1"));
        assert_eq!(text.matches("@Shop.Api_HostAddress").count(), 1);
    }
}
