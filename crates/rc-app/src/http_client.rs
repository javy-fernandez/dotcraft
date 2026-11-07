//! Explicit HTTP requests from editor text. No files, scripts, or project code are executed.
//! Inspired by CrabCraft's MIT/Apache-2.0 HTTP client; this smaller implementation shares
//! its bounded response and cancellation approach without its environment or OAuth layers.

use rc_core::{Inbox, Runtime, Waker};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use std::time::{Duration, Instant};

pub const MAX_REQUEST_BYTES: usize = 1024 * 1024;
pub const MAX_BODY_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_SHOWN_CHARS: usize = 512 * 1024;
pub const TIMEOUT: Duration = Duration::from_secs(60);
const MAX_VARIABLES: usize = 256;
const MAX_VARIABLE_DEPTH: usize = 32;
const MAX_VARIABLE_REFERENCES: usize = 4096;

/// File variables are literal text substitutions. No environment or expression evaluation.
/// Bounds follow CrabCraft's http_vars.rs approach, with an additional reference budget.
struct Variables<'a> {
    values: &'a BTreeMap<String, String>,
    cache: BTreeMap<String, String>,
    stack: Vec<String>,
    cached_bytes: usize,
    references: usize,
}

fn variable_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 128 && name.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

fn variable_limit() -> HttpError {
    HttpError::Invalid("HTTP variable expansion exceeds its size, reference, or nesting limit".into())
}

impl<'a> Variables<'a> {
    fn new(values: &'a BTreeMap<String, String>) -> Self {
        Self { values, cache: BTreeMap::new(), stack: Vec::new(), cached_bytes: 0, references: 0 }
    }

    fn variable(&mut self, name: &str) -> Result<String, HttpError> {
        self.references += 1;
        if self.references > MAX_VARIABLE_REFERENCES || self.stack.len() >= MAX_VARIABLE_DEPTH { return Err(variable_limit()); }
        if let Some(value) = self.cache.get(name) { return Ok(value.clone()); }
        if self.stack.iter().any(|entry| entry == name) {
            return Err(HttpError::Invalid(format!("cyclic HTTP variable: {name}")));
        }
        let raw = self.values.get(name).ok_or_else(|| HttpError::Invalid(format!("undefined HTTP variable: {name}")))?;
        self.stack.push(name.to_string());
        let result = self.expand(raw);
        self.stack.pop();
        let expanded = result?;
        if expanded.len() > MAX_REQUEST_BYTES.saturating_sub(self.cached_bytes) { return Err(variable_limit()); }
        self.cached_bytes += expanded.len();
        self.cache.insert(name.to_string(), expanded.clone());
        Ok(expanded)
    }

    fn expand(&mut self, text: &str) -> Result<String, HttpError> {
        let mut rest = text;
        let mut output = String::new();
        let append = |output: &mut String, value: &str| {
            if value.len() > MAX_REQUEST_BYTES.saturating_sub(output.len()) { return Err(variable_limit()); }
            output.push_str(value);
            Ok(())
        };
        while let Some(start) = rest.find("{{") {
            append(&mut output, &rest[..start])?;
            let after = &rest[start + 2..];
            let end = after.find("}}").ok_or_else(|| HttpError::Invalid("unclosed HTTP variable placeholder".into()))?;
            let name = after[..end].trim();
            if !variable_name(name) { return Err(HttpError::Invalid("invalid HTTP variable name; expressions are not supported".into())); }
            append(&mut output, &self.variable(name)?)?;
            rest = &after[end + 2..];
        }
        append(&mut output, rest)?;
        Ok(output)
    }
}

#[derive(Clone, Debug)]
pub struct Request {
    pub line: usize,
    pub method: reqwest::Method,
    pub url: reqwest::Url,
    pub headers: reqwest::header::HeaderMap,
    pub body: String,
}

impl Request {
    pub fn title(&self) -> String { format!("{} {}", self.method, self.url) }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HttpError {
    Invalid(String),
    Network(String),
    TimedOut,
    Cancelled,
}

/// Parse a deliberately small HTTP-file subset. Unsupported constructs block execution
/// rather than silently changing what the author intended to send.
pub fn parse_requests(text: &str) -> Result<Vec<Request>, HttpError> {
    let invalid = |message: &str| HttpError::Invalid(message.to_string());
    if text.len() > MAX_REQUEST_BYTES {
        return Err(invalid("request file exceeds 1 MiB"));
    }
    let mut blocks = vec![(0, Vec::<&str>::new())];
    for (line, raw) in text.split('\n').enumerate() {
        let raw = raw.strip_suffix('\r').unwrap_or(raw);
        if raw.trim_start().starts_with("###") {
            blocks.push((line + 1, Vec::new()));
        } else if let Some((_, lines)) = blocks.last_mut() {
            lines.push(raw);
        }
    }
    let mut variables = BTreeMap::new();
    for (_, lines) in &mut blocks {
        let mut before_request = true;
        for raw in lines {
            let line = raw.trim();
            if before_request && line.starts_with('@') {
                let (name, value) = line[1..].split_once('=').ok_or_else(|| invalid("expected @name = value before a request"))?;
                let name = name.trim();
                if !variable_name(name) { return Err(invalid("invalid HTTP file variable name")); }
                if variables.len() >= MAX_VARIABLES { return Err(variable_limit()); }
                if variables.insert(name.to_string(), value.trim().to_string()).is_some() {
                    return Err(HttpError::Invalid(format!("duplicate HTTP variable: {name}")));
                }
                // Preserve source line numbers while excluding declarations from requests.
                *raw = "";
            } else if !line.is_empty() && !line.starts_with('#') && !line.starts_with("//") {
                before_request = false;
            }
        }
    }
    let mut resolver = Variables::new(&variables);
    let mut expanded_bytes = 0usize;
    let mut requests = Vec::new();
    for (start, lines) in blocks {
        let Some(first) = lines.iter().position(|line| {
            let line = line.trim();
            !line.is_empty() && !line.starts_with('#') && !line.starts_with("//")
        }) else { continue };
        for line in &lines {
            let trimmed = line.trim_start();
            if trimmed.starts_with('@')
                || trimmed.starts_with("< ") || trimmed.starts_with("<\t")
                || trimmed.starts_with("> ") || trimmed.starts_with(">\t") || trimmed.starts_with(">>")
                || trimmed.starts_with("# @") || trimmed.starts_with("// @")
            {
                return Err(invalid("directives, file bodies, scripts, and declarations after a request line are not supported"));
            }
        }
        let request_line = resolver.expand(lines[first])?;
        let parts: Vec<_> = request_line.split_whitespace().collect();
        let (method, url) = match parts.as_slice() {
            [url] if url.starts_with("http://") || url.starts_with("https://") => ("GET", *url),
            [method, url] => (*method, *url),
            _ => return Err(invalid("expected METHOD URL or an absolute HTTP URL; explicit protocol versions are not supported")),
        };
        if !matches!(method, "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD" | "OPTIONS" | "TRACE" | "CONNECT") {
            return Err(invalid("unsupported HTTP method"));
        }
        let method = reqwest::Method::from_bytes(method.as_bytes()).map_err(|_| invalid("invalid method"))?;
        let url = reqwest::Url::parse(url).map_err(|_| invalid("invalid absolute URL"))?;
        if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() || !url.username().is_empty() || url.password().is_some() {
            return Err(invalid("use an absolute HTTP or HTTPS URL without embedded credentials"));
        }
        let mut headers = reqwest::header::HeaderMap::new();
        let mut body_at = lines.len();
        for (index, line) in lines.iter().enumerate().skip(first + 1) {
            if line.trim().is_empty() { body_at = index + 1; break; }
            if line.trim_start().starts_with('#') || line.trim_start().starts_with("//") { continue; }
            let (name, value) = line.split_once(':').ok_or_else(|| invalid("expected a header or a blank line before the body"))?;
            let name = resolver.expand(name.trim())?;
            let value = resolver.expand(value.trim())?;
            expanded_bytes = expanded_bytes.saturating_add(name.len()).saturating_add(value.len());
            if expanded_bytes > MAX_REQUEST_BYTES { return Err(variable_limit()); }
            let name = reqwest::header::HeaderName::from_bytes(name.as_bytes()).map_err(|_| invalid("invalid header name after variable interpolation"))?;
            let value = reqwest::header::HeaderValue::from_str(&value).map_err(|_| invalid("invalid header value after variable interpolation"))?;
            headers.append(name, value);
        }
        let body = resolver.expand(&lines[body_at..].join("\n"))?;
        expanded_bytes = expanded_bytes.saturating_add(request_line.len()).saturating_add(body.len());
        if expanded_bytes > MAX_REQUEST_BYTES { return Err(variable_limit()); }
        requests.push(Request { line: start + first, method, url, headers, body });
    }
    Ok(requests)
}

#[derive(Debug)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub truncated: bool,
    pub elapsed: Duration,
}

/// Only retained body bytes are bounded by `max`; the network stack owns its read buffers.
pub async fn send(request: Request, timeout: Duration, max: usize, cancel: Arc<AtomicBool>) -> Result<Response, HttpError> {
    let work = async {
        let mut builder = reqwest::Client::builder()
            .user_agent(concat!("DotCraft/", env!("CARGO_PKG_VERSION")))
            .redirect(reqwest::redirect::Policy::none())
            .timeout(timeout);
        if matches!(request.url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]")) {
            builder = builder.no_proxy();
        }
        let client = builder.build().map_err(network_error)?;
        let started = Instant::now();
        let mut response = client.request(request.method, request.url).headers(request.headers).body(request.body).send().await.map_err(network_error)?;
        let status = response.status().as_u16();
        let headers = response.headers().iter().map(|(key, value)| (key.to_string(), String::from_utf8_lossy(value.as_bytes()).into_owned())).collect();
        let mut body = Vec::new();
        let mut truncated = false;
        while let Some(chunk) = response.chunk().await.map_err(network_error)? {
            let remaining = max.saturating_sub(body.len());
            body.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
            if chunk.len() > remaining { truncated = true; break; }
        }
        Ok(Response { status, headers, body, truncated, elapsed: started.elapsed() })
    };
    tokio::select! {
        biased;
        _ = async {
            while !cancel.load(Ordering::Relaxed) { tokio::time::sleep(Duration::from_millis(25)).await; }
        } => Err(HttpError::Cancelled),
        result = tokio::time::timeout(timeout, work) => result.unwrap_or(Err(HttpError::TimedOut)),
    }
}

fn network_error(error: reqwest::Error) -> HttpError {
    if error.is_timeout() { HttpError::TimedOut } else { HttpError::Network(error.to_string()) }
}

pub struct HttpClient {
    inbox: Inbox<Result<Response, HttpError>>,
    cancel: Option<Arc<AtomicBool>>,
    pub selected: usize,
    pub title: String,
    pub response: Option<Response>,
    pub error: Option<HttpError>,
    parsed: Option<(PathBuf, u64, u64, Result<Vec<Request>, HttpError>)>,
}

impl HttpClient {
    pub fn new(waker: Waker) -> Self {
        Self { inbox: Inbox::new(waker), cancel: None, selected: 0, title: String::new(), response: None, error: None, parsed: None }
    }

    /// Materialize and parse editor text only when the active document version changes.
    pub fn prepare(&mut self, path: &Path, document: u64, version: u64, source: impl FnOnce() -> Result<String, HttpError>) {
        if self.parsed.as_ref().is_some_and(|(cached, id, revision, _)| cached == path && *id == document && *revision == version) { return; }
        self.parsed = Some((path.to_path_buf(), document, version, source().and_then(|text| parse_requests(&text))));
    }

    pub fn requests(&self) -> Option<&Result<Vec<Request>, HttpError>> {
        self.parsed.as_ref().map(|(_, _, _, requests)| requests)
    }

    pub fn running(&self) -> bool { self.cancel.is_some() }

    pub fn start(&mut self, request: Request) {
        if self.running() { return; }
        self.title = request.title();
        self.response = None;
        self.error = None;
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = Some(cancel.clone());
        let out = self.inbox.sender();
        Runtime::spawn(async move { out.send(send(request, TIMEOUT, MAX_BODY_BYTES, cancel).await); });
    }

    pub fn cancel(&self) {
        if let Some(cancel) = &self.cancel { cancel.store(true, Ordering::Relaxed); }
    }

    pub fn update(&mut self) {
        for result in self.inbox.drain() {
            self.cancel = None;
            match result {
                Ok(response) => self.response = Some(response),
                Err(error) => self.error = Some(error),
            }
        }
    }
}

impl Drop for HttpClient {
    fn drop(&mut self) { self.cancel(); }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    fn request(url: &str) -> Request {
        parse_requests(&format!("POST {url}\nX-Example: value\nContent-Type: text/plain\n\nhello")).unwrap().remove(0)
    }

    #[test]
    fn cached_selection_tracks_edits_and_reopened_documents() {
        let mut client = HttpClient::new(Waker::noop());
        let path = Path::new("sample.http");
        client.prepare(path, 1, 0, || Ok("GET https://example.com/first".into()));
        client.prepare(path, 1, 0, || panic!("unchanged frames must not materialize the buffer"));
        assert_eq!(client.requests().unwrap().as_ref().unwrap()[0].url.path(), "/first");
        client.prepare(path, 1, 1, || Ok("GET https://example.com/edited".into()));
        assert_eq!(client.requests().unwrap().as_ref().unwrap()[0].url.path(), "/edited");
        // A reopened file may have the same buffer version but a new document identity.
        client.prepare(path, 2, 1, || Ok("GET https://example.com/reopened".into()));
        assert_eq!(client.requests().unwrap().as_ref().unwrap()[0].url.path(), "/reopened");
        client.prepare(Path::new("different.http"), 2, 1, || Ok("invalid".into()));
        assert!(client.requests().unwrap().is_err());
    }

    #[test]
    fn parses_multiple_requests_and_rejects_unsupported_syntax() {
        let parsed = parse_requests("# comment\r\nGET https://example.com/\r\n\r\n### second\r\nPOST http://localhost:123/a\r\nX-Test: yes\r\n\r\nbody").unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].line, 1);
        assert_eq!(parsed[1].line, 4);
        assert_eq!(parsed[1].body, "body");
        assert_eq!(parsed[1].headers["x-test"], "yes");
        for text in ["GET file:///tmp/private", "GET https://{{host}}", "@host\nGET https://example.com", "GET https://example.com\n\n< secret.txt", "GET https://example.com\n\n> {% script %}", "GET https://example.com HTTP/1.1", "GET https://example.com\ninvalid header"] {
            assert!(parse_requests(text).is_err(), "{text}");
        }
        assert!(parse_requests(&"x".repeat(MAX_REQUEST_BYTES + 1)).is_err());
        let xml = parse_requests("POST https://example.com\nContent-Type: application/xml\n\n<root><item>hello</item></root>").unwrap();
        assert_eq!(xml[0].body, "<root><item>hello</item></root>");
    }

    #[test]
    fn http_variables_resolve_nested_values_without_evaluating_expressions() {
        let parsed = parse_requests("@host = example.com\n@url = https://{{ host }}/api\n@header = X-Example\n@value = café\n@empty =\n### first\nPOST {{url}}\n{{header}}: {{value}}\n\n{{value}} {{empty}} $HOME $(echo untouched)\n### second\nGET {{url}}/next").unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].line, 6);
        assert_eq!(parsed[0].url.as_str(), "https://example.com/api");
        assert_eq!(parsed[0].headers["x-example"].as_bytes(), "café".as_bytes());
        assert_eq!(parsed[0].body, "café  $HOME $(echo untouched)");
        assert_eq!(parsed[1].url.as_str(), "https://example.com/api/next");
        // Forward declarations are shared across request blocks.
        let forward = parse_requests("GET {{url}}\n### definitions\n@url = https://example.com/").unwrap();
        assert_eq!(forward[0].line, 0);
        assert_eq!(forward[0].url.host_str(), Some("example.com"));
        // Consecutive JSON closing braces are literal payload, not variable syntax.
        let json = parse_requests("@value = nested\nPOST https://example.com\nContent-Type: application/json\n\n{\"outer\":{\"value\":\"{{value}}\"}}").unwrap();
        assert_eq!(json[0].body, "{\"outer\":{\"value\":\"nested\"}}");
    }

    #[test]
    fn http_variables_report_errors_and_validate_expanded_urls_and_headers() {
        let cases = [
            ("GET https://{{missing}}", "undefined HTTP variable: missing"),
            ("@a = {{b}}\n@b = {{a}}\nGET https://{{a}}", "cyclic HTTP variable"),
            ("GET https://{{host", "unclosed HTTP variable"),
            ("GET https://{{$HOME}}", "expressions are not supported"),
            ("@host = example.com\n@host = localhost\nGET https://{{host}}", "duplicate HTTP variable: host"),
            ("@url = file:///tmp/private\nGET {{url}}", "absolute HTTP or HTTPS URL"),
            ("@header = invalid name\nGET https://example.com\n{{header}}: value", "invalid header name after variable interpolation"),
            ("@value = good\rbad\nGET https://example.com\nX-Test: {{value}}", "invalid header value after variable interpolation"),
            ("GET https://example.com\n@late = value", "declarations after a request line"),
        ];
        for (source, expected) in cases {
            let error = parse_requests(source).unwrap_err();
            assert!(matches!(error, HttpError::Invalid(ref reason) if reason.contains(expected)), "{source}: {error:?}");
        }
    }

    #[test]
    fn http_variable_expansion_limits_bound_memory_and_work() {
        let mut exponential = format!("@v0 = {}\n", "x".repeat(1024));
        for index in 1..20 { exponential.push_str(&format!("@v{index} = {{{{v{}}}}}{{{{v{}}}}}\n", index - 1, index - 1)); }
        exponential.push_str("POST https://example.com\n\n{{v19}}");
        assert!(matches!(parse_requests(&exponential), Err(HttpError::Invalid(reason)) if reason.contains("limit")));

        let mut deep = "@v0 = end\n".to_string();
        for index in 1..=MAX_VARIABLE_DEPTH { deep.push_str(&format!("@v{index} = {{{{v{}}}}}\n", index - 1)); }
        deep.push_str(&format!("POST https://example.com\n\n{{{{v{MAX_VARIABLE_DEPTH}}}}}"));
        assert!(parse_requests(&deep).is_err());

        let mut many = String::new();
        for index in 0..=MAX_VARIABLES { many.push_str(&format!("@v{index} = x\n")); }
        many.push_str("GET https://example.com");
        assert!(parse_requests(&many).is_err());

        let references = format!("@empty =\nPOST https://example.com\n\n{}", "{{empty}}".repeat(MAX_VARIABLE_REFERENCES + 1));
        assert!(parse_requests(&references).is_err());
        let aggregate = format!("@large = {}\nPOST https://example.com\n\n{{{{large}}}}\n### second\nPOST https://example.com\n\n{{{{large}}}}", "x".repeat(MAX_REQUEST_BYTES / 2));
        assert!(matches!(parse_requests(&aggregate), Err(HttpError::Invalid(reason)) if reason.contains("limit")));
    }

    #[test]
    fn http_file_variables_are_substituted_in_real_localhost_requests() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut received = Vec::new();
            let mut buffer = [0; 2048];
            while !received.ends_with(b"<item>substituted</item>") {
                let size = socket.read(&mut buffer).unwrap();
                assert!(size > 0);
                received.extend_from_slice(&buffer[..size]);
            }
            let request = String::from_utf8(received).unwrap();
            assert!(request.starts_with("POST /items HTTP/1.1\r\n"));
            assert!(request.contains("x-substituted: works\r\n"));
            assert!(request.contains("content-type: application/xml\r\n"));
            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok").unwrap();
        });
        let source = format!("@host = {address}\n@base = http://{{{{host}}}}\n@header = X-Substituted\n@value = substituted\nPOST {{{{base}}}}/items\n{{{{header}}}}: works\nContent-Type: application/xml\n\n<item>{{{{value}}}}</item>");
        let request = parse_requests(&source).unwrap().remove(0);
        let response = Runtime::block_on(send(request, Duration::from_secs(5), MAX_BODY_BYTES, Arc::new(AtomicBool::new(false)))).unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(response.body, b"ok");
        server.join().unwrap();
    }

    #[test]
    fn local_server_receives_request_and_response_is_bounded() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut received = Vec::new();
            let mut buffer = [0; 1024];
            loop {
                let size = socket.read(&mut buffer).unwrap();
                assert!(size > 0);
                received.extend_from_slice(&buffer[..size]);
                if received.ends_with(b"</root>") { break; }
            }
            let text = String::from_utf8(received).unwrap();
            assert!(text.starts_with("POST /endpoint HTTP/1.1"));
            assert!(text.contains("x-example: value"));
            assert!(text.contains("content-type: application/xml"));
            assert!(text.ends_with("<root>hello</root>"));
            socket.write_all(b"HTTP/1.1 201 Created\r\nContent-Length: 10\r\nX-Reply: yes\r\nConnection: close\r\n\r\n0123456789").unwrap();
        });
        let mut xml = request(&format!("http://{address}/endpoint"));
        xml.headers.insert(reqwest::header::CONTENT_TYPE, reqwest::header::HeaderValue::from_static("application/xml"));
        xml.body = "<root>hello</root>".into();
        let response = Runtime::block_on(send(xml, Duration::from_secs(5), 4, Arc::new(AtomicBool::new(false)))).unwrap();
        assert_eq!(response.status, 201);
        assert_eq!(response.body, b"0123");
        assert!(response.truncated);
        assert!(response.headers.iter().any(|(name, value)| name == "x-reply" && value == "yes"));
        server.join().unwrap();
    }

    #[test]
    fn stalled_responses_can_be_cancelled_or_time_out() {
        for cancel_request in [true, false] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let (ready, connected) = std::sync::mpsc::channel();
            let server = std::thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                socket.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                let mut buffer = [0; 2048];
                let mut received = Vec::new();
                while !received.ends_with(b"hello") {
                    let size = socket.read(&mut buffer).unwrap();
                    assert!(size > 0);
                    received.extend_from_slice(&buffer[..size]);
                }
                socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nx").unwrap();
                ready.send(()).unwrap();
                // EOF arrives promptly when cancellation/timeout drops the response future.
                let _ = socket.read(&mut buffer);
            });
            let cancel = Arc::new(AtomicBool::new(false));
            let token = cancel.clone();
            let trigger = std::thread::spawn(move || {
                connected.recv_timeout(Duration::from_secs(5)).unwrap();
                if cancel_request { token.store(true, Ordering::Relaxed); }
            });
            let start = Instant::now();
            let result = Runtime::block_on(send(request(&format!("http://{address}/")), Duration::from_millis(250), 100, cancel));
            assert_eq!(result.unwrap_err(), if cancel_request { HttpError::Cancelled } else { HttpError::TimedOut });
            assert!(start.elapsed() < Duration::from_secs(3));
            trigger.join().unwrap();
            server.join().unwrap();
        }
    }
}
