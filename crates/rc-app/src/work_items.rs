//! Work item links of the Git windows: the issue trackers of the repository's remotes, the
//! custom patterns of the settings and the ids offered by the completion of `#` / `AB#` in
//! the commit message. Ids come from recent commit messages (offline); when a GitHub, GitLab or
//! Azure DevOps token is available (environment variables, or `gh auth token`), the open
//! issues / work items of the tracker are read in the background and listed first.

use rc_core::settings::IssueLink;
use rc_core::{Inbox, Runtime, Waker};
use rc_vcs::work_items::{self, LinkPattern, Linker, Tracker, TrackerKind, WorkItem};
use rc_vcs::{Commit, Repository};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

enum Event {
    Trackers(PathBuf, Vec<Tracker>),
    Remote(PathBuf, Result<Vec<WorkItem>, String>),
}

pub struct WorkItems {
    /// The repository the trackers belong to.
    root: Option<PathBuf>,
    trackers: Vec<Tracker>,
    /// The settings' patterns the linker was built with.
    patterns: Option<Vec<IssueLink>>,
    linker: Arc<Linker>,
    /// Invalid custom patterns.
    pub errors: Vec<String>,
    /// Log the offline items were computed from: (length, newest hash).
    history_of: (usize, Option<String>),
    history: Vec<WorkItem>,
    /// Open items read from the tracker's API.
    remote: Vec<WorkItem>,
    pub remote_error: Option<String>,
    inbox: Inbox<Event>,
}

impl WorkItems {
    pub fn new(waker: Waker) -> Self {
        WorkItems {
            root: None,
            trackers: Vec::new(),
            patterns: None,
            linker: Arc::new(Linker::default()),
            errors: Vec::new(),
            history_of: (0, None),
            history: Vec::new(),
            remote: Vec::new(),
            remote_error: None,
            inbox: Inbox::new(waker),
        }
    }

    /// Once per frame of a Git window: reads the remotes of a newly opened repository, rebuilds
    /// the linker when the trackers or the settings change and the offline ids when the log does.
    pub fn sync(&mut self, repo: Option<&Repository>, patterns: &[IssueLink], log: &[Commit]) {
        let mut rebuild = false;
        for ev in self.inbox.drain() {
            match ev {
                Event::Trackers(root, trackers) if self.root.as_ref() == Some(&root) => {
                    if let Some(t) = trackers.first().cloned() {
                        let out = self.inbox.sender();
                        let r = root.clone();
                        Runtime::spawn(async move {
                            if let Some(token) = discover_token(&t).await {
                                out.send(Event::Remote(r, fetch_open_items(&t, &token).await));
                            }
                        });
                    }
                    self.trackers = trackers;
                    rebuild = true;
                }
                Event::Remote(root, r) if self.root.as_ref() == Some(&root) => match r {
                    Ok(items) => self.remote = items,
                    Err(e) => self.remote_error = Some(e),
                },
                _ => {}
            }
        }
        let root = repo.map(|r| r.root().to_path_buf());
        if root != self.root {
            self.root = root.clone();
            self.trackers.clear();
            self.remote.clear();
            self.remote_error = None;
            rebuild = true;
            if let (Some(repo), Some(root)) = (repo.cloned(), root) {
                let out = self.inbox.sender();
                Runtime::spawn(async move {
                    let mut remotes = repo.remotes().await.unwrap_or_default();
                    // `origin` decides what `#` means.
                    remotes.sort_by_key(|r| r.name != "origin");
                    let trackers = remotes.iter().filter_map(|r| work_items::tracker_from_remote(&r.fetch_url)).collect();
                    out.send(Event::Trackers(root, trackers));
                });
            }
        }
        if rebuild || self.patterns.as_deref() != Some(patterns) {
            self.set_linker(patterns);
        }
        let key = (log.len(), log.first().map(|c| c.hash.clone()));
        if rebuild || key != self.history_of {
            self.history = work_items::recent_work_items(&self.linker, log.iter().map(|c| (c.subject.as_str(), c.body.as_str())));
            self.history_of = key;
        }
    }

    fn set_linker(&mut self, patterns: &[IssueLink]) {
        let custom: Vec<LinkPattern> = patterns.iter().map(|p| LinkPattern { pattern: p.pattern.clone(), url: p.url.clone() }).collect();
        let (linker, errors) = Linker::new(&self.trackers, &custom);
        self.linker = Arc::new(linker);
        self.errors = errors;
        self.patterns = Some(patterns.to_vec());
    }

    pub fn linker(&self) -> Arc<Linker> {
        self.linker.clone()
    }

    #[cfg(test)]
    pub fn find(&self, text: &str) -> Vec<rc_vcs::WorkItemLink> {
        self.linker.find(text)
    }

    pub fn trackers(&self) -> &[Tracker] {
        &self.trackers
    }

    /// The completion candidates: the tracker's open items, then the ids of recent commits.
    pub fn items(&self) -> Vec<WorkItem> {
        let mut out = self.remote.clone();
        for w in &self.history {
            if !out.iter().any(|r| r.kind == w.kind && r.id == w.id) {
                out.push(w.clone());
            }
        }
        out
    }

    /// Whether `items` came (partly) from the tracker's API.
    pub fn online(&self) -> bool {
        !self.remote.is_empty()
    }
}

/// Syncs the links with the open repository, its log and the settings (once per frame of a
/// Git window).
pub fn sync(app: &mut crate::app::DotCraftApp) {
    app.vcs.work_items.sync(app.vcs.repo.as_ref(), &app.settings.issue_links, &app.vcs.log);
}

/// A token for the tracker: environment variables first, then the GitHub CLI. Tests never
/// look one up, so they cannot reach a real service.
async fn discover_token(t: &Tracker) -> Option<String> {
    if cfg!(test) {
        return None;
    }
    let env = |names: &[&str]| names.iter().find_map(|n| std::env::var(n).ok().filter(|v| !v.trim().is_empty()));
    match t.kind {
        TrackerKind::GitHub => {
            let from_env = if t.host == "github.com" { env(&["GH_TOKEN", "GITHUB_TOKEN"]) } else { env(&["GH_ENTERPRISE_TOKEN", "GITHUB_ENTERPRISE_TOKEN"]) };
            if from_env.is_some() {
                return from_env;
            }
            // Apps started from the Finder do not get Homebrew's PATH.
            for gh in ["gh", "/opt/homebrew/bin/gh", "/usr/local/bin/gh"] {
                let run = tokio::process::Command::new(gh).args(["auth", "token", "--hostname", &t.host]).stdin(std::process::Stdio::null()).kill_on_drop(true).output();
                if let Ok(Ok(out)) = tokio::time::timeout(Duration::from_secs(5), run).await
                    && out.status.success()
                {
                    let token = String::from_utf8_lossy(&out.stdout).trim().to_string();
                    return (!token.is_empty()).then_some(token);
                }
            }
            None
        }
        TrackerKind::GitLab => env(&["GITLAB_TOKEN"]),
        TrackerKind::Azure => env(&["AZURE_DEVOPS_EXT_PAT", "SYSTEM_ACCESSTOKEN"]),
    }
}

/// The open issues / active work items of a tracker, most recently updated first.
pub async fn fetch_open_items(t: &Tracker, token: &str) -> Result<Vec<WorkItem>, String> {
    fetch(t, token).await.map_err(|e| format!("{e:#}"))
}

async fn fetch(t: &Tracker, token: &str) -> anyhow::Result<Vec<WorkItem>> {
    use serde_json::Value;
    let client = reqwest::Client::builder().timeout(Duration::from_secs(15)).user_agent(concat!("DotCraft/", env!("CARGO_PKG_VERSION"))).build()?;
    let num = |v: &Value, k: &str| v.get(k).and_then(Value::as_u64);
    let text = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    Ok(match t.kind {
        TrackerKind::GitHub => {
            let list: Value = client
                .get(format!("{}/issues?state=open&sort=updated&per_page=50", t.api))
                .bearer_auth(token)
                .header("Accept", "application/vnd.github+json")
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            list.as_array().into_iter().flatten().filter_map(|i| Some(WorkItem { kind: t.kind, id: num(i, "number")?, title: text(i, "title") })).collect()
        }
        TrackerKind::GitLab => {
            let list: Value = client.get(format!("{}/issues?state=opened&order_by=updated_at&per_page=50", t.api)).header("PRIVATE-TOKEN", token).send().await?.error_for_status()?.json().await?;
            list.as_array().into_iter().flatten().filter_map(|i| Some(WorkItem { kind: t.kind, id: num(i, "iid")?, title: text(i, "title") })).collect()
        }
        TrackerKind::Azure => {
            let query = "SELECT [System.Id] FROM WorkItems WHERE [System.TeamProject] = @project AND [System.State] NOT IN ('Closed', 'Done', 'Removed') ORDER BY [System.ChangedDate] DESC";
            let found: Value = client
                .post(format!("{}/_apis/wit/wiql?$top=50&api-version=7.0", t.api))
                .basic_auth("", Some(token))
                .json(&serde_json::json!({ "query": query }))
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            let ids: Vec<u64> = found.get("workItems").and_then(Value::as_array).into_iter().flatten().filter_map(|w| num(w, "id")).take(50).collect();
            if ids.is_empty() {
                return Ok(Vec::new());
            }
            let list = ids.iter().map(u64::to_string).collect::<Vec<_>>().join(",");
            let items: Value = client
                .get(format!("{}/_apis/wit/workitems?ids={list}&fields=System.Id,System.Title&api-version=7.0", t.api))
                .basic_auth("", Some(token))
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            let mut out: Vec<WorkItem> = items
                .get("value")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|w| Some(WorkItem { kind: t.kind, id: num(w, "id")?, title: w.get("fields").map(|f| text(f, "System.Title")).unwrap_or_default() }))
                .collect();
            // Keep the query's order (most recently changed first).
            out.sort_by_key(|w| ids.iter().position(|&i| i == w.id));
            out
        }
    })
}

/// Opens a link in the browser (tests record it instead).
pub fn open_link(url: &str) {
    #[cfg(test)]
    OPENED.with(|o| o.borrow_mut().push(url.to_string()));
    #[cfg(not(test))]
    crate::ui::run_windows::open_url(url);
}

#[cfg(test)]
thread_local! {
    pub static OPENED: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;

    /// A one-shot HTTP server answering each request with the body `answer` gives for its
    /// request line; returns the base URL and the requests (line + headers + body) it saw.
    fn serve(n: usize, answer: fn(&str) -> String) -> (String, std::thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let mut seen = Vec::new();
            for _ in 0..n {
                let (stream, _) = listener.accept().unwrap();
                stream.set_nonblocking(false).unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut head = String::new();
                let mut len = 0;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        len = v.trim().parse().unwrap();
                    }
                    if line == "\r\n" {
                        break;
                    }
                    head.push_str(&line);
                }
                let mut body = vec![0; len];
                reader.read_exact(&mut body).unwrap();
                let first = head.lines().next().unwrap_or_default().to_string();
                let reply = answer(&first);
                let mut stream = stream;
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}", reply.len()).unwrap();
                seen.push(format!("{head}\n{}", String::from_utf8_lossy(&body)));
            }
            seen
        });
        (base, handle)
    }

    fn tracker(kind: TrackerKind, api: String) -> Tracker {
        Tracker { kind, web: "https://example.invalid/x".into(), api, host: "example.invalid".into() }
    }

    #[test]
    fn github_api_lists_open_issues_with_the_token() {
        let (base, server) = serve(1, |_| r#"[{"number":42,"title":"Coupons expire too early"},{"number":7,"title":"Gift cards","pull_request":{}}]"#.into());
        let t = tracker(TrackerKind::GitHub, format!("{base}/repos/o/r"));
        let items = Runtime::block_on(fetch_open_items(&t, "gh-secret")).unwrap();
        assert_eq!(items.iter().map(|w| (w.id, w.title.as_str())).collect::<Vec<_>>(), vec![(42, "Coupons expire too early"), (7, "Gift cards")]);
        let seen = server.join().unwrap();
        assert!(seen[0].starts_with("GET /repos/o/r/issues?state=open"), "{}", seen[0]);
        assert!(seen[0].to_ascii_lowercase().contains("authorization: bearer gh-secret"));
    }

    #[test]
    fn azure_api_runs_a_wiql_query_and_reads_titles() {
        let (base, server) = serve(2, |line| {
            if line.starts_with("POST") { r#"{"workItems":[{"id":1234},{"id":56}]}"#.into() } else { r#"{"value":[{"id":56,"fields":{"System.Title":"Login fails"}},{"id":1234,"fields":{"System.Title":"Export orders"}}]}"#.into() }
        });
        let t = tracker(TrackerKind::Azure, format!("{base}/org/Project"));
        let items = Runtime::block_on(fetch_open_items(&t, "pat")).unwrap();
        assert_eq!(items.iter().map(|w| (w.id, w.title.as_str())).collect::<Vec<_>>(), vec![(1234, "Export orders"), (56, "Login fails")]);
        let seen = server.join().unwrap();
        assert!(seen[0].starts_with("POST /org/Project/_apis/wit/wiql"), "{}", seen[0]);
        assert!(seen[0].contains("@project"));
        // Basic auth with an empty user: base64(":pat").
        assert!(seen[0].contains("Basic OnBhdA=="), "{}", seen[0]);
        assert!(seen[1].starts_with("GET /org/Project/_apis/wit/workitems?ids=1234,56"), "{}", seen[1]);
    }

    #[test]
    fn gitlab_api_and_failures() {
        let (base, server) = serve(1, |_| r#"[{"iid":3,"title":"Dark mode"}]"#.into());
        let t = tracker(TrackerKind::GitLab, format!("{base}/api/v4/projects/g%2Fr"));
        assert_eq!(Runtime::block_on(fetch_open_items(&t, "glpat")).unwrap()[0].id, 3);
        assert!(server.join().unwrap()[0].to_ascii_lowercase().contains("private-token: glpat"));
        // Nothing listening: an error, not a panic (the completion falls back to the history).
        let closed = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
        let t = tracker(TrackerKind::GitHub, format!("http://{closed}/repos/o/r"));
        assert!(Runtime::block_on(fetch_open_items(&t, "x")).is_err());
    }

    #[test]
    fn sync_builds_links_from_the_remote_and_history_from_the_log() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let git = |args: &[&str]| assert!(std::process::Command::new("git").args(args).current_dir(&root).output().unwrap().status.success(), "{args:?}");
        git(&["init", "-q", "-b", "main"]);
        git(&["remote", "add", "origin", "git@ssh.dev.azure.com:v3/contoso/Shop/Shop"]);
        let repo = Runtime::block_on(Repository::discover(&root)).unwrap();
        let commit = |subject: &str| Commit {
            hash: subject.into(),
            parents: vec![],
            author_name: String::new(),
            author_email: String::new(),
            author_time: 0,
            committer_name: String::new(),
            committer_email: String::new(),
            commit_time: 0,
            refs: vec![],
            head_branch: None,
            subject: subject.into(),
            body: String::new(),
        };
        let log = vec![commit("Export orders AB#1234"), commit("Fix login #56")];
        let mut w = WorkItems::new(Waker::noop());
        let patterns = vec![IssueLink { pattern: r"SHOP-(\d+)".into(), url: "https://jira.local/SHOP-$1".into() }];
        let start = std::time::Instant::now();
        while w.trackers().is_empty() {
            assert!(start.elapsed() < Duration::from_secs(20), "remotes not read");
            w.sync(Some(&repo), &patterns, &log);
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(w.find("AB#1234 SHOP-9").iter().map(|l| l.url.as_str()).collect::<Vec<_>>(), vec!["https://dev.azure.com/contoso/Shop/_workitems/edit/1234", "https://jira.local/SHOP-9"]);
        assert_eq!(w.items().iter().map(|i| i.id).collect::<Vec<_>>(), vec![1234, 56]);
        assert!(!w.online());
        // Changing the settings rebuilds the links.
        w.sync(Some(&repo), &[], &log);
        assert_eq!(w.find("SHOP-9").len(), 0);
        // Closing the repository forgets its tracker.
        w.sync(None, &[], &[]);
        assert!(w.find("AB#1").is_empty() && w.items().is_empty());
    }
}
