//! Credentials for private NuGet feeds (Azure Artifacts, GitHub Packages…): the
//! `packageSourceCredentials` of the nuget.config hierarchy (with `%VAR%` expansion; encrypted
//! `Password`s through DPAPI on Windows), `VSS_NUGET_EXTERNAL_FEED_ENDPOINTS` and NuGet
//! credential provider plugins (the JSON-over-stdio plugin protocol, non-interactive first, then
//! device flow on request).
//!
//! Credentials stay in memory for the session ([`Session`]), are only sent to the feed's own
//! origin (scheme, host and port, also across redirects) and never appear in `Debug` output.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Context as _;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::nuget::NugetClient;

// ------------------------------------------------------------------ credentials

/// A credential for a feed. `Debug` never shows the secret.
#[derive(Clone, PartialEq, Eq)]
pub enum Credential {
    Basic { username: String, password: String },
    Bearer(String),
}

impl std::fmt::Debug for Credential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Credential::Basic { username, .. } => write!(f, "Basic({username}, ***)"),
            Credential::Bearer(_) => write!(f, "Bearer(***)"),
        }
    }
}

impl Credential {
    pub fn basic(username: impl Into<String>, password: impl Into<String>) -> Self {
        Credential::Basic { username: username.into(), password: password.into() }
    }

    fn secret(&self) -> &str {
        match self {
            Credential::Basic { password, .. } => password,
            Credential::Bearer(t) => t,
        }
    }

    fn apply(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match self {
            Credential::Basic { username, password } => req.basic_auth(username, Some(password)),
            Credential::Bearer(t) => req.bearer_auth(t),
        }
    }
}

/// Replaces the credential's secret (and any `user:password@` in URLs) with `***`.
pub fn redact(text: &str, credential: Option<&Credential>) -> String {
    let mut out = text.to_string();
    if let Some(c) = credential
        && c.secret().len() >= 4
    {
        out = out.replace(c.secret(), "***");
    }
    // `scheme://user:secret@host` → `scheme://***@host`.
    let mut res = String::with_capacity(out.len());
    let mut rest = out.as_str();
    while let Some(i) = rest.find("://") {
        let (head, tail) = rest.split_at(i + 3);
        res.push_str(head);
        let end = tail.find(|c: char| c == '/' || c.is_whitespace()).unwrap_or(tail.len());
        match tail[..end].rfind('@') {
            Some(at) => {
                res.push_str("***");
                rest = &tail[at..];
            }
            None => rest = tail,
        }
    }
    res.push_str(rest);
    res
}

/// The feed rejected the request (401/403). `with_credential`: a credential was sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthError {
    pub status: u16,
    pub with_credential: bool,
}

impl std::fmt::Display for AuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (self.status, self.with_credential) {
            (403, true) => write!(f, "access denied (403): the credentials have no access to this feed"),
            (_, true) => write!(f, "the credentials were rejected ({})", self.status),
            _ => write!(f, "the source needs credentials ({})", self.status),
        }
    }
}

impl std::error::Error for AuthError {}

/// Whether `e` is (or wraps) an [`AuthError`].
pub fn is_auth_error(e: &anyhow::Error) -> bool {
    e.chain().any(|c| c.downcast_ref::<AuthError>().is_some())
}

fn same_origin(a: &reqwest::Url, b: &reqwest::Url) -> bool {
    a.scheme() == b.scheme() && a.host_str().map(str::to_ascii_lowercase) == b.host_str().map(str::to_ascii_lowercase) && a.port_or_known_default() == b.port_or_known_default()
}

/// GETs `url` for the feed whose service index is `index_url`. The credential goes only to the
/// feed's origin; redirects are followed here (the client must not follow them) so a redirect to
/// another host never carries it. 401/403 become an [`AuthError`].
pub async fn get(http: &reqwest::Client, index_url: &str, credential: Option<&Credential>, url: &str) -> anyhow::Result<reqwest::Response> {
    let feed = reqwest::Url::parse(index_url)?;
    let mut url = reqwest::Url::parse(url)?;
    for _ in 0..10 {
        let cred = credential.filter(|_| same_origin(&feed, &url));
        let mut req = http.get(url.clone());
        if let Some(c) = cred {
            req = c.apply(req);
        }
        let resp = req.send().await.map_err(|e| anyhow::anyhow!(redact(&format!("{e:#}"), credential)))?;
        let status = resp.status();
        if status.is_redirection()
            && let Some(loc) = resp.headers().get(reqwest::header::LOCATION).and_then(|l| l.to_str().ok())
        {
            url = url.join(loc)?;
            continue;
        }
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Err(AuthError { status: status.as_u16(), with_credential: cred.is_some() }.into());
        }
        return Ok(resp);
    }
    anyhow::bail!("too many redirects")
}

// ------------------------------------------------------------------ nuget.config

/// A `packageSourceCredentials` entry as written in nuget.config.
#[derive(Clone, PartialEq, Eq)]
pub struct ConfigCredential {
    pub username: String,
    pub password: ConfigPassword,
}

#[derive(Clone, PartialEq, Eq)]
pub enum ConfigPassword {
    /// `ClearTextPassword` (may hold `%VAR%`).
    Clear(String),
    /// `Password`: base64 of a DPAPI blob, only readable on Windows by the same user.
    Encrypted(String),
}

impl std::fmt::Debug for ConfigPassword {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigPassword::Clear(_) => write!(f, "Clear(***)"),
            ConfigPassword::Encrypted(_) => write!(f, "Encrypted(***)"),
        }
    }
}

impl std::fmt::Debug for ConfigCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ConfigCredential({}, {:?})", self.username, self.password)
    }
}

/// Decodes XML local names the way NuGet writes source names (`My_x0020_Feed` → `My Feed`).
fn decode_name(name: &str) -> String {
    let mut out = String::new();
    let mut rest = name;
    while let Some(i) = rest.find("_x") {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        let hex = tail.get(2..6).filter(|h| h.chars().all(|c| c.is_ascii_hexdigit()));
        match (hex, tail.get(6..7)) {
            (Some(h), Some("_")) => {
                out.extend(u32::from_str_radix(h, 16).ok().and_then(char::from_u32));
                rest = &tail[7..];
            }
            _ => {
                out.push_str("_x");
                rest = &tail[2..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// The `packageSourceCredentials` of one nuget.config: (source name, credential).
pub fn parse_credentials(text: &str) -> Vec<(String, ConfigCredential)> {
    let Ok(doc) = roxmltree::Document::parse(text) else { return Vec::new() };
    let mut out = Vec::new();
    for section in doc.root_element().children().filter(|n| n.is_element() && n.tag_name().name() == "packageSourceCredentials") {
        for src in section.children().filter(|n| n.is_element()) {
            let get = |key: &str| src.children().filter(|n| n.is_element()).find(|n| n.attribute("key").is_some_and(|k| k.eq_ignore_ascii_case(key))).and_then(|n| n.attribute("value")).map(str::to_string);
            let Some(username) = get("Username") else { continue };
            let password = match (get("ClearTextPassword"), get("Password")) {
                (Some(p), _) => ConfigPassword::Clear(p),
                (None, Some(p)) => ConfigPassword::Encrypted(p),
                _ => continue,
            };
            out.push((decode_name(src.tag_name().name()), ConfigCredential { username, password }));
        }
    }
    out
}

/// Expands `%NAME%` like .NET's `Environment.ExpandEnvironmentVariables`: unknown names stay.
pub fn expand_env(text: &str, var: &dyn Fn(&str) -> Option<String>) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(i) = rest.find('%') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        match after.find('%') {
            Some(j) => match Some(&after[..j]).filter(|n| !n.is_empty()).and_then(var) {
                Some(v) => {
                    out.push_str(&v);
                    rest = &after[j + 1..];
                }
                None => {
                    // Keep `%NAME`; the closing `%` may open the next one.
                    out.push('%');
                    out.push_str(&after[..j]);
                    rest = &after[j..];
                }
            },
            None => {
                out.push_str(&rest[i..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

fn env_var(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

/// A usable credential from a config entry, or why there is none.
pub fn resolve_config_credential(c: &ConfigCredential, var: &dyn Fn(&str) -> Option<String>) -> Result<Credential, String> {
    let username = expand_env(&c.username, var);
    let password = match &c.password {
        ConfigPassword::Clear(p) => expand_env(p, var),
        ConfigPassword::Encrypted(p) => dpapi::decrypt(p)?,
    };
    Ok(Credential::Basic { username, password })
}

/// The user's NuGet.Config files (`%AppData%\NuGet` on Windows, `~/.nuget/NuGet` elsewhere).
fn user_configs(home: Option<PathBuf>, appdata: Option<PathBuf>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(a) = appdata {
        out.push(a.join("NuGet").join("NuGet.Config"));
    }
    if let Some(h) = home {
        out.push(h.join(".nuget").join("NuGet").join("NuGet.Config"));
    }
    out
}

/// The nuget.config files that apply in `dir`, closest first, then the user's.
pub fn config_files(dir: &Path) -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from);
    let appdata = if cfg!(windows) { std::env::var_os("APPDATA").map(PathBuf::from) } else { None };
    config_files_with(dir, home, appdata)
}

fn config_files_with(dir: &Path, home: Option<PathBuf>, appdata: Option<PathBuf>) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = Vec::new();
    let mut d = Some(dir);
    while let Some(cur) = d {
        if let Some(p) = ["nuget.config", "NuGet.Config", "NuGet.config"].iter().map(|n| cur.join(n)).find(|p| p.is_file()) {
            files.push(p);
        }
        d = cur.parent();
    }
    for p in user_configs(home, appdata) {
        if p.is_file() && !files.iter().any(|f| same_file(f, &p)) {
            files.push(p);
        }
    }
    files
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (dunce::canonicalize(a), dunce::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

/// The credential nuget.config gives `source` (closest file wins), or why it can't be used.
pub fn config_credential(dir: &Path, source: &str) -> Option<Result<Credential, String>> {
    config_credential_in(&config_files(dir), source, &env_var)
}

fn config_credential_in(files: &[PathBuf], source: &str, var: &dyn Fn(&str) -> Option<String>) -> Option<Result<Credential, String>> {
    for f in files {
        let Ok(text) = std::fs::read_to_string(f) else { continue };
        if let Some((_, c)) = parse_credentials(&text).into_iter().find(|(name, _)| name.eq_ignore_ascii_case(source)) {
            return Some(resolve_config_credential(&c, var));
        }
    }
    None
}

/// The credential `VSS_NUGET_EXTERNAL_FEED_ENDPOINTS` (JSON) gives the feed at `url`.
pub fn external_endpoint(json: &str, url: &str) -> Option<Credential> {
    let v: Value = serde_json::from_str(json).ok()?;
    let norm = |s: &str| s.trim().trim_end_matches('/').to_ascii_lowercase();
    v.get("endpointCredentials")?.as_array()?.iter().find(|e| e.get("endpoint").and_then(Value::as_str).is_some_and(|e| norm(e) == norm(url))).and_then(|e| {
        let password = e.get("password").and_then(Value::as_str)?.to_string();
        let username = e.get("username").and_then(Value::as_str).filter(|u| !u.is_empty()).unwrap_or("VssSessionToken").to_string();
        Some(Credential::Basic { username, password })
    })
}

// ------------------------------------------------------------------ DPAPI

#[cfg(windows)]
mod dpapi {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptUnprotectData};

    /// Decrypts a nuget.config `Password` (base64 DPAPI blob, entropy "NuGet", current user).
    pub fn decrypt(b64: &str) -> Result<String, String> {
        let mut data = super::base64_decode(b64).ok_or("the encrypted password is not valid base64")?;
        let mut entropy = b"NuGet".to_vec();
        let input = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_mut_ptr() };
        let ent = CRYPT_INTEGER_BLOB { cbData: entropy.len() as u32, pbData: entropy.as_mut_ptr() };
        let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
        // SAFETY: the blobs point at live buffers; the output is freed with LocalFree.
        let ok = unsafe { CryptUnprotectData(&input, std::ptr::null_mut(), &ent, std::ptr::null(), std::ptr::null(), CRYPTPROTECT_UI_FORBIDDEN, &mut out) };
        if ok == 0 || out.pbData.is_null() {
            return Err("Windows could not decrypt the password (it was encrypted by another user or machine)".into());
        }
        // SAFETY: DPAPI returned `cbData` bytes at `pbData`.
        let bytes = unsafe { std::slice::from_raw_parts(out.pbData, out.cbData as usize) }.to_vec();
        unsafe { LocalFree(out.pbData as _) };
        String::from_utf8(bytes).map_err(|_| "the decrypted password is not UTF-8".to_string())
    }
}

#[cfg(not(windows))]
mod dpapi {
    pub fn decrypt(_b64: &str) -> Result<String, String> {
        Err("the password is encrypted with Windows DPAPI, which only Windows can read; use ClearTextPassword (e.g. %NUGET_TOKEN%) or a credential provider".into())
    }
}

#[cfg_attr(not(any(windows, test)), allow(dead_code))]
fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let val = |c: u8| match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    };
    let clean: Vec<u8> = s.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    let body = clean.strip_suffix(b"==").or_else(|| clean.strip_suffix(b"=")).unwrap_or(&clean);
    let mut out = Vec::with_capacity(body.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0);
    for &b in body {
        acc = (acc << 6) | val(b)? as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

// ------------------------------------------------------------------ credential provider plugins

/// A NuGet plugin (credential provider) and how to start it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plugin {
    pub name: String,
    pub program: PathBuf,
    pub args: Vec<String>,
}

impl Plugin {
    /// A plugin file: `.dll` runs through `dotnet`, anything else directly; both get `-Plugin`.
    pub fn from_path(path: &Path, dotnet: Option<&Path>) -> Option<Plugin> {
        let name = path.file_stem()?.to_string_lossy().into_owned();
        let is_dll = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("dll"));
        Some(if is_dll {
            Plugin { name, program: dotnet?.to_path_buf(), args: vec![path.to_string_lossy().into_owned(), "-Plugin".into()] }
        } else {
            Plugin { name, program: path.to_path_buf(), args: vec!["-Plugin".into()] }
        })
    }
}

/// The installed credential providers, in the order NuGet asks them.
pub fn discover_plugins(dotnet: Option<&Path>) -> Vec<Plugin> {
    let home = std::env::var_os("USERPROFILE").filter(|_| cfg!(windows)).or_else(|| std::env::var_os("HOME")).map(PathBuf::from);
    discover_plugins_with(&env_var, home, std::env::var_os("PATH"), dotnet)
}

fn discover_plugins_with(var: &dyn Fn(&str) -> Option<String>, home: Option<PathBuf>, path: Option<std::ffi::OsString>, dotnet: Option<&Path>) -> Vec<Plugin> {
    // Explicit paths replace the conventional folders.
    let explicit = var("NUGET_NETCORE_PLUGIN_PATHS").or_else(|| var("NUGET_PLUGIN_PATHS")).filter(|v| !v.trim().is_empty());
    let mut files: Vec<PathBuf> = Vec::new();
    if let Some(list) = explicit {
        let seps: &[char] = if cfg!(windows) { &[';'] } else { &[';', ':'] };
        files.extend(list.split(seps).map(str::trim).filter(|s| !s.is_empty()).map(PathBuf::from).filter(|p| p.is_file()));
        if cfg!(windows)
            && let Some(fx) = var("NUGET_NETFX_PLUGIN_PATHS")
        {
            files.extend(fx.split(';').map(str::trim).filter(|s| !s.is_empty()).map(PathBuf::from).filter(|p| p.is_file()));
        }
    } else if let Some(home) = home {
        // `~/.nuget/plugins/netcore/<name>/<name>.dll` (and netfx `.exe` on Windows).
        let mut kinds = vec![("netcore", "dll")];
        if cfg!(windows) {
            kinds.push(("netfx", "exe"));
        }
        for (kind, ext) in kinds {
            let Ok(rd) = std::fs::read_dir(home.join(".nuget").join("plugins").join(kind)) else { continue };
            let mut dirs: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
            dirs.sort();
            for d in dirs {
                let Some(name) = d.file_name().map(|n| n.to_string_lossy().into_owned()) else { continue };
                let f = d.join(format!("{name}.{ext}"));
                if f.is_file() {
                    files.push(f);
                }
            }
        }
    }
    // `nuget-plugin-*` executables on PATH (NuGet 6.13+).
    if let Some(path) = path {
        for dir in std::env::split_paths(&path) {
            let Ok(rd) = std::fs::read_dir(&dir) else { continue };
            let mut found: Vec<PathBuf> = rd
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_file() && p.file_name().is_some_and(|n| n.to_string_lossy().to_ascii_lowercase().starts_with("nuget-plugin-")))
                .filter(|p| !cfg!(windows) || p.extension().is_some_and(|e| e.eq_ignore_ascii_case("exe")))
                .collect();
            found.sort();
            for f in found {
                if !files.iter().any(|x| x.file_name() == f.file_name()) {
                    files.push(f);
                }
            }
        }
    }
    files.iter().filter_map(|f| Plugin::from_path(f, dotnet)).collect()
}

/// What a plugin answered for a feed.
#[derive(Debug, PartialEq, Eq)]
pub enum PluginAnswer {
    Credential(Credential),
    /// No credential; the plugin's message when it gave one.
    NotFound(Option<String>),
}

const PROTOCOL: &str = "2.0.0";

/// A running plugin speaking NuGet's plugin protocol (one JSON message per line).
struct PluginProcess<'a> {
    child: tokio::process::Child,
    stdin: tokio::process::ChildStdin,
    lines: tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    next_id: u64,
    log: &'a (dyn Fn(String) + Send + Sync),
}

impl<'a> PluginProcess<'a> {
    fn start(plugin: &Plugin, log: &'a (dyn Fn(String) + Send + Sync)) -> anyhow::Result<Self> {
        let mut cmd = tokio::process::Command::new(&plugin.program);
        cmd.args(&plugin.args).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).kill_on_drop(true);
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        let mut child = cmd.spawn().with_context(|| format!("could not start {}", plugin.name))?;
        let stdin = child.stdin.take().context("no stdin")?;
        let stdout = child.stdout.take().context("no stdout")?;
        Ok(PluginProcess { child, stdin, lines: BufReader::new(stdout).lines(), next_id: 0, log })
    }

    async fn send(&mut self, msg: &Value) -> anyhow::Result<()> {
        let mut line = serde_json::to_string(msg)?;
        line.push('\n');
        self.stdin.write_all(line.as_bytes()).await?;
        self.stdin.flush().await?;
        Ok(())
    }

    /// Sends a request and waits for its response, answering the plugin's own requests
    /// meanwhile. Progress messages restart the `timeout`.
    async fn request(&mut self, method: &str, payload: Value, timeout: Duration) -> anyhow::Result<Value> {
        self.next_id += 1;
        let id = format!("dotcraft-{}", self.next_id);
        self.send(&json!({"RequestId": id, "Type": "Request", "Method": method, "Payload": payload})).await?;
        loop {
            let line = match tokio::time::timeout(timeout, self.lines.next_line()).await {
                Err(_) => anyhow::bail!("the credential provider did not answer {method} in time"),
                Ok(r) => r?.ok_or_else(|| anyhow::anyhow!("the credential provider exited during {method}"))?,
            };
            // Anything that is not a message (stray output) is ignored.
            let Ok(msg) = serde_json::from_str::<Value>(line.trim()) else { continue };
            let s = |k: &str| msg.get(k).and_then(Value::as_str).unwrap_or("").to_string();
            let (rid, kind, m) = (s("RequestId"), s("Type"), s("Method"));
            match kind.as_str() {
                "Response" if rid == id => return Ok(msg.get("Payload").cloned().unwrap_or(Value::Null)),
                "Fault" if rid == id => {
                    let why = msg.get("Payload").and_then(|p| p.get("Message")).and_then(Value::as_str).unwrap_or("fault").to_string();
                    anyhow::bail!("the credential provider failed: {why}")
                }
                "Request" => {
                    let payload = msg.get("Payload").cloned().unwrap_or(Value::Null);
                    let answer = match m.as_str() {
                        "Handshake" => json!({"ResponseCode": "Success", "ProtocolVersion": PROTOCOL}),
                        "Log" => {
                            if let Some(text) = payload.get("Message").and_then(Value::as_str) {
                                (self.log)(text.to_string());
                            }
                            json!({"ResponseCode": "Success"})
                        }
                        // Proxy or other credentials from the host: none to give.
                        _ => json!({"ResponseCode": "NotFound"}),
                    };
                    self.send(&json!({"RequestId": rid, "Type": "Response", "Method": m, "Payload": answer})).await?;
                }
                // Progress (and anything else) for a long request: keep waiting.
                _ => {}
            }
        }
    }

    async fn close(mut self) {
        let _ = tokio::time::timeout(Duration::from_secs(2), self.request("Close", Value::Null, Duration::from_secs(2))).await;
        let _ = self.child.start_kill();
        let _ = tokio::time::timeout(Duration::from_secs(2), self.child.wait()).await;
    }
}

fn success(payload: &Value) -> bool {
    payload.get("ResponseCode").and_then(Value::as_str) == Some("Success")
}

/// Asks one plugin for credentials for `uri` (the service index URL). Non-interactive asks for
/// cached or silent tokens only; interactive lets it run a device flow, whose instructions
/// arrive through `log`. `timeout` bounds each wait for an answer.
pub async fn query_plugin(plugin: &Plugin, uri: &str, interactive: bool, is_retry: bool, timeout: Duration, log: &(dyn Fn(String) + Send + Sync)) -> anyhow::Result<PluginAnswer> {
    let mut p = PluginProcess::start(plugin, log)?;
    let r = query(&mut p, uri, interactive, is_retry, timeout).await;
    p.close().await;
    r
}

async fn query(p: &mut PluginProcess<'_>, uri: &str, interactive: bool, is_retry: bool, timeout: Duration) -> anyhow::Result<PluginAnswer> {
    let short = Duration::from_secs(20).min(timeout);
    let hs = p.request("Handshake", json!({"ProtocolVersion": PROTOCOL, "MinimumProtocolVersion": "1.0.0"}), short).await?;
    anyhow::ensure!(success(&hs), "the credential provider refused the handshake");
    let _ = p.request("MonitorNuGetProcessExit", json!({"ProcessId": std::process::id()}), short).await;
    let secs = timeout.as_secs();
    let init = p.request("Initialize", json!({"ClientVersion": "6.12.0", "Culture": "en-US", "RequestTimeout": format!("{:02}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60)}), short).await?;
    anyhow::ensure!(success(&init), "the credential provider could not initialize");
    let claims = p.request("GetOperationClaims", json!({"PackageSourceRepository": null, "ServiceIndex": null}), short).await?;
    if let Some(list) = claims.get("Claims").and_then(Value::as_array)
        && !list.iter().any(|c| c.as_str() == Some("Authentication"))
    {
        return Ok(PluginAnswer::NotFound(None));
    }
    let _ = p.request("SetLogLevel", json!({"LogLevel": "Minimal"}), short).await;
    let r = p.request("GetAuthenticationCredentials", json!({"Uri": uri, "IsRetry": is_retry, "IsNonInteractive": !interactive, "CanShowDialog": false}), timeout).await?;
    let message = r.get("Message").and_then(Value::as_str).filter(|m| !m.is_empty()).map(str::to_string);
    if !success(&r) {
        return Ok(PluginAnswer::NotFound(message));
    }
    let username = r.get("Username").and_then(Value::as_str).unwrap_or("").to_string();
    let Some(password) = r.get("Password").and_then(Value::as_str).filter(|p| !p.is_empty()).map(str::to_string) else {
        return Ok(PluginAnswer::NotFound(message));
    };
    let types: Vec<String> = r.get("AuthenticationTypes").and_then(Value::as_array).map(|a| a.iter().filter_map(|t| t.as_str().map(str::to_ascii_lowercase)).collect()).unwrap_or_default();
    let bearer = !types.is_empty() && types.iter().all(|t| t == "bearer");
    Ok(PluginAnswer::Credential(if bearer { Credential::Bearer(password) } else { Credential::Basic { username: if username.is_empty() { "VssSessionToken".into() } else { username }, password } }))
}

// ------------------------------------------------------------------ connecting to a feed

/// Whether DotCraft can browse a source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourceStatus {
    Checking,
    Ok,
    /// Needs a credential that a provider can get interactively (Sign in).
    NeedsSignIn(String),
    Failed(String),
}

/// Credentials obtained this session, by service index URL (memory only).
#[derive(Clone, Default)]
pub struct Session {
    creds: Arc<Mutex<HashMap<String, Credential>>>,
}

impl Session {
    fn get(&self, url: &str) -> Option<Credential> {
        self.creds.lock().ok()?.get(url).cloned()
    }

    fn put(&self, url: &str, c: Credential) {
        if let Ok(mut m) = self.creds.lock() {
            m.insert(url.to_string(), c);
        }
    }

    pub fn forget(&self, url: &str) {
        if let Ok(mut m) = self.creds.lock() {
            m.remove(url);
        }
    }
}

/// A source to connect to and where its credentials may come from.
#[derive(Clone, Debug)]
pub struct Feed {
    pub name: String,
    pub url: String,
    /// From nuget.config (or why it can't be used).
    pub config: Option<Result<Credential, String>>,
    /// From `VSS_NUGET_EXTERNAL_FEED_ENDPOINTS`.
    pub endpoint: Option<Credential>,
}

impl Feed {
    /// The feed with the credentials of the nuget.config files that apply in `dir` and the environment.
    pub fn new(name: &str, url: &str, dir: &Path) -> Feed {
        let endpoint = env_var("VSS_NUGET_EXTERNAL_FEED_ENDPOINTS").and_then(|j| external_endpoint(&j, url));
        Feed { name: name.to_string(), url: url.to_string(), config: config_credential(dir, name), endpoint }
    }
}

/// The result of connecting: the status and, when OK, a client with the service index loaded.
pub struct Connection {
    pub status: SourceStatus,
    pub client: Option<NugetClient>,
}

enum Probe {
    Ok(NugetClient),
    Unauthorized(String),
    Failed(String),
}

async fn probe(url: &str, cred: Option<&Credential>) -> Probe {
    let client = match NugetClient::with_credentials(url, cred.cloned()) {
        Ok(c) => c,
        Err(e) => return Probe::Failed(format!("{e:#}")),
    };
    match client.service_index().await {
        Ok(_) => Probe::Ok(client),
        Err(e) if is_auth_error(&e) => Probe::Unauthorized(redact(&format!("{e:#}"), cred)),
        Err(e) => Probe::Failed(redact(&format!("{e:#}"), cred)),
    }
}

/// Timeout for a non-interactive provider request.
pub const SILENT_TIMEOUT: Duration = Duration::from_secs(60);
/// Timeout for an interactive sign-in (device flow).
pub const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(15 * 60);

/// Connects to `feed` without user interaction: the session's credential, nuget.config's, the
/// environment's, none, then the providers silently.
pub async fn connect(feed: &Feed, session: &Session, plugins: &[Plugin]) -> Connection {
    let mut tried: Vec<Credential> = Vec::new();
    let mut problems: Vec<String> = Vec::new();
    let config = match &feed.config {
        Some(Ok(c)) => Some(c.clone()),
        Some(Err(why)) => {
            problems.push(format!("nuget.config: {why}"));
            None
        }
        None => None,
    };
    let candidates: Vec<Credential> = [session.get(&feed.url), config, feed.endpoint.clone()].into_iter().flatten().collect();
    let mut last = String::new();
    for c in candidates {
        if tried.contains(&c) {
            continue;
        }
        match probe(&feed.url, Some(&c)).await {
            Probe::Ok(client) => {
                session.put(&feed.url, c);
                return Connection { status: SourceStatus::Ok, client: Some(client) };
            }
            Probe::Unauthorized(why) => last = why,
            Probe::Failed(why) => return Connection { status: SourceStatus::Failed(why), client: None },
        }
        tried.push(c);
    }
    if tried.is_empty() {
        match probe(&feed.url, None).await {
            Probe::Ok(client) => return Connection { status: SourceStatus::Ok, client: Some(client) },
            Probe::Unauthorized(why) => last = why,
            Probe::Failed(why) => return Connection { status: SourceStatus::Failed(why), client: None },
        }
    }
    session.forget(&feed.url);
    let quiet = |_: String| {};
    for plugin in plugins {
        match query_plugin(plugin, &feed.url, false, !tried.is_empty(), SILENT_TIMEOUT, &quiet).await {
            Ok(PluginAnswer::Credential(c)) if !tried.contains(&c) => match probe(&feed.url, Some(&c)).await {
                Probe::Ok(client) => {
                    session.put(&feed.url, c);
                    return Connection { status: SourceStatus::Ok, client: Some(client) };
                }
                Probe::Unauthorized(why) => last = why,
                Probe::Failed(why) => return Connection { status: SourceStatus::Failed(why), client: None },
            },
            Ok(_) => {}
            Err(e) => problems.push(format!("{}: {e:#}", plugin.name)),
        }
    }
    problems.insert(0, last);
    let why = problems.into_iter().filter(|p| !p.is_empty()).collect::<Vec<_>>().join("; ");
    let status = if plugins.is_empty() {
        SourceStatus::Failed(format!("{why}. Add packageSourceCredentials to nuget.config or install a credential provider"))
    } else {
        SourceStatus::NeedsSignIn(why)
    };
    Connection { status, client: None }
}

/// Signs in interactively through the providers (device flow messages arrive through `log`).
pub async fn sign_in(feed: &Feed, session: &Session, plugins: &[Plugin], log: &(dyn Fn(String) + Send + Sync)) -> Connection {
    if plugins.is_empty() {
        return Connection { status: SourceStatus::Failed("no NuGet credential provider is installed".into()), client: None };
    }
    session.forget(&feed.url);
    let mut problems: Vec<String> = Vec::new();
    for plugin in plugins {
        match query_plugin(plugin, &feed.url, true, true, SIGN_IN_TIMEOUT, log).await {
            Ok(PluginAnswer::Credential(c)) => match probe(&feed.url, Some(&c)).await {
                Probe::Ok(client) => {
                    session.put(&feed.url, c);
                    return Connection { status: SourceStatus::Ok, client: Some(client) };
                }
                Probe::Unauthorized(why) => problems.push(why),
                Probe::Failed(why) => return Connection { status: SourceStatus::Failed(why), client: None },
            },
            Ok(PluginAnswer::NotFound(Some(m))) => problems.push(format!("{}: {m}", plugin.name)),
            Ok(PluginAnswer::NotFound(None)) => {}
            Err(e) => problems.push(format!("{}: {e:#}", plugin.name)),
        }
    }
    let why = if problems.is_empty() { "no credential provider handles this feed".to_string() } else { problems.join("; ") };
    Connection { status: SourceStatus::NeedsSignIn(why), client: None }
}

/// The URL and user code in a device flow message ("…open the page https://… and enter the code XYZ…").
pub fn device_code(message: &str) -> (Option<String>, Option<String>) {
    let words: Vec<&str> = message.split_whitespace().collect();
    let trim = |w: &str| w.trim_matches(|c: char| matches!(c, '.' | ',' | ';' | ')' | '(' | '"' | '\'')).to_string();
    let url = words.iter().find(|w| w.contains("https://") || w.contains("http://")).map(|w| trim(&w[w.find("http").unwrap_or(0)..]));
    let code = words.windows(2).find(|p| p[0].eq_ignore_ascii_case("code")).map(|p| trim(p[1])).filter(|c| c.len() >= 4 && c.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '-'));
    (url, code)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, Read, Write};

    fn vars(list: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |n: &str| list.iter().find(|(k, _)| k.eq_ignore_ascii_case(n)).map(|(_, v)| v.to_string())
    }

    #[test]
    fn expands_environment_variables_like_dotnet() {
        let v = vars(&[("TOKEN", "s3cr3t"), ("USER", "me")]);
        assert_eq!(expand_env("%TOKEN%", &v), "s3cr3t");
        assert_eq!(expand_env("a-%USER%-%TOKEN%-b", &v), "a-me-s3cr3t-b");
        assert_eq!(expand_env("%MISSING%", &v), "%MISSING%");
        assert_eq!(expand_env("50%%TOKEN%", &v), "50%s3cr3t");
        assert_eq!(expand_env("%MISSING%TOKEN%", &v), "%MISSINGs3cr3t");
        assert_eq!(expand_env("100%", &v), "100%");
        assert_eq!(expand_env("plain", &v), "plain");
    }

    #[test]
    fn parses_credentials_clear_encrypted_and_encoded_names() {
        let creds = parse_credentials(
            r#"<?xml version="1.0" encoding="utf-8"?>
<configuration>
  <packageSourceCredentials>
    <My_x0020_Feed><add key="Username" value="me" /><add key="ClearTextPassword" value="%FEED_PAT%" /></My_x0020_Feed>
    <Windows><add key="username" value="w" /><add key="Password" value="AQAAANCMnd8=" /></Windows>
    <NoPassword><add key="Username" value="x" /></NoPassword>
  </packageSourceCredentials>
</configuration>"#,
        );
        assert_eq!(creds.len(), 2);
        assert_eq!(creds[0].0, "My Feed");
        assert_eq!(creds[0].1.password, ConfigPassword::Clear("%FEED_PAT%".into()));
        assert_eq!(creds[1].0, "Windows");
        assert_eq!(creds[1].1.password, ConfigPassword::Encrypted("AQAAANCMnd8=".into()));
        let c = resolve_config_credential(&creds[0].1, &vars(&[("FEED_PAT", "pat123")])).unwrap();
        assert_eq!(c, Credential::basic("me", "pat123"));
        if !cfg!(windows) {
            assert!(resolve_config_credential(&creds[1].1, &vars(&[])).unwrap_err().contains("DPAPI"));
        }
        assert_eq!(decode_name("A_x002F_B_x"), "A/B_x");
    }

    #[test]
    fn debug_and_errors_never_show_secrets() {
        let c = Credential::basic("me", "hunter22");
        assert_eq!(format!("{c:?}"), "Basic(me, ***)");
        assert_eq!(format!("{:?}", Credential::Bearer("tok3n".into())), "Bearer(***)");
        let cc = ConfigCredential { username: "me".into(), password: ConfigPassword::Clear("hunter22".into()) };
        assert!(!format!("{cc:?}").contains("hunter22"));
        assert_eq!(redact("bad hunter22 at https://me:hunter22@feed.example/v3", Some(&c)), "bad *** at https://***@feed.example/v3");
        assert_eq!(redact("GET https://feed.example/a@b failed", None), "GET https://feed.example/a@b failed");
    }

    #[test]
    fn closest_config_wins_and_user_config_is_last() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let sln = root.join("repo/src");
        std::fs::create_dir_all(&sln).unwrap();
        let home = root.join("home");
        std::fs::create_dir_all(home.join(".nuget/NuGet")).unwrap();
        let entry = |user: &str| format!(r#"<configuration><packageSourceCredentials><Feed><add key="Username" value="{user}" /><add key="ClearTextPassword" value="p" /></Feed></packageSourceCredentials></configuration>"#);
        std::fs::write(root.join("repo/nuget.config"), entry("repo")).unwrap();
        std::fs::write(home.join(".nuget/NuGet/NuGet.Config"), entry("user").replace("Feed>", "Other>")).unwrap();
        let files = config_files_with(&sln, Some(home.clone()), None);
        assert_eq!(files, vec![root.join("repo/nuget.config"), home.join(".nuget/NuGet/NuGet.Config")]);
        assert_eq!(config_credential_in(&files, "feed", &vars(&[])), Some(Ok(Credential::basic("repo", "p"))));
        assert_eq!(config_credential_in(&files, "other", &vars(&[])), Some(Ok(Credential::basic("user", "p"))));
        assert_eq!(config_credential_in(&files, "nuget.org", &vars(&[])), None);
    }

    #[test]
    fn external_feed_endpoints_match_the_source() {
        let json = r#"{"endpointCredentials":[{"endpoint":"https://pkgs.dev.azure.com/o/_packaging/f/nuget/v3/index.json","password":"pat"},{"endpoint":"https://nuget.pkg.github.com/o/index.json","username":"gh","password":"ghp"}]}"#;
        assert_eq!(external_endpoint(json, "https://PKGS.dev.azure.com/o/_packaging/f/nuget/v3/index.json/"), Some(Credential::basic("VssSessionToken", "pat")));
        assert_eq!(external_endpoint(json, "https://nuget.pkg.github.com/o/index.json"), Some(Credential::basic("gh", "ghp")));
        assert_eq!(external_endpoint(json, "https://api.nuget.org/v3/index.json"), None);
        assert_eq!(external_endpoint("not json", "x"), None);
    }

    #[test]
    fn base64_decodes() {
        assert_eq!(base64_decode("TnVHZXQ=").unwrap(), b"NuGet");
        assert_eq!(base64_decode("TnVH\nZXQh").unwrap(), b"NuGet!");
        assert!(base64_decode("@@").is_none());
    }

    #[test]
    fn finds_device_flow_url_and_code() {
        let (url, code) = device_code("[CredentialProvider]To sign in, use a web browser to open the page https://microsoft.com/devicelogin and enter the code ABCD12345 to authenticate.");
        assert_eq!(url.as_deref(), Some("https://microsoft.com/devicelogin"));
        assert_eq!(code.as_deref(), Some("ABCD12345"));
        assert_eq!(device_code("Using cached token"), (None, None));
    }

    #[test]
    fn discovers_plugins_by_convention_and_explicit_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let cp = root.join(".nuget/plugins/netcore/CredentialProvider.Microsoft");
        std::fs::create_dir_all(&cp).unwrap();
        std::fs::write(cp.join("CredentialProvider.Microsoft.dll"), "").unwrap();
        std::fs::write(cp.join("Other.dll"), "").unwrap();
        let bin = root.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let exe = bin.join(if cfg!(windows) { "nuget-plugin-test.exe" } else { "nuget-plugin-test" });
        std::fs::write(&exe, "").unwrap();
        let dotnet = Path::new("/usr/local/share/dotnet/dotnet");
        let found = discover_plugins_with(&vars(&[]), Some(root.clone()), Some(bin.clone().into_os_string()), Some(dotnet));
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].program, dotnet);
        assert_eq!(found[0].args, vec![cp.join("CredentialProvider.Microsoft.dll").to_string_lossy().into_owned(), "-Plugin".to_string()]);
        assert_eq!(found[1].program, exe);
        // Without dotnet a .dll plugin can't run.
        assert_eq!(discover_plugins_with(&vars(&[]), Some(root.clone()), None, None).len(), 0);
        // NUGET_PLUGIN_PATHS replaces the conventional folder.
        let explicit = root.join("custom.exe");
        std::fs::write(&explicit, "").unwrap();
        let leaked: &'static str = Box::leak(explicit.to_string_lossy().into_owned().into_boxed_str());
        let list: &'static [(&'static str, &'static str)] = Box::leak(Box::new([("NUGET_PLUGIN_PATHS", leaked)]));
        let found = discover_plugins_with(&vars(list), Some(root.clone()), None, Some(dotnet));
        assert_eq!(found.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["custom"]);
    }

    // ---------------------------------------------------------------- fake credential provider

    const FAKE: &str = "dotcraft-fake-plugin=";

    /// Run as a plugin (see [`fake_plugin`]), this test speaks the protocol on stdio like the
    /// Azure Artifacts Credential Provider: it starts its own handshake, logs a device flow
    /// message when interactive and returns a token only then (or always in `silent` mode).
    /// In a normal test run it does nothing.
    #[test]
    fn fake_credential_provider_main() {
        let Some(mode) = std::env::args().find_map(|a| a.strip_prefix(FAKE).map(str::to_string)) else { return };
        let mut out = std::io::stdout();
        let mut send = |v: Value| {
            writeln!(out, "{v}").unwrap();
            out.flush().unwrap();
        };
        send(json!({"RequestId": "p1", "Type": "Request", "Method": "Handshake", "Payload": {"ProtocolVersion": "2.0.0", "MinimumProtocolVersion": "1.0.0"}}));
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
            if msg["Type"] != "Request" {
                continue;
            }
            let id = msg["RequestId"].as_str().unwrap_or("").to_string();
            let method = msg["Method"].as_str().unwrap_or("").to_string();
            let reply = |payload: Value| json!({"RequestId": id, "Type": "Response", "Method": method, "Payload": payload});
            match method.as_str() {
                "Handshake" => send(reply(json!({"ResponseCode": "Success", "ProtocolVersion": "2.0.0"}))),
                "GetOperationClaims" => send(reply(json!({"ResponseCode": "Success", "Claims": ["Authentication"]}))),
                "GetAuthenticationCredentials" => {
                    let p = &msg["Payload"];
                    let interactive = p["IsNonInteractive"] == false;
                    send(json!({"RequestId": id, "Type": "Progress", "Method": method, "Payload": {"Percentage": 0}}));
                    if !p["Uri"].as_str().unwrap_or("").ends_with("/v3/index.json") {
                        send(reply(json!({"ResponseCode": "NotFound", "Message": "unknown feed"})));
                    } else if interactive || mode == "silent" {
                        if interactive {
                            send(json!({"RequestId": "p2", "Type": "Request", "Method": "Log", "Payload": {"LogLevel": "Minimal", "Message": "[CredentialProvider]To sign in, use a web browser to open the page https://microsoft.com/devicelogin and enter the code FAKE1234 to authenticate."}}));
                        }
                        send(reply(json!({"ResponseCode": "Success", "Username": "VssSessionToken", "Password": "plugin-token", "AuthenticationTypes": ["Basic"]})));
                    } else {
                        send(reply(json!({"ResponseCode": "NotFound", "Message": "interactive sign-in required"})));
                    }
                }
                "Close" => {
                    send(reply(json!({"ResponseCode": "Success"})));
                    break;
                }
                _ => send(reply(json!({"ResponseCode": "Success"}))),
            }
        }
        std::process::exit(0);
    }

    /// This test binary, started so that it runs only [`fake_credential_provider_main`].
    fn fake_plugin(mode: &str) -> Plugin {
        let args = ["nuget_auth::tests::fake_credential_provider_main", &format!("{FAKE}{mode}"), "--exact", "--nocapture", "--test-threads=1"];
        Plugin { name: "FakeProvider".into(), program: std::env::current_exe().unwrap(), args: args.iter().map(|a| a.to_string()).collect() }
    }

    async fn ask(mode: &str, uri: &str, interactive: bool) -> (anyhow::Result<PluginAnswer>, Vec<String>) {
        let log = Arc::new(Mutex::new(Vec::new()));
        let l2 = log.clone();
        let sink = move |s: String| l2.lock().unwrap().push(s);
        let r = query_plugin(&fake_plugin(mode), uri, interactive, false, Duration::from_secs(30), &sink).await;
        let lines = log.lock().unwrap().clone();
        (r, lines)
    }

    #[test]
    fn plugin_protocol_silent_then_interactive() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let feed = "https://pkgs.dev.azure.com/org/_packaging/feed/nuget/v3/index.json";
            // Non-interactive: the provider needs a sign-in and says so.
            let (r, log) = ask("interactive", feed, false).await;
            assert_eq!(r.unwrap(), PluginAnswer::NotFound(Some("interactive sign-in required".into())));
            assert!(log.is_empty());
            // Interactive: the device flow message arrives through Log, then the token.
            let (r, log) = ask("interactive", feed, true).await;
            assert_eq!(r.unwrap(), PluginAnswer::Credential(Credential::basic("VssSessionToken", "plugin-token")));
            assert_eq!(log.len(), 1);
            assert_eq!(device_code(&log[0]).1.as_deref(), Some("FAKE1234"));
            // Feeds it does not know.
            let (r, _) = ask("silent", "https://example.com/feed", false).await;
            assert_eq!(r.unwrap(), PluginAnswer::NotFound(Some("unknown feed".into())));
        });
    }

    // ---------------------------------------------------------------- local HTTP feed

    struct Seen {
        path: String,
        auth: Option<String>,
    }

    /// A tiny HTTP/1.1 server: `respond(path, authorization)` gives (status, headers, body).
    fn serve(respond: impl Fn(&str, Option<&str>) -> (u16, Vec<(String, String)>, String) + Send + 'static) -> (String, Arc<Mutex<Vec<Seen>>>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s2 = seen.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                // Accepted sockets inherit nonblocking from the listener on BSD/macOS.
                stream.set_nonblocking(false).unwrap();
                let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
                let mut first = String::new();
                if reader.read_line(&mut first).is_err() {
                    continue;
                }
                let path = first.split_whitespace().nth(1).unwrap_or("").to_string();
                let mut auth = None;
                loop {
                    let mut h = String::new();
                    if reader.read_line(&mut h).unwrap_or(0) == 0 || h.trim().is_empty() {
                        break;
                    }
                    if let Some((k, v)) = h.split_once(':')
                        && k.eq_ignore_ascii_case("authorization")
                    {
                        auth = Some(v.trim().to_string());
                    }
                }
                let (status, headers, body) = respond(&path, auth.as_deref());
                s2.lock().unwrap().push(Seen { path, auth });
                let mut resp = format!("HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n", body.len());
                for (k, v) in headers {
                    resp.push_str(&format!("{k}: {v}\r\n"));
                }
                resp.push_str("\r\n");
                resp.push_str(&body);
                let _ = stream.write_all(resp.as_bytes());
                let _ = stream.flush();
                let mut sink = [0u8; 1];
                let _ = stream.set_read_timeout(Some(Duration::from_millis(10)));
                let _ = stream.read(&mut sink);
            }
        });
        (base, seen)
    }

    #[test]
    fn redirects_and_foreign_resources_never_receive_the_credential() {
        let (other, other_seen) = serve(|_, _| (200, vec![], r#"{"totalHits":1,"data":[{"id":"A","version":"1.0.0"}],"versions":["1.0.0"]}"#.to_string()));
        let expected = "Basic bWU6cHc=";
        let other2 = other.clone();
        let feed_base: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
        let fb = feed_base.clone();
        let (feed, feed_seen) = serve(move |path, auth| {
            if auth != Some(expected) {
                return (401, vec![], String::new());
            }
            let me = fb.lock().unwrap().clone();
            match path {
                "/v3/index.json" => (200, vec![], format!(r#"{{"resources":[{{"@id":"{me}/query","@type":"SearchQueryService"}},{{"@id":"{other2}/flat/","@type":"PackageBaseAddress/3.0.0"}}]}}"#)),
                p if p.starts_with("/query") => (302, vec![("Location".into(), format!("{other2}/elsewhere"))], String::new()),
                _ => (404, vec![], String::new()),
            }
        });
        *feed_base.lock().unwrap() = feed.clone();
        let index = format!("{feed}/v3/index.json");
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let anon = NugetClient::with_credentials(&index, None).unwrap();
            assert!(is_auth_error(&anon.service_index().await.unwrap_err()));
            let client = NugetClient::with_credentials(&index, Some(Credential::basic("me", "pw"))).unwrap();
            // Search: the feed redirects to the other host.
            let r = client.search("a", false, 0, 10).await.unwrap();
            assert_eq!(r.packages.len(), 1);
            // Versions: the flat container is on the other host.
            client.versions("A").await.unwrap();
        });
        let feed_seen = feed_seen.lock().unwrap();
        assert!(feed_seen.iter().any(|s| s.path == "/v3/index.json" && s.auth.as_deref() == Some(expected)));
        assert!(feed_seen.iter().any(|s| s.path.starts_with("/query") && s.auth.as_deref() == Some(expected)));
        let other_seen = other_seen.lock().unwrap();
        assert!(other_seen.iter().any(|s| s.path == "/elsewhere"), "the redirect was followed");
        assert!(other_seen.iter().any(|s| s.path == "/flat/a/index.json"));
        assert!(other_seen.iter().all(|s| s.auth.is_none()), "the credential leaked to another host");
    }

    #[test]
    fn connect_uses_config_credentials_then_a_silent_provider() {
        let expected_cfg = "Basic bWU6cHc=";
        let (feed, _) = serve(move |path, auth| match (path, auth) {
            ("/v3/index.json", Some(a)) if a == expected_cfg || a == "Basic VnNzU2Vzc2lvblRva2VuOnBsdWdpbi10b2tlbg==" => (200, vec![], r#"{"resources":[]}"#.into()),
            _ => (401, vec![], String::new()),
        });
        let url = format!("{feed}/v3/index.json");
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let session = Session::default();
            // nuget.config credentials.
            let f = Feed { name: "Feed".into(), url: url.clone(), config: Some(Ok(Credential::basic("me", "pw"))), endpoint: None };
            let c = connect(&f, &session, &[]).await;
            assert_eq!(c.status, SourceStatus::Ok);
            assert!(c.client.is_some());
            // No credentials, no providers: a clear failure.
            let f = Feed { name: "Feed".into(), url: url.clone(), config: None, endpoint: None };
            let c = connect(&f, &Session::default(), &[]).await;
            assert!(matches!(&c.status, SourceStatus::Failed(why) if why.contains("credential provider")), "{:?}", c.status);
            // An encrypted password off Windows: the reason is kept.
            let f = Feed { name: "Feed".into(), url: url.clone(), config: Some(Err("DPAPI only".into())), endpoint: None };
            let c = connect(&f, &Session::default(), &[]).await;
            assert!(matches!(&c.status, SourceStatus::Failed(why) if why.contains("DPAPI only")), "{:?}", c.status);
            // A provider that needs interaction: needs sign-in; then signing in works and is cached.
            let plugin = fake_plugin("interactive");
            let f = Feed { name: "Feed".into(), url: url.clone(), config: None, endpoint: None };
            let session = Session::default();
            let c = connect(&f, &session, std::slice::from_ref(&plugin)).await;
            assert!(matches!(&c.status, SourceStatus::NeedsSignIn(why) if why.contains("401")), "{:?}", c.status);
            let messages = Arc::new(Mutex::new(Vec::<String>::new()));
            let m2 = messages.clone();
            let log = move |s: String| m2.lock().unwrap().push(s);
            let c = sign_in(&f, &session, std::slice::from_ref(&plugin), &log).await;
            assert_eq!(c.status, SourceStatus::Ok);
            assert_eq!(messages.lock().unwrap().len(), 1);
            // The session remembers it: no provider needed now.
            let c = connect(&f, &session, &[]).await;
            assert_eq!(c.status, SourceStatus::Ok);
            // A provider with a cached token works silently.
            let c = connect(&f, &Session::default(), &[fake_plugin("silent")]).await;
            assert_eq!(c.status, SourceStatus::Ok);
        });
    }
}
