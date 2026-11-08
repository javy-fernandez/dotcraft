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
    /// The environment of `http-client.env.json` used to resolve variables.
    pub env: Option<String>,
    /// "POST http://localhost:5112/orders" of the request in flight or answered.
    pub request: Option<String>,
    pub response: Option<Result<HttpResponse, String>>,
    /// The response body in a read-only editor (JSON or text colours).
    pub body: Option<rc_editor::EditorState>,
    pub show_headers: bool,
    pending: Option<Receiver<Result<HttpResponse, String>>>,
    waker: Waker,
    next_id: u64,
}

impl HttpClient {
    pub fn new(waker: Waker) -> Self {
        HttpClient { env: None, request: None, response: None, body: None, show_headers: false, pending: None, waker, next_id: 1 << 40 }
    }

    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }

    /// Sends the request of `text` at `line` (its block) from the file `path`.
    pub fn send(&mut self, path: &Path, text: &str, line: usize) -> Result<(), String> {
        let file = http_file::parse(text);
        let index = file.request_at_line(line).ok_or_else(|| rc_kit::i18n::tr("No request at this line").to_string())?;
        let req = http_file::resolve(&file, index, self.env.as_deref(), path).map_err(|e| format!("{e:#}"))?;
        self.request = Some(format!("{} {}", req.method, req.url));
        self.response = None;
        self.body = None;
        let (tx, rx) = channel();
        let waker = self.waker.clone();
        std::thread::spawn(move || {
            let result = rc_core::runtime::Runtime::block_on(http_file::send(&req, std::time::Duration::from_secs(60))).map_err(|e| format!("{e:#}"));
            let _ = tx.send(result);
            waker.wake();
        });
        self.pending = Some(rx);
        Ok(())
    }

    pub fn poll(&mut self) -> bool {
        let Some(rx) = &self.pending else { return false };
        let Ok(result) = rx.try_recv() else { return false };
        self.pending = None;
        if let Ok(resp) = &result {
            let json = resp.content_type.as_deref().is_some_and(|c| c.contains("json"));
            let name = if json { "response.json" } else if resp.content_type.as_deref().is_some_and(|c| c.contains("html")) { "response.html" } else { "response.txt" };
            self.next_id += 1;
            let mut e = rc_editor::EditorState::new(self.next_id, Some(PathBuf::from(name)), &http_file::pretty_body(resp));
            e.read_only = true;
            e.parse_blocking();
            self.body = Some(e);
        }
        self.response = Some(result);
        true
    }
}

/// A project's name, file and endpoints.
pub type ProjectEndpoints = (String, PathBuf, Vec<Endpoint>);

/// Endpoints of the solution's projects, scanned in the background.
pub struct Endpoints {
    pub projects: Vec<ProjectEndpoints>,
    pub scanning: bool,
    pub filter: String,
    pending: Option<Receiver<Vec<ProjectEndpoints>>>,
}

impl Endpoints {
    pub fn new() -> Self {
        Endpoints { projects: Vec::new(), scanning: false, filter: String::new(), pending: None }
    }

    pub fn refresh(&mut self, projects: Vec<(String, PathBuf)>, waker: Waker) {
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let found: Vec<ProjectEndpoints> = projects
                .into_iter()
                .filter_map(|(name, path)| {
                    let dir = path.parent()?.to_path_buf();
                    let eps = rc_dotnet::endpoints::scan_project(&dir);
                    (!eps.is_empty()).then_some((name, path, eps))
                })
                .collect();
            let _ = tx.send(found);
            waker.wake();
        });
        self.pending = Some(rx);
        self.scanning = true;
    }

    pub fn poll(&mut self) {
        if let Some(rx) = &self.pending
            && let Ok(found) = rx.try_recv()
        {
            self.projects = found;
            self.pending = None;
            self.scanning = false;
        }
    }

    pub fn count(&self) -> usize {
        self.projects.iter().map(|p| p.2.len()).sum()
    }
}

/// "Generate Request in HTTP Client": appends a request for `ep` to `<Project>.http` next to the
/// project (Visual Studio's convention), with a `@host` from launchSettings.json; returns the
/// file and the line of the new request.
pub fn generate_request(project: &Path, ep: &Endpoint) -> std::io::Result<(PathBuf, usize)> {
    let dir = project.parent().unwrap_or(Path::new("."));
    let name = project.file_stem().map_or_else(|| "requests".to_string(), |s| s.to_string_lossy().into_owned());
    let file = dir.join(format!("{name}.http"));
    let mut text = std::fs::read_to_string(&file).unwrap_or_default();
    // Visual Studio's template: `@Shop.Api_HostAddress`.
    let host_var = format!("{name}_HostAddress");
    if !text.contains(&format!("@{host_var}")) {
        let host = rc_dotnet::LaunchSettings::load(project)
            .ok()
            .flatten()
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
    std::fs::write(&file, text)?;
    Ok((file, line))
}

#[cfg(test)]
mod tests {
    use super::*;

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
