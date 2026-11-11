//! Pull requests on the repository's hosting service (GitHub and Azure DevOps Repos) through
//! their REST APIs: detecting the service from a remote URL, listing and opening pull requests,
//! commenting, voting, completing and creating them. Personal access tokens are kept in the
//! OS keychain ([`secrets`]); they are only ever sent to the repository's API origin.

pub mod azure;
pub mod fake;
pub mod github;
pub mod mock;
pub mod secrets;

use std::time::Duration;

/// The hosting service of a repository.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HostKind {
    GitHub,
    AzureDevOps,
}

impl HostKind {
    pub fn name(self) -> &'static str {
        match self {
            HostKind::GitHub => "GitHub",
            HostKind::AzureDevOps => "Azure DevOps",
        }
    }
}

/// A repository on a hosting service, from a remote URL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostRepo {
    pub kind: HostKind,
    /// Web host: `github.com`, `dev.azure.com`, a GitHub Enterprise host.
    pub host: String,
    /// GitHub: the owner. Azure DevOps: the organization.
    pub owner: String,
    /// Azure DevOps project (empty on GitHub).
    pub project: String,
    pub name: String,
    /// REST API root: `https://api.github.com`, `https://ghe.example.com/api/v3`,
    /// `https://dev.azure.com/org`.
    pub api: String,
    /// Web root for links: `https://github.com`, `https://dev.azure.com/org`.
    pub web: String,
}

impl HostRepo {
    /// The service and repository of a remote URL (HTTPS, SSH or scp-like), or `None` when it
    /// is neither GitHub nor Azure DevOps.
    pub fn parse(url: &str) -> Option<HostRepo> {
        let (host, path) = split_url(url.trim())?;
        let host = host.to_ascii_lowercase();
        let segments: Vec<String> = path.trim_matches('/').split('/').filter(|s| !s.is_empty()).map(percent_decode).collect();
        if host == "dev.azure.com" || host == "ssh.dev.azure.com" || host.ends_with(".visualstudio.com") {
            return parse_azure(&host, &segments);
        }
        if host == "github.com" || host == "ssh.github.com" || host.split('.').any(|l| l == "github") {
            let [owner, name, ..] = segments.as_slice() else { return None };
            let name = name.strip_suffix(".git").unwrap_or(name);
            if owner.is_empty() || name.is_empty() {
                return None;
            }
            let public = host == "github.com" || host == "ssh.github.com";
            let web_host = if public { "github.com".to_string() } else { host.clone() };
            return Some(HostRepo {
                kind: HostKind::GitHub,
                api: if public { "https://api.github.com".into() } else { format!("https://{web_host}/api/v3") },
                web: format!("https://{web_host}"),
                host: web_host,
                owner: owner.clone(),
                project: String::new(),
                name: name.to_string(),
            });
        }
        None
    }

    /// `owner/name`, or `organization/project/name` on Azure DevOps.
    pub fn display_name(&self) -> String {
        match self.kind {
            HostKind::GitHub => format!("{}/{}", self.owner, self.name),
            HostKind::AzureDevOps if self.project == self.name => format!("{}/{}", self.owner, self.name),
            HostKind::AzureDevOps => format!("{}/{}/{}", self.owner, self.project, self.name),
        }
    }

    /// The keychain account of this service's token: one per GitHub host, one per Azure
    /// DevOps organization.
    pub fn account(&self) -> String {
        match self.kind {
            HostKind::GitHub => format!("github:{}", self.host),
            HostKind::AzureDevOps => format!("azure:{}/{}", self.host, self.owner),
        }
    }

    /// Where the user creates a personal access token.
    pub fn token_page(&self) -> String {
        match self.kind {
            HostKind::GitHub => format!("{}/settings/tokens/new?scopes=repo&description=DotCraft", self.web),
            HostKind::AzureDevOps => format!("{}/_usersSettings/tokens", self.web),
        }
    }

    /// The pull request list on the web.
    pub fn pulls_page(&self) -> String {
        match self.kind {
            HostKind::GitHub => format!("{}/{}/{}/pulls", self.web, enc(&self.owner), enc(&self.name)),
            HostKind::AzureDevOps => format!("{}/{}/_git/{}/pullrequests", self.web, enc(&self.project), enc(&self.name)),
        }
    }
}

/// `(host, path)` of `https://user@host:port/path`, `ssh://git@host/path` or `git@host:path`.
fn split_url(url: &str) -> Option<(String, String)> {
    if let Some((scheme, rest)) = url.split_once("://") {
        if !matches!(scheme.to_ascii_lowercase().as_str(), "http" | "https" | "ssh" | "git") {
            return None;
        }
        let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
        let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
        let host = host.split(':').next().unwrap_or(host);
        return (!host.is_empty()).then(|| (host.to_string(), path.to_string()));
    }
    // scp-like: [user@]host:path
    let (authority, path) = url.split_once(':')?;
    if authority.contains('/') || path.starts_with("//") {
        return None;
    }
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    (!host.is_empty()).then(|| (host.to_string(), path.to_string()))
}

fn parse_azure(host: &str, s: &[String]) -> Option<HostRepo> {
    let (org, project, name) = if host == "ssh.dev.azure.com" || host.starts_with("vs-ssh.") {
        // v3/{org}/{project}/{repo}
        match s {
            [v, org, project, name] if v == "v3" => (org.clone(), project.clone(), name.clone()),
            _ => return None,
        }
    } else if host == "dev.azure.com" {
        // {org}/{project}/_git/{repo} or {org}/_git/{repo}
        match s {
            [org, project, g, name, ..] if g == "_git" => (org.clone(), project.clone(), name.clone()),
            [org, g, name, ..] if g == "_git" => (org.clone(), name.clone(), name.clone()),
            _ => return None,
        }
    } else {
        // {org}.visualstudio.com/[DefaultCollection/]{project}/_git/{repo}
        let org = host.strip_suffix(".visualstudio.com")?.to_string();
        let s: Vec<&String> = s.iter().filter(|x| !x.eq_ignore_ascii_case("DefaultCollection")).collect();
        match s.as_slice() {
            [project, g, name, ..] if *g == "_git" => (org, (*project).clone(), (*name).clone()),
            [g, name, ..] if *g == "_git" => (org, (*name).clone(), (*name).clone()),
            _ => return None,
        }
    };
    let name = name.strip_suffix(".git").unwrap_or(&name).to_string();
    Some(azure_repo(org, project, name))
}

fn azure_repo(org: String, project: String, name: String) -> HostRepo {
    let web = format!("https://dev.azure.com/{}", enc(&org));
    HostRepo { kind: HostKind::AzureDevOps, host: "dev.azure.com".into(), owner: org, project, name, api: web.clone(), web }
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Ok(v) = u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("zz"), 16)
        {
            out.push(v);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Percent-encodes a path segment or query value (everything but RFC 3986 unreserved).
pub fn enc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Encodes a file path, keeping its `/` separators.
pub fn enc_path(path: &str) -> String {
    path.split('/').map(enc).collect::<Vec<_>>().join("/")
}

// ------------------------------------------------------------------ model

/// The state of a pull request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrState {
    Open,
    Draft,
    Merged,
    Closed,
}

/// A reviewer's vote.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Vote {
    None,
    Commented,
    Approved,
    /// Azure DevOps "Approved with suggestions".
    ApprovedWithSuggestions,
    /// GitHub "Changes requested", Azure DevOps "Waiting for author".
    ChangesRequested,
    /// Azure DevOps "Rejected".
    Rejected,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reviewer {
    pub name: String,
    pub vote: Vote,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PullRequest {
    /// The number on GitHub, the pull request id on Azure DevOps.
    pub id: u64,
    pub title: String,
    pub description: String,
    pub author: String,
    /// GitHub login or Azure DevOps identity id.
    pub author_id: String,
    pub source_branch: String,
    pub target_branch: String,
    pub state: PrState,
    /// ISO 8601.
    pub created: String,
    pub web_url: String,
    pub head_sha: String,
    pub base_sha: String,
    pub reviewers: Vec<Reviewer>,
    /// `Some(false)` when it has merge conflicts; `None` while the service computes it.
    pub mergeable: Option<bool>,
}

impl PullRequest {
    pub fn is_open(&self) -> bool {
        matches!(self.state, PrState::Open | PrState::Draft)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileChange {
    Added,
    Modified,
    Deleted,
    Renamed,
}

/// A file changed by a pull request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrFile {
    /// Repository-relative, `/`-separated, without a leading `/`.
    pub path: String,
    /// The path before a rename.
    pub old_path: Option<String>,
    pub change: FileChange,
    pub additions: Option<u32>,
    pub deletions: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Comment {
    pub id: String,
    pub author: String,
    pub body: String,
    pub created: String,
}

/// A conversation: a general comment or one on a line of a file, with its replies.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Thread {
    pub id: String,
    pub path: Option<String>,
    pub line: Option<u32>,
    pub resolved: bool,
    pub comments: Vec<Comment>,
}

/// What the signed-in user may do with a pull request.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Permissions {
    pub can_review: bool,
    pub can_merge: bool,
}

/// An opened pull request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrDetails {
    pub pr: PullRequest,
    pub files: Vec<PrFile>,
    pub threads: Vec<Thread>,
    /// The commits the file diffs compare (merge base ↔ source tip).
    pub base_commit: String,
    pub head_commit: String,
    pub permissions: Permissions,
}

/// The signed-in user and what the repository allows them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Viewer {
    /// GitHub login or Azure DevOps identity id.
    pub id: String,
    pub name: String,
    pub default_branch: String,
    /// GitHub: push access (needed to merge). Always true on Azure DevOps, whose branch
    /// policies are checked when completing.
    pub can_push: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NewPullRequest {
    pub title: String,
    pub description: String,
    /// Branch names without `refs/heads/`.
    pub source: String,
    pub target: String,
    pub draft: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MergeMethod {
    #[default]
    Merge,
    Squash,
    Rebase,
}

impl MergeMethod {
    pub const ALL: [MergeMethod; 3] = [MergeMethod::Merge, MergeMethod::Squash, MergeMethod::Rebase];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Review {
    Approve,
    RequestChanges,
}

// ------------------------------------------------------------------ errors

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HostError {
    /// The token is missing, expired or revoked.
    #[error("The token was refused: {0}")]
    Unauthorized(String),
    #[error("Not allowed: {0}")]
    Forbidden(String),
    #[error("Not found: {0}")]
    NotFound(String),
    #[error("{0}")]
    Rejected(String),
    #[error("Network error: {0}")]
    Network(String),
    #[error("Unexpected response: {0}")]
    Parse(String),
}

pub type HostResult<T> = std::result::Result<T, HostError>;

// ------------------------------------------------------------------ client

/// A REST client for one repository with one token.
#[derive(Clone)]
pub struct HostClient {
    pub repo: HostRepo,
    token: String,
    http: reqwest::Client,
}

impl std::fmt::Debug for HostClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "HostClient({}, ***)", self.repo.display_name())
    }
}

impl HostClient {
    pub fn new(repo: HostRepo, token: &str) -> HostResult<HostClient> {
        let http = reqwest::Client::builder()
            .user_agent(concat!("DotCraft/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(60))
            // Never forward the token to another origin.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| HostError::Network(e.to_string()))?;
        Ok(HostClient { repo, token: token.trim().to_string(), http })
    }

    fn request(&self, method: reqwest::Method, url: &str) -> reqwest::RequestBuilder {
        let req = self.http.request(method, url);
        match self.repo.kind {
            HostKind::GitHub => req.bearer_auth(&self.token).header("Accept", "application/vnd.github+json").header("X-GitHub-Api-Version", "2022-11-28"),
            HostKind::AzureDevOps => req.basic_auth("", Some(&self.token)).header("Accept", "application/json"),
        }
    }

    /// Sends a request and returns the body; HTTP errors become [`HostError`]s whose text never
    /// contains the token.
    async fn send(&self, req: reqwest::RequestBuilder) -> HostResult<String> {
        let resp = req.send().await.map_err(|e| HostError::Network(self.redact(&e.without_url().to_string())))?;
        let status = resp.status();
        let body = resp.text().await.map_err(|e| HostError::Network(self.redact(&e.to_string())))?;
        if status.is_success() {
            return Ok(body);
        }
        let message = self.redact(&error_message(&body).unwrap_or_else(|| status.canonical_reason().unwrap_or("error").to_string()));
        Err(match status.as_u16() {
            401 => HostError::Unauthorized(message),
            // Azure DevOps answers 203 + a sign-in page to bad PATs; GitHub 403 for missing scopes.
            403 => HostError::Forbidden(message),
            404 => HostError::NotFound(message),
            300..=399 => HostError::Unauthorized(message),
            _ => HostError::Rejected(message),
        })
    }

    async fn json(&self, req: reqwest::RequestBuilder) -> HostResult<serde_json::Value> {
        let body = self.send(req).await?;
        serde_json::from_str(&body).map_err(|_| {
            // Azure DevOps answers a bad PAT with an HTML sign-in page and 203.
            if body.trim_start().starts_with('<') { HostError::Unauthorized("the service answered with a sign-in page".into()) } else { HostError::Parse(truncate(&body, 200)) }
        })
    }

    async fn get(&self, url: &str) -> HostResult<serde_json::Value> {
        self.json(self.request(reqwest::Method::GET, url)).await
    }

    async fn post(&self, url: &str, body: &serde_json::Value) -> HostResult<serde_json::Value> {
        self.json(self.request(reqwest::Method::POST, url).json(body)).await
    }

    fn redact(&self, text: &str) -> String {
        if self.token.is_empty() { text.to_string() } else { text.replace(&self.token, "***") }
    }

    /// The signed-in user, the default branch and whether they can push (checks the token).
    pub async fn viewer(&self) -> HostResult<Viewer> {
        match self.repo.kind {
            HostKind::GitHub => github::viewer(self).await,
            HostKind::AzureDevOps => azure::viewer(self).await,
        }
    }

    /// Open (and draft) pull requests, newest first.
    pub async fn list(&self) -> HostResult<Vec<PullRequest>> {
        match self.repo.kind {
            HostKind::GitHub => github::list(self).await,
            HostKind::AzureDevOps => azure::list(self).await,
        }
    }

    /// A pull request with its files and conversations.
    pub async fn details(&self, id: u64, viewer: &Viewer) -> HostResult<PrDetails> {
        match self.repo.kind {
            HostKind::GitHub => github::details(self, id, viewer).await,
            HostKind::AzureDevOps => azure::details(self, id, viewer).await,
        }
    }

    /// A file's text at a commit (`None` when it does not exist there).
    pub async fn file_text(&self, path: &str, commit: &str) -> HostResult<Option<String>> {
        if commit.is_empty() {
            return Ok(None);
        }
        let r = match self.repo.kind {
            HostKind::GitHub => github::file_text(self, path, commit).await,
            HostKind::AzureDevOps => azure::file_text(self, path, commit).await,
        };
        match r {
            Err(HostError::NotFound(_)) => Ok(None),
            r => r,
        }
    }

    /// A new general comment.
    pub async fn comment(&self, id: u64, body: &str) -> HostResult<()> {
        match self.repo.kind {
            HostKind::GitHub => github::comment(self, id, body).await,
            HostKind::AzureDevOps => azure::comment(self, id, body).await,
        }
    }

    /// A reply in a conversation.
    pub async fn reply(&self, id: u64, thread: &Thread, body: &str) -> HostResult<()> {
        match self.repo.kind {
            HostKind::GitHub => github::reply(self, id, thread, body).await,
            HostKind::AzureDevOps => azure::reply(self, id, thread, body).await,
        }
    }

    /// Approves or requests changes (Azure DevOps: votes "Approved" / "Waiting for author").
    pub async fn review(&self, id: u64, review: Review, body: &str, viewer: &Viewer) -> HostResult<()> {
        match self.repo.kind {
            HostKind::GitHub => github::review(self, id, review, body).await,
            HostKind::AzureDevOps => azure::review(self, id, review, body, viewer).await,
        }
    }

    /// Merges (completes) a pull request.
    pub async fn merge(&self, pr: &PullRequest, method: MergeMethod) -> HostResult<()> {
        match self.repo.kind {
            HostKind::GitHub => github::merge(self, pr, method).await,
            HostKind::AzureDevOps => azure::merge(self, pr, method).await,
        }
    }

    pub async fn create(&self, new: &NewPullRequest) -> HostResult<PullRequest> {
        match self.repo.kind {
            HostKind::GitHub => github::create(self, new).await,
            HostKind::AzureDevOps => azure::create(self, new).await,
        }
    }
}

/// The message of a JSON error body (`message` on GitHub, `message` / `value.Message` on Azure).
fn error_message(body: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let mut m = v.get("message").and_then(|m| m.as_str()).map(str::to_string)?;
    // GitHub validation errors carry the details in `errors`.
    if let Some(errors) = v.get("errors").and_then(|e| e.as_array()) {
        let details: Vec<String> = errors.iter().filter_map(|e| e.get("message").and_then(|m| m.as_str()).map(str::to_string).or_else(|| e.as_str().map(str::to_string))).collect();
        if !details.is_empty() {
            m = format!("{m}: {}", details.join("; "));
        }
    }
    Some(m)
}

fn truncate(s: &str, n: usize) -> String {
    match s.char_indices().nth(n) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s.to_string(),
    }
}

/// A string field of a JSON value (`""` when missing).
fn s(v: &serde_json::Value, key: &str) -> String {
    v.get(key).and_then(|x| x.as_str()).unwrap_or_default().to_string()
}

/// The GitHub CLI's token for `host` (`gh auth token`), when it is installed and signed in.
pub async fn gh_cli_token(host: &str) -> Option<String> {
    let out = tokio::process::Command::new("gh").args(["auth", "token", "--hostname", host]).stdin(std::process::Stdio::null()).output().await.ok()?;
    let token = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (out.status.success() && !token.is_empty()).then_some(token)
}

#[cfg(test)]
mod tests;
