//! `.http` files (JetBrains HTTP Client / Visual Studio): parsing, environments, variable
//! resolution and sending.
//!
//! - [`parse`] splits the text into requests (`###` separators, `# @name`, `@var = value`).
//! - [`environments`] lists the environments of `http-client.env.json` /
//!   `http-client.private.env.json` (Rider) and Visual Studio's `$shared` / `.env.json.user`.
//! - [`resolve`] substitutes `{{var}}` and dynamic variables (`{{$guid}}`, `{{$timestamp}}`,
//!   `{{$randomInt}}`, `{{$datetime iso8601}}`…) and loads `< ./file` bodies.
//! - [`send`] performs the request (invalid certificates are accepted only for localhost).

use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const ENV_FILE: &str = "http-client.env.json";
pub const PRIVATE_ENV_FILE: &str = "http-client.private.env.json";
/// Visual Studio's per-user overrides.
pub const USER_ENV_FILE: &str = "http-client.env.json.user";

const METHODS: &[&str] = &["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS", "TRACE", "CONNECT"];

#[derive(Debug, Clone, Default, PartialEq)]
pub struct HttpFile {
    /// `@name = value` file variables, in file order (later definitions win).
    pub variables: Vec<(String, String)>,
    pub requests: Vec<HttpRequest>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HttpRequest {
    /// From `### name` or `# @name name` / `// @name name`.
    pub name: Option<String>,
    pub method: String,
    /// Unresolved URL, query continuation lines already appended.
    pub url: String,
    /// e.g. `HTTP/1.1` when written on the request line.
    pub http_version: Option<String>,
    /// Unresolved headers, in order.
    pub headers: Vec<(String, String)>,
    pub body: Option<RequestBody>,
    /// 0-based line of the request line (where the editor shows the ▶ marker).
    pub start_line: usize,
    /// 0-based lines of the whole block, excluding the `###` separators (end exclusive).
    pub range: Range<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RequestBody {
    /// Body text as written (variables still unresolved).
    Inline(String),
    /// `< path` (sent raw) or `<@ path` (variables substituted), relative to the `.http` dir.
    File { path: String, substitute: bool },
}

impl HttpFile {
    /// Index of the request whose block contains `line` (0-based).
    pub fn request_at_line(&self, line: usize) -> Option<usize> {
        self.requests.iter().position(|r| r.range.contains(&line))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedRequest {
    pub name: Option<String>,
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
}

#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: u16,
    /// Reason phrase (canonical for the status; HTTP/2 has none on the wire).
    pub reason: String,
    /// `HTTP/1.1`, `HTTP/2`…
    pub version: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// Body decoded as UTF-8 (lossy).
    pub text: String,
    pub elapsed: Duration,
    pub content_type: Option<String>,
}

/// Variables that could not be resolved; [`resolve`] returns it inside the `anyhow::Error`
/// (downcast with `err.downcast_ref::<UnresolvedVariables>()`).
#[derive(Debug, Clone, PartialEq)]
pub struct UnresolvedVariables {
    pub names: Vec<String>,
    pub environment: Option<String>,
}

impl std::fmt::Display for UnresolvedVariables {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let list = self.names.iter().map(|n| format!("{{{{{n}}}}}")).collect::<Vec<_>>().join(", ");
        match &self.environment {
            Some(env) => write!(f, "unresolved variables: {list} (environment '{env}')"),
            None => write!(f, "unresolved variables: {list} (no environment selected)"),
        }
    }
}

impl std::error::Error for UnresolvedVariables {}

// ---------------------------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------------------------

fn is_separator(line: &str) -> bool {
    line.trim_start().starts_with("###")
}

fn is_comment(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with('#') || t.starts_with("//")
}

fn comment_text(line: &str) -> &str {
    let t = line.trim_start();
    t.strip_prefix("//").or_else(|| t.strip_prefix('#')).unwrap_or(t).trim()
}

/// `@name = value` (also `@name=value`).
fn parse_variable(line: &str) -> Option<(String, String)> {
    let rest = line.trim().strip_prefix('@')?;
    let (name, value) = rest.split_once('=')?;
    let name = name.trim();
    if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.')) {
        return None;
    }
    Some((name.to_string(), value.trim().to_string()))
}

/// `@name foo` / `@name=foo` / `@name = foo` inside a comment.
fn parse_name_tag(comment: &str) -> Option<String> {
    let rest = comment.strip_prefix("@name")?;
    if !(rest.is_empty() || rest.starts_with([' ', '\t', '='])) {
        return None;
    }
    let name = rest.trim_start_matches([' ', '\t', '=']).trim();
    (!name.is_empty()).then(|| name.to_string())
}

pub fn parse(text: &str) -> HttpFile {
    let lines: Vec<&str> = text.lines().collect();
    let mut file = HttpFile::default();
    let mut block_start = 0;
    let mut block_name: Option<String> = None;
    for i in 0..=lines.len() {
        if i == lines.len() || is_separator(lines[i]) {
            parse_block(&lines, block_start..i, block_name.take(), &mut file);
            if i < lines.len() {
                let name = lines[i].trim_start().trim_start_matches('#').trim();
                block_name = (!name.is_empty()).then(|| name.to_string());
                block_start = i + 1;
            }
        }
    }
    file
}

fn parse_block(lines: &[&str], range: Range<usize>, mut name: Option<String>, file: &mut HttpFile) {
    let mut i = range.start;
    // Preamble: blank lines, comments (`# @name`), variables.
    let request_line = loop {
        if i >= range.end {
            return;
        }
        let line = lines[i];
        if line.trim().is_empty() {
        } else if is_comment(line) {
            if let Some(n) = parse_name_tag(comment_text(line)) {
                name = Some(n);
            }
        } else if let Some(var) = parse_variable(line) {
            file.variables.push(var);
        } else {
            break i;
        }
        i += 1;
    };

    let mut tokens: Vec<&str> = lines[request_line].split_whitespace().collect();
    let method = match tokens.first() {
        Some(t) if METHODS.contains(&t.to_ascii_uppercase().as_str()) => tokens.remove(0).to_ascii_uppercase(),
        _ => "GET".to_string(),
    };
    let http_version = match tokens.last() {
        Some(t) if t.starts_with("HTTP/") && tokens.len() > 1 => tokens.pop().map(str::to_string),
        _ => None,
    };
    let mut url = tokens.join(" ");
    i += 1;

    // Query continuation lines.
    while i < range.end {
        let t = lines[i].trim();
        if t.starts_with('?') || t.starts_with('&') {
            url.push_str(t);
            i += 1;
        } else {
            break;
        }
    }

    // Headers until the first blank line.
    let mut headers = Vec::new();
    while i < range.end {
        let line = lines[i];
        i += 1;
        if line.trim().is_empty() {
            break;
        }
        if is_comment(line) {
            continue;
        }
        if let Some((k, v)) = line.split_once(':') {
            headers.push((k.trim().to_string(), v.trim().to_string()));
        }
    }

    // Body until the end of the block; response handlers / redirections (`> {% %}`, `>> file`)
    // end it.
    let mut body_lines: Vec<&str> = Vec::new();
    while i < range.end {
        let t = lines[i].trim_start();
        if t.starts_with("> ") || t.starts_with(">>") || t.starts_with(">{%") {
            break;
        }
        body_lines.push(lines[i]);
        i += 1;
    }
    while body_lines.last().is_some_and(|l| l.trim().is_empty()) {
        body_lines.pop();
    }
    while body_lines.first().is_some_and(|l| l.trim().is_empty()) {
        body_lines.remove(0);
    }
    let body = if body_lines.is_empty() {
        None
    } else if body_lines.len() == 1 && body_lines[0].trim_start().starts_with('<') {
        let t = body_lines[0].trim_start();
        let (substitute, path) = match t.strip_prefix("<@") {
            Some(p) => (true, p),
            None => (false, &t[1..]),
        };
        Some(RequestBody::File { path: path.trim().to_string(), substitute })
    } else {
        let multipart = headers.iter().any(|(k, v)| k.eq_ignore_ascii_case("content-type") && v.to_ascii_lowercase().starts_with("multipart/"));
        Some(RequestBody::Inline(body_lines.join(if multipart { "\r\n" } else { "\n" })))
    };

    file.requests.push(HttpRequest { name, method, url, http_version, headers, body, start_line: request_line, range });
}

// ---------------------------------------------------------------------------------------------
// Environments
// ---------------------------------------------------------------------------------------------

/// Directories from the workspace root to the `.http` file that hold environment files.
fn env_dirs(http_path: &Path) -> Vec<PathBuf> {
    let Some(mut dir) = http_path.parent().map(Path::to_path_buf) else { return Vec::new() };
    if dir.as_os_str().is_empty() {
        dir = PathBuf::from(".");
    }
    let mut dirs = Vec::new();
    let mut cur = Some(dir.as_path());
    while let Some(d) = cur {
        if [ENV_FILE, PRIVATE_ENV_FILE, USER_ENV_FILE].iter().any(|f| d.join(f).is_file()) {
            dirs.push(d.to_path_buf());
        }
        if is_root(d) {
            break;
        }
        cur = d.parent();
    }
    dirs.reverse();
    dirs
}

fn is_root(dir: &Path) -> bool {
    if dir.join(".git").exists() {
        return true;
    }
    std::fs::read_dir(dir).is_ok_and(|rd| {
        rd.flatten().any(|e| {
            let p = e.path();
            matches!(p.extension().and_then(|x| x.to_str()), Some("sln" | "slnx"))
        })
    })
}

/// Reads bounded input, rejecting non-regular files before opening them.
pub fn read_bounded_file(path: &Path, max: usize) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let meta = std::fs::metadata(path)?;
    if !meta.is_file() || meta.len() > max as u64 { return Err(std::io::Error::other("input must be a bounded regular file")); }
    let mut bytes = Vec::new();
    std::fs::File::open(path)?.take(max as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > max { return Err(std::io::Error::other("input file exceeds its size limit")); }
    Ok(bytes)
}

fn read_env_json(path: &Path) -> Option<serde_json::Map<String, serde_json::Value>> {
    let text = String::from_utf8(read_bounded_file(path, 1024 * 1024).ok()?).ok()?;
    match serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()? {
        serde_json::Value::Object(m) => Some(m),
        _ => None,
    }
}

/// Env files in increasing priority: public, private (Rider), `.user` (Visual Studio).
/// Within each kind, a file nearer to the request overrides values from a parent.
fn env_files(http_path: &Path) -> Vec<serde_json::Map<String, serde_json::Value>> {
    let dirs = env_dirs(http_path);
    [ENV_FILE, PRIVATE_ENV_FILE, USER_ENV_FILE]
        .iter()
        .flat_map(|name| dirs.iter().filter_map(move |dir| read_env_json(&dir.join(name))))
        .collect()
}

/// Environment names available for `http_path` (`$shared` excluded), in file order.
pub fn environments(http_path: &Path) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for m in env_files(http_path) {
        for k in m.keys() {
            if !k.starts_with('$') && !names.contains(k) {
                names.push(k.clone());
            }
        }
    }
    names
}

fn flatten_into(prefix: &str, value: &serde_json::Value, out: &mut HashMap<String, String>) {
    match value {
        serde_json::Value::Object(m) => {
            for (k, v) in m {
                let key = if prefix.is_empty() { k.clone() } else { format!("{prefix}.{k}") };
                flatten_into(&key, v, out);
            }
        }
        serde_json::Value::String(s) => {
            out.insert(prefix.to_string(), s.clone());
        }
        serde_json::Value::Null => {}
        other => {
            out.insert(prefix.to_string(), other.to_string());
        }
    }
}

/// Variables of environment `env` for `http_path`: `$shared` first, then the environment, each
/// file overriding the previous one (public < private < `.user`).
pub fn environment_variables(http_path: &Path, env: Option<&str>) -> HashMap<String, String> {
    let mut vars = HashMap::new();
    let files = env_files(http_path);
    for m in &files {
        if let Some(shared) = m.get("$shared") {
            flatten_into("", shared, &mut vars);
        }
    }
    if let Some(env) = env {
        for m in &files {
            if let Some(v) = m.get(env) {
                flatten_into("", v, &mut vars);
            }
        }
    }
    vars
}

// ---------------------------------------------------------------------------------------------
// Variable resolution
// ---------------------------------------------------------------------------------------------

/// Resolves request `index` of `file` with environment `env`. File variables override
/// environment variables. Fails with [`UnresolvedVariables`] when any `{{var}}` is unknown.
pub fn resolve(file: &HttpFile, index: usize, env: Option<&str>, http_path: &Path) -> anyhow::Result<ResolvedRequest> {
    let req = file.requests.get(index).ok_or_else(|| anyhow::anyhow!("no request #{index} in the .http file"))?;
    let mut vars = environment_variables(http_path, env);
    for (k, v) in &file.variables {
        vars.insert(k.clone(), v.clone());
    }
    let mut r = Resolver { vars: &vars, missing: Vec::new(), remaining: 16 * 1024 * 1024, exhausted: false };

    let url = r.substitute(&req.url, 0);
    let headers: Vec<(String, String)> = req.headers.iter().map(|(k, v)| (r.substitute(k, 0), r.substitute(v, 0))).collect();
    let body = match &req.body {
        None => None,
        Some(RequestBody::Inline(text)) => Some(r.substitute(text, 0).into_bytes()),
        Some(RequestBody::File { path, substitute }) => {
            let rel = r.substitute(path, 0);
            let base = http_path.parent().unwrap_or(Path::new("."));
            let full = base.join(&rel);
            let bytes = read_bounded_file(&full, 16 * 1024 * 1024).map_err(|e| anyhow::anyhow!("cannot read request body file {}: {e}", full.display()))?;
            if *substitute {
                Some(r.substitute(&String::from_utf8_lossy(&bytes), 0).into_bytes())
            } else {
                Some(bytes)
            }
        }
    };

    if r.exhausted { anyhow::bail!("HTTP variable expansion exceeds 16 MiB"); }
    if !r.missing.is_empty() {
        return Err(UnresolvedVariables { names: r.missing, environment: env.map(str::to_string) }.into());
    }
    let url = url.trim().to_string();
    if url.is_empty() {
        anyhow::bail!("the request has no URL");
    }
    Ok(ResolvedRequest { name: req.name.clone(), method: req.method.clone(), url, headers, body })
}

struct Resolver<'a> {
    vars: &'a HashMap<String, String>,
    missing: Vec<String>,
    remaining: usize,
    exhausted: bool,
}

impl Resolver<'_> {
    fn substitute(&mut self, text: &str, depth: usize) -> String {
        if text.len() > self.remaining { self.exhausted = true; return String::new(); }
        self.remaining -= text.len();
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        while let Some(start) = rest.find("{{") {
            let Some(len) = rest[start + 2..].find("}}") else { break };
            out.push_str(&rest[..start]);
            let expr = rest[start + 2..start + 2 + len].trim();
            match self.lookup(expr, depth) {
                Some(v) => out.push_str(&v),
                None => {
                    if !self.missing.iter().any(|m| m == expr) {
                        self.missing.push(expr.to_string());
                    }
                    out.push_str(&rest[start..start + 4 + len]);
                }
            }
            rest = &rest[start + 4 + len..];
        }
        out.push_str(rest);
        out
    }

    fn lookup(&mut self, expr: &str, depth: usize) -> Option<String> {
        if expr.starts_with('$') {
            let value = dynamic_variable(expr)?;
            if value.len() > self.remaining { self.exhausted = true; return Some(String::new()); }
            self.remaining -= value.len();
            return Some(value);
        }
        let value = self.vars.get(expr)?;
        if depth >= 16 {
            if value.len() > self.remaining { self.exhausted = true; return Some(String::new()); }
            self.remaining -= value.len();
            return Some(value.clone());
        }
        Some(self.substitute(value, depth + 1))
    }
}

/// `$guid`, `$uuid`, `$random.uuid`, `$timestamp`, `$isoTimestamp`, `$randomInt [min max]`,
/// `$random.integer(min, max)`, `$datetime iso8601|rfc1123`, `$localDatetime …`,
/// `$processEnv NAME` / `$env.NAME`.
fn dynamic_variable(expr: &str) -> Option<String> {
    let mut parts = expr.split_whitespace();
    let name = parts.next()?;
    let args: Vec<&str> = parts.collect();
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    match name {
        "$guid" | "$uuid" | "$random.uuid" => Some(guid()),
        "$timestamp" => Some(now.as_secs().to_string()),
        "$isoTimestamp" => Some(iso8601(now.as_secs() as i64)),
        "$randomInt" => {
            let (min, max) = match args.as_slice() {
                [a, b] => (a.parse().ok()?, b.parse().ok()?),
                [] => (0i64, 1000i64),
                _ => return None,
            };
            Some(random_in(min, max).to_string())
        }
        "$datetime" | "$localDatetime" => match args.first().copied() {
            Some("rfc1123") => Some(rfc1123(now.as_secs() as i64)),
            Some("iso8601") | None => Some(iso8601(now.as_secs() as i64)),
            _ => None,
        },
        "$processEnv" => std::env::var(args.first()?.trim_start_matches('%')).ok(),
        _ => {
            if let Some(var) = name.strip_prefix("$env.") {
                return std::env::var(var).ok();
            }
            if let Some(inner) = name.strip_prefix("$random.integer(").and_then(|s| s.strip_suffix(')')) {
                let (a, b) = inner.split_once(',')?;
                return Some(random_in(a.trim().parse().ok()?, b.trim().parse().ok()?).to_string());
            }
            None
        }
    }
}

fn random_u64() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos());
    h.finish()
}

/// Random integer in `[min, max)` (`min` when the range is empty).
fn random_in(min: i64, max: i64) -> i64 {
    if max <= min {
        return min;
    }
    min + (random_u64() % (max - min) as u64) as i64
}

fn guid() -> String {
    let mut b = [0u8; 16];
    b[..8].copy_from_slice(&random_u64().to_le_bytes());
    b[8..].copy_from_slice(&random_u64().to_le_bytes());
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

/// (year, month, day) for days since 1970-01-01 (Howard Hinnant's algorithm).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

fn iso8601(secs: i64) -> String {
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    let s = secs.rem_euclid(86_400);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", s / 3600, s % 3600 / 60, s % 60)
}

fn rfc1123(secs: i64) -> String {
    const DAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let days = secs.div_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    let s = secs.rem_euclid(86_400);
    format!("{}, {d:02} {} {y:04} {:02}:{:02}:{:02} GMT", DAYS[days.rem_euclid(7) as usize], MONTHS[m as usize - 1], s / 3600, s % 3600 / 60, s % 60)
}

// ---------------------------------------------------------------------------------------------
// Sending
// ---------------------------------------------------------------------------------------------

fn is_local_host(host: &str) -> bool {
    let h = host.trim_start_matches('[').trim_end_matches(']');
    h.eq_ignore_ascii_case("localhost") || h.to_ascii_lowercase().ends_with(".localhost") || h == "::1" || h.starts_with("127.")
}

/// Sends `req`. Certificate errors are ignored only for localhost (ASP.NET dev certificates);
/// localhost also bypasses the system proxy. Redirects are followed (up to 10).
pub async fn send(req: &ResolvedRequest, timeout: Duration) -> anyhow::Result<HttpResponse> {
    let url = reqwest::Url::parse(&req.url).map_err(|e| anyhow::anyhow!("invalid URL '{}': {e}", req.url))?;
    let local = url.host_str().is_some_and(is_local_host);
    let mut builder = reqwest::Client::builder().timeout(timeout).redirect(reqwest::redirect::Policy::limited(10));
    if local {
        builder = builder.tls_danger_accept_invalid_certs(true).no_proxy();
    }
    let client = builder.build()?;
    let method = reqwest::Method::from_bytes(req.method.as_bytes()).map_err(|_| anyhow::anyhow!("invalid HTTP method '{}'", req.method))?;
    let mut rb = client.request(method, url);
    for (k, v) in &req.headers {
        rb = rb.header(k.as_str(), v.as_str());
    }
    if let Some(body) = &req.body {
        rb = rb.body(body.clone());
    }

    let started = Instant::now();
    let mut resp = rb.send().await.map_err(|e| anyhow::anyhow!("{}: {}", req.url, error_chain(&e)))?;
    let status = resp.status();
    let version = format!("{:?}", resp.version());
    let headers: Vec<(String, String)> =
        resp.headers().iter().map(|(k, v)| (k.as_str().to_string(), String::from_utf8_lossy(v.as_bytes()).into_owned())).collect();
    let content_type = resp.headers().get(reqwest::header::CONTENT_TYPE).map(|v| String::from_utf8_lossy(v.as_bytes()).into_owned());
    let mut body = Vec::new();
    while let Some(chunk) = resp.chunk().await? {
        if body.len().saturating_add(chunk.len()) > 8 * 1024 * 1024 { anyhow::bail!("HTTP response exceeds 8 MiB"); }
        body.extend_from_slice(&chunk);
    }
    let elapsed = started.elapsed();
    Ok(HttpResponse {
        status: status.as_u16(),
        reason: status.canonical_reason().unwrap_or("").to_string(),
        version,
        headers,
        text: String::from_utf8_lossy(&body).into_owned(),
        body,
        elapsed,
        content_type,
    })
}

fn error_chain(e: &dyn std::error::Error) -> String {
    let mut s = e.to_string();
    let mut src = e.source();
    while let Some(inner) = src {
        let t = inner.to_string();
        if !s.contains(&t) {
            s.push_str(": ");
            s.push_str(&t);
        }
        src = inner.source();
    }
    s
}

// ---------------------------------------------------------------------------------------------
// Pretty printing
// ---------------------------------------------------------------------------------------------

/// The body for display: JSON re-indented (keeping key order and number formatting), anything
/// else as text.
pub fn pretty_body(resp: &HttpResponse) -> String {
    let ct = resp.content_type.as_deref().unwrap_or("").to_ascii_lowercase();
    let t = resp.text.trim_start_matches('\u{feff}').trim();
    let looks_json = ct.contains("json") || t.starts_with('{') || t.starts_with('[');
    if looks_json && serde_json::from_str::<serde::de::IgnoredAny>(t).is_ok() {
        return indent_json(t);
    }
    resp.text.clone()
}

/// Re-indents valid JSON text with two spaces.
fn indent_json(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 2);
    let mut level = 0usize;
    let mut in_str = false;
    let mut escaped = false;
    let mut chars = text.chars().peekable();
    let newline = |out: &mut String, level: usize| {
        out.push('\n');
        out.push_str(&"  ".repeat(level));
    };
    while let Some(c) = chars.next() {
        if in_str {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        match c {
            '"' => {
                in_str = true;
                out.push(c);
            }
            '{' | '[' => {
                out.push(c);
                while chars.peek().is_some_and(|c| c.is_whitespace()) {
                    chars.next();
                }
                if matches!(chars.peek(), Some('}' | ']')) {
                    out.push(chars.next().unwrap_or(']'));
                } else {
                    level += 1;
                    newline(&mut out, level);
                }
            }
            '}' | ']' => {
                level = level.saturating_sub(1);
                newline(&mut out, level);
                out.push(c);
            }
            ',' => {
                out.push(c);
                newline(&mut out, level);
            }
            ':' => out.push_str(": "),
            c if c.is_whitespace() => {}
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
@host = https://localhost:5001
@id = 42

### Get item
GET {{host}}/items/{{id}} HTTP/1.1
Accept: application/json
# a comment header line

###
# @name create
POST {{host}}/items
Content-Type: application/json

{
  \"name\": \"{{name}}\",
  # not a comment inside the body
  \"id\": {{id}}
}

### no method
{{host}}/search
    ?q=hello
    &page=2

###
// only comments here

###
PUT {{host}}/upload
Content-Type: application/json

< ./payload.json
";

    #[test]
    fn parses_requests_variables_and_lines() {
        let f = parse(SAMPLE);
        assert_eq!(f.variables, vec![("host".into(), "https://localhost:5001".into()), ("id".into(), "42".into())]);
        assert_eq!(f.requests.len(), 4);

        let r = &f.requests[0];
        assert_eq!(r.name.as_deref(), Some("Get item"));
        assert_eq!((r.method.as_str(), r.url.as_str(), r.http_version.as_deref()), ("GET", "{{host}}/items/{{id}}", Some("HTTP/1.1")));
        assert_eq!(r.headers, vec![("Accept".into(), "application/json".into())]);
        assert_eq!(r.body, None);
        assert_eq!(r.start_line, 4);
        assert_eq!(r.range, 4..8);

        let r = &f.requests[1];
        assert_eq!(r.name.as_deref(), Some("create"));
        assert_eq!(r.method, "POST");
        assert_eq!(r.start_line, 10);
        let Some(RequestBody::Inline(body)) = &r.body else { panic!("inline body expected") };
        assert!(body.starts_with('{') && body.ends_with('}'));
        assert!(body.contains("# not a comment inside the body"));

        let r = &f.requests[2];
        assert_eq!(r.name.as_deref(), Some("no method"));
        assert_eq!((r.method.as_str(), r.url.as_str()), ("GET", "{{host}}/search?q=hello&page=2"));

        let r = &f.requests[3];
        assert_eq!(r.body, Some(RequestBody::File { path: "./payload.json".into(), substitute: false }));
        assert_eq!(f.request_at_line(r.start_line), Some(3));
        assert_eq!(f.request_at_line(0), None);
    }

    #[test]
    fn single_request_without_separator() {
        let f = parse("https://example.com/\n");
        assert_eq!(f.requests.len(), 1);
        assert_eq!((f.requests[0].method.as_str(), f.requests[0].url.as_str(), f.requests[0].start_line), ("GET", "https://example.com/", 0));
        assert!(parse("# just a comment\n\n").requests.is_empty());
    }

    #[test]
    fn deep_and_dynamic_variable_expansions_obey_the_budget() {
        let root = tempfile::tempdir().unwrap(); let path = root.path().join("bounded.http");
        let mut source = String::new();
        for index in 0..15 { source.push_str(&format!("@v{index} = {{{{v{}}}}}\n", index + 1)); }
        source.push_str(&format!("@v15 = {}\n@payload = {}\nGET http://localhost/{{{{v0}}}}\n", "{{payload}}".repeat(100), "x".repeat(256 * 1024)));
        assert!(source.len() < 1024 * 1024);
        assert!(resolve(&parse(&source), 0, None, &path).unwrap_err().to_string().contains("16 MiB"));
        // Small input references can expand into much larger dynamic values in a body file.
        std::fs::write(root.path().join("body.txt"), "{{$guid}}".repeat(500_000)).unwrap();
        let request = parse("POST http://localhost/\n\n<@ body.txt\n");
        assert!(resolve(&request, 0, None, &path).unwrap_err().to_string().contains("16 MiB"));
    }

    fn write(dir: &Path, name: &str, text: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, text).unwrap();
        p
    }

    #[test]
    fn environments_and_resolution() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join(".git")).unwrap();
        let sub = root.path().join("api");
        std::fs::create_dir(&sub).unwrap();
        write(root.path(), ENV_FILE, r#"{"$shared":{"name":"shared"},"dev":{"host":"http://dev","token":"public"},"prod":{"host":"http://prod","auth":{"user":"u"}}}"#);
        write(root.path(), PRIVATE_ENV_FILE, r#"{"dev":{"token":"secret"},"local":{"host":"http://localhost"}}"#);
        let http = write(&sub, "api.http", "");
        write(&sub, "payload.json", "{\"n\":\"{{name}}\"}");
        assert_eq!(environments(&http), vec!["dev", "prod", "local"]);

        let f = parse(SAMPLE);
        // No environment: `name` (only in $shared) resolves, host comes from the file.
        let r = resolve(&f, 1, None, &http).unwrap();
        assert_eq!(r.url, "https://localhost:5001/items");
        assert_eq!(String::from_utf8(r.body.unwrap()).unwrap(), "{\n  \"name\": \"shared\",\n  # not a comment inside the body\n  \"id\": 42\n}");

        let r = resolve(&f, 3, Some("dev"), &http).unwrap();
        assert_eq!(r.body.as_deref(), Some(&b"{\"n\":\"{{name}}\"}"[..]));

        let vars = environment_variables(&http, Some("dev"));
        assert_eq!(vars["token"], "secret");
        assert_eq!(vars["host"], "http://dev");
        assert_eq!(environment_variables(&http, Some("prod"))["auth.user"], "u");

        // Environment variables referenced through file variables.
        let f2 = parse("@base = {{host}}/v1\nGET {{base}}/x?t={{token}}\nAuthorization: Bearer {{token}}\n");
        let r = resolve(&f2, 0, Some("dev"), &http).unwrap();
        assert_eq!(r.url, "http://dev/v1/x?t=secret");
        assert_eq!(r.headers, vec![("Authorization".into(), "Bearer secret".into())]);

        let err = resolve(&f2, 0, Some("local"), &http).unwrap_err();
        let un = err.downcast_ref::<UnresolvedVariables>().unwrap();
        assert_eq!(un.names, vec!["token"]);
        assert!(err.to_string().contains("{{token}}") && err.to_string().contains("local"), "{err}");

        // `<@` substitutes variables in the file.
        let f3 = parse("POST http://x/\n\n<@ payload.json\n");
        assert_eq!(resolve(&f3, 0, None, &http).unwrap().body.as_deref(), Some(&b"{\"n\":\"shared\"}"[..]));
        assert!(resolve(&f3, 5, None, &http).is_err());
    }

    #[test]
    fn visual_studio_user_file() {
        let root = tempfile::tempdir().unwrap();
        write(root.path(), "App.sln", "");
        write(root.path(), ENV_FILE, r#"{"$shared":{"v":"1"},"dev":{"host":"h"}}"#);
        write(root.path(), USER_ENV_FILE, r#"{"dev":{"host":"mine"}}"#);
        let http = root.path().join("a.http");
        assert_eq!(environments(&http), vec!["dev"]);
        let vars = environment_variables(&http, Some("dev"));
        assert_eq!((vars["host"].as_str(), vars["v"].as_str()), ("mine", "1"));
    }

    #[test]
    fn dynamic_variables() {
        let f = parse("GET http://x/{{$guid}}/{{$timestamp}}/{{$randomInt}}/{{ $randomInt 5 6 }}/{{$datetime iso8601}}\n");
        let r = resolve(&f, 0, None, Path::new("/nonexistent/a.http")).unwrap();
        let parts: Vec<&str> = r.url.trim_start_matches("http://x/").split('/').collect();
        assert_eq!(parts[0].len(), 36);
        assert_eq!(parts[0].as_bytes()[14], b'4');
        assert!(parts[1].parse::<u64>().unwrap() > 1_600_000_000);
        assert!(parts[2].parse::<u32>().unwrap() < 1000);
        assert_eq!(parts[3], "5");
        assert!(parts[4].ends_with('Z') && parts[4].len() == 20, "{}", parts[4]);
        assert_ne!(guid(), guid());
        assert_eq!(iso8601(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(rfc1123(0), "Thu, 01 Jan 1970 00:00:00 GMT");
        let err = resolve(&parse("GET http://x/{{$nope}}\n"), 0, None, Path::new("a.http")).unwrap_err();
        assert!(err.to_string().contains("$nope"));
    }

    #[test]
    fn pretty_prints_json_keeping_order() {
        let resp = |ct: &str, text: &str| HttpResponse {
            status: 200,
            reason: "OK".into(),
            version: "HTTP/1.1".into(),
            headers: vec![],
            body: text.as_bytes().to_vec(),
            text: text.into(),
            elapsed: Duration::ZERO,
            content_type: Some(ct.into()),
        };
        assert_eq!(
            pretty_body(&resp("application/json", r#"{"z":1,"a":[1,2],"s":"x,{y}:\"","e":{},"n":1.50}"#)),
            "{\n  \"z\": 1,\n  \"a\": [\n    1,\n    2\n  ],\n  \"s\": \"x,{y}:\\\"\",\n  \"e\": {},\n  \"n\": 1.50\n}"
        );
        assert_eq!(pretty_body(&resp("text/plain", "hello")), "hello");
        assert_eq!(pretty_body(&resp("application/json", "{broken")), "{broken");
    }

    #[test]
    fn local_hosts() {
        assert!(is_local_host("localhost") && is_local_host("127.0.0.1") && is_local_host("[::1]") && is_local_host("api.localhost"));
        assert!(!is_local_host("example.com") && !is_local_host("localhost.example.com"));
    }

    #[tokio::test]
    async fn sends_to_local_server() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut buf = Vec::new();
            let mut chunk = [0u8; 1024];
            // Read headers, then Content-Length bytes of body.
            loop {
                let n = s.read(&mut chunk).unwrap();
                buf.extend_from_slice(&chunk[..n]);
                if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&buf[..end]).to_ascii_lowercase();
                    let len: usize = head.lines().find_map(|l| l.strip_prefix("content-length:")).map_or(0, |v| v.trim().parse().unwrap());
                    if buf.len() >= end + 4 + len || n == 0 {
                        break;
                    }
                }
            }
            let body = b"{\"ok\":true,\"items\":[1]}";
            write!(s, "HTTP/1.1 201 Created\r\nContent-Type: application/json\r\nX-Test: yes\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
            s.write_all(body).unwrap();
            String::from_utf8_lossy(&buf).into_owned()
        });

        let f = parse(&format!("@port = {port}\n### create\nPOST http://localhost:{{{{port}}}}/api?x=1\nContent-Type: application/json\n\n{{\"a\": {{{{$randomInt 7 8}}}}}}\n"));
        let req = resolve(&f, 0, None, Path::new("/nonexistent/test.http")).unwrap();
        let resp = send(&req, Duration::from_secs(10)).await.unwrap();
        let raw = server.join().unwrap();

        assert!(raw.starts_with("POST /api?x=1 HTTP/1.1\r\n"), "{raw}");
        assert!(raw.to_ascii_lowercase().contains("content-type: application/json"));
        assert!(raw.ends_with("{\"a\": 7}"), "{raw}");
        assert_eq!((resp.status, resp.reason.as_str(), resp.version.as_str()), (201, "Created", "HTTP/1.1"));
        assert_eq!(resp.content_type.as_deref(), Some("application/json"));
        assert!(resp.headers.iter().any(|(k, v)| k == "x-test" && v == "yes"));
        assert_eq!(resp.text, "{\"ok\":true,\"items\":[1]}");
        assert_eq!(pretty_body(&resp), "{\n  \"ok\": true,\n  \"items\": [\n    1\n  ]\n}");
        assert!(resp.elapsed > Duration::ZERO);

        assert!(send(&ResolvedRequest { name: None, method: "GET".into(), url: "not a url".into(), headers: vec![], body: None }, Duration::from_secs(1)).await.is_err());
    }
}
