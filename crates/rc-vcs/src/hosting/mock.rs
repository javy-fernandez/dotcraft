//! A tiny local HTTP/1.1 server standing in for GitHub or Azure DevOps in tests (no test ever
//! talks to the real services).

use std::io::{BufRead, Read, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// A request the mock server received.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub method: String,
    /// Path and query, as sent.
    pub path: String,
    pub authorization: Option<String>,
    pub body: String,
}

type Handler = dyn Fn(&Request) -> (u16, String) + Send + Sync;

/// A running mock server; requests are answered by the handler and recorded.
pub struct MockServer {
    pub base: String,
    pub requests: Arc<Mutex<Vec<Request>>>,
}

impl MockServer {
    /// Starts a server on 127.0.0.1; `respond` gives (status, JSON body) per request.
    pub fn start(respond: impl Fn(&Request) -> (u16, String) + Send + Sync + 'static) -> MockServer {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind mock server");
        let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let seen = requests.clone();
        let respond: Arc<Handler> = Arc::new(respond);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { break };
                let (seen, respond) = (seen.clone(), respond.clone());
                std::thread::spawn(move || {
                    let _ = handle(stream, &seen, &*respond);
                });
            }
        });
        MockServer { base, requests }
    }

    /// The requests received so far.
    pub fn seen(&self) -> Vec<Request> {
        self.requests.lock().map(|r| r.clone()).unwrap_or_default()
    }
}

fn handle(mut stream: std::net::TcpStream, seen: &Mutex<Vec<Request>>, respond: &Handler) -> std::io::Result<()> {
    // Accepted sockets inherit nonblocking from the listener on BSD/macOS.
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut reader = std::io::BufReader::new(stream.try_clone()?);
    loop {
        let mut first = String::new();
        if reader.read_line(&mut first)? == 0 {
            return Ok(());
        }
        let mut parts = first.split_whitespace();
        let method = parts.next().unwrap_or("").to_string();
        let path = parts.next().unwrap_or("").to_string();
        let (mut authorization, mut length, mut close) = (None, 0usize, false);
        loop {
            let mut h = String::new();
            if reader.read_line(&mut h)? == 0 || h.trim().is_empty() {
                break;
            }
            if let Some((k, v)) = h.split_once(':') {
                let v = v.trim();
                match k.trim().to_ascii_lowercase().as_str() {
                    "authorization" => authorization = Some(v.to_string()),
                    "content-length" => length = v.parse().unwrap_or(0),
                    "connection" => close = v.eq_ignore_ascii_case("close"),
                    _ => {}
                }
            }
        }
        let mut body = vec![0u8; length];
        reader.read_exact(&mut body)?;
        let req = Request { method, path, authorization, body: String::from_utf8_lossy(&body).into_owned() };
        let (status, text) = respond(&req);
        if let Ok(mut s) = seen.lock() {
            s.push(req);
        }
        let head = format!("HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n", text.len());
        stream.write_all(head.as_bytes())?;
        stream.write_all(text.as_bytes())?;
        stream.flush()?;
        if close {
            return Ok(());
        }
    }
}
