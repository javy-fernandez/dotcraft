//! Pull requests of the workspace's repository on GitHub or Azure DevOps: the service is
//! detected from the remotes, the personal access token comes from (and goes to) the OS
//! keychain, and every REST call runs on the shared runtime, reporting back through an inbox.

use rc_core::{Inbox, Runtime, Waker};
use rc_kit::i18n::{tr, trf};
use rc_vcs::Repository;
use rc_vcs::hosting::secrets::{Keychain, MemoryStore, SecretStore};
use rc_vcs::hosting::{HostClient, HostError, HostKind, HostRepo, MergeMethod, NewPullRequest, PrDetails, PrFile, PullRequest, Review, Thread, Viewer};
use std::path::PathBuf;
use std::sync::Arc;

/// What the app does with a result.
pub enum Output {
    Notice(String, bool),
    /// A file diff for the diff viewer: (title, repository-relative path, left, right).
    Diff(String, String, String, String),
}

enum Event {
    Detected(PathBuf, Option<HostRepo>),
    /// The token was checked: (result, token, whether to store it).
    Connected(Result<Viewer, HostError>, String, bool),
    /// A list reply; the number is the request it answers (older replies are dropped).
    List(u64, Result<Vec<PullRequest>, HostError>),
    Details(u64, Result<PrDetails, HostError>),
    Diff(String, String, Result<(String, String), HostError>),
    /// An action finished: (what, result with the notice, pull request to reload).
    Done(&'static str, Result<String, HostError>, Option<u64>),
    Created(Result<PullRequest, HostError>),
    CliToken(Option<String>),
}

/// The New Pull Request dialog.
#[derive(Clone, Debug, Default)]
pub struct CreateForm {
    pub title: String,
    pub description: String,
    pub source: String,
    pub target: String,
    pub draft: bool,
    /// Local branches (sources) and target branches on the remote.
    pub sources: Vec<String>,
    pub targets: Vec<String>,
    /// The source branch is not on the remote yet.
    pub unpushed: bool,
    pub busy: bool,
    pub error: Option<String>,
}

pub struct PullRequests {
    /// The repository on its hosting service (None: no GitHub / Azure DevOps remote).
    pub repo: Option<HostRepo>,
    detected_root: Option<PathBuf>,
    /// The repository was set by a test, not detected from the workspace.
    pinned: bool,
    pub detecting: bool,
    client: Option<HostClient>,
    pub viewer: Option<Viewer>,
    pub connecting: bool,
    pub token_input: String,
    pub list: Vec<PullRequest>,
    pub loading: bool,
    /// Id of the latest list request: a slower, older reply must not undo a newer selection.
    list_request: u64,
    pub error: Option<String>,
    pub selected: Option<u64>,
    pub details: Option<PrDetails>,
    pub details_loading: bool,
    pub filter: String,
    pub new_comment: String,
    /// The conversation being answered and the reply text.
    pub reply: Option<(String, String)>,
    pub review_body: String,
    pub merge_method: MergeMethod,
    pub confirm_merge: bool,
    pub create: Option<CreateForm>,
    pub busy: Option<&'static str>,
    /// Description, Files, Conversations.
    pub open: [bool; 3],
    store: Arc<dyn SecretStore>,
    inbox: Inbox<Event>,
    waker: Waker,
}

fn error_text(e: &HostError) -> String {
    match e {
        HostError::Unauthorized(_) => tr("The token was refused. Sign in again with a valid personal access token.").to_string(),
        HostError::Forbidden(m) => trf("Not allowed: {reason}", &[("reason", m)]),
        HostError::Network(m) => trf("Network error: {reason}", &[("reason", m)]),
        e => e.to_string(),
    }
}

impl PullRequests {
    pub fn new(waker: Waker) -> Self {
        // Tests never touch the user's keychain.
        let store: Arc<dyn SecretStore> = if cfg!(test) { Arc::new(MemoryStore::default()) } else { Arc::new(Keychain) };
        Self::with_store(waker, store)
    }

    pub fn with_store(waker: Waker, store: Arc<dyn SecretStore>) -> Self {
        PullRequests {
            repo: None,
            detected_root: None,
            pinned: false,
            detecting: false,
            client: None,
            viewer: None,
            connecting: false,
            token_input: String::new(),
            list: Vec::new(),
            loading: false,
            list_request: 0,
            error: None,
            selected: None,
            details: None,
            details_loading: false,
            filter: String::new(),
            new_comment: String::new(),
            reply: None,
            review_body: String::new(),
            merge_method: MergeMethod::default(),
            confirm_merge: false,
            create: None,
            busy: None,
            open: [true, true, true],
            store,
            inbox: Inbox::new(waker.clone()),
            waker,
        }
    }

    fn reset(&mut self) {
        *self = Self::with_store(self.waker.clone(), self.store.clone());
    }

    pub fn signed_in(&self) -> bool {
        self.client.is_some() && self.viewer.is_some()
    }

    /// Follows the workspace's repository: detects its service when it changes.
    pub fn poll(&mut self, repo: Option<&Repository>) -> Vec<Output> {
        let root = repo.map(|r| r.root().to_path_buf());
        if !self.pinned && root != self.detected_root {
            self.reset();
            self.detected_root = root.clone();
            if let (Some(repo), Some(root)) = (repo.cloned(), root) {
                self.detecting = true;
                let out = self.inbox.sender();
                Runtime::spawn(async move {
                    let remotes = repo.remotes().await.unwrap_or_default();
                    // `origin` first, then the others in order.
                    let mut ordered: Vec<&rc_vcs::Remote> = remotes.iter().filter(|r| r.name == "origin").collect();
                    ordered.extend(remotes.iter().filter(|r| r.name != "origin"));
                    let found = ordered.iter().find_map(|r| HostRepo::parse(&r.fetch_url).or_else(|| HostRepo::parse(&r.push_url)));
                    out.send(Event::Detected(root, found));
                });
            }
        }
        self.drain()
    }

    fn drain(&mut self) -> Vec<Output> {
        let mut outs = Vec::new();
        for ev in self.inbox.drain() {
            match ev {
                Event::Detected(root, found) => {
                    if Some(&root) != self.detected_root.as_ref() {
                        continue;
                    }
                    self.detecting = false;
                    self.set_repo(found);
                }
                Event::Connected(r, token, save) => {
                    self.connecting = false;
                    match r {
                        Ok(viewer) => {
                            if save
                                && let Some(repo) = &self.repo
                                && let Err(e) = self.store.set(&repo.account(), &token)
                            {
                                outs.push(Output::Notice(trf("Could not save the token in the keychain: {reason}", &[("reason", &e)]), true));
                            }
                            self.token_input.clear();
                            self.error = None;
                            self.viewer = Some(viewer);
                            self.refresh();
                        }
                        Err(e) => {
                            self.client = None;
                            self.error = Some(error_text(&e));
                        }
                    }
                }
                Event::List(request, _) if request != self.list_request => {}
                Event::List(_, r) => {
                    self.loading = false;
                    match r {
                        Ok(list) => {
                            self.list = list;
                            if self.selected.is_some_and(|id| !self.list.iter().any(|p| p.id == id)) && self.details.as_ref().is_none_or(|d| !d.pr.is_open()) {
                                self.selected = None;
                                self.details = None;
                            }
                        }
                        Err(e) => self.error = Some(error_text(&e)),
                    }
                }
                Event::Details(id, r) => {
                    if self.selected != Some(id) {
                        continue;
                    }
                    self.details_loading = false;
                    match r {
                        Ok(d) => self.details = Some(d),
                        Err(e) => self.error = Some(error_text(&e)),
                    }
                }
                Event::Diff(title, path, r) => match r {
                    Ok((left, right)) => outs.push(Output::Diff(title, path, left, right)),
                    Err(e) => outs.push(Output::Notice(error_text(&e), true)),
                },
                Event::Done(what, r, reload) => {
                    self.busy = None;
                    match r {
                        Ok(notice) => {
                            match what {
                                "comment" => self.new_comment.clear(),
                                "reply" => self.reply = None,
                                "review" => self.review_body.clear(),
                                _ => {}
                            }
                            if !notice.is_empty() {
                                outs.push(Output::Notice(notice, false));
                            }
                            if what == "merge" {
                                self.refresh();
                            }
                            if let Some(id) = reload {
                                self.load_details(id);
                            }
                        }
                        Err(e) => outs.push(Output::Notice(error_text(&e), true)),
                    }
                }
                Event::Created(r) => match r {
                    Ok(pr) => {
                        self.create = None;
                        outs.push(Output::Notice(trf("Created pull request #{id}", &[("id", &pr.id)]), false));
                        let id = pr.id;
                        self.list.insert(0, pr);
                        self.select(id);
                        self.refresh();
                    }
                    Err(e) => {
                        if let Some(f) = &mut self.create {
                            f.busy = false;
                            f.error = Some(error_text(&e));
                        }
                    }
                },
                Event::CliToken(token) => match token {
                    Some(t) => self.connect(t, true),
                    None => {
                        self.connecting = false;
                        self.error = Some(tr("The GitHub CLI is not installed or not signed in (gh auth login).").to_string());
                    }
                },
            }
        }
        outs
    }

    /// Sets the detected repository and connects with the stored token, if any.
    pub fn set_repo(&mut self, repo: Option<HostRepo>) {
        self.repo = repo;
        self.client = None;
        self.viewer = None;
        if let Some(r) = &self.repo
            && let Some(token) = self.store.get(&r.account())
        {
            self.connect(token, false);
        }
    }

    fn connect(&mut self, token: String, save: bool) {
        let Some(repo) = self.repo.clone() else { return };
        let client = match HostClient::new(repo, &token) {
            Ok(c) => c,
            Err(e) => {
                self.error = Some(error_text(&e));
                return;
            }
        };
        self.client = Some(client.clone());
        self.connecting = true;
        self.error = None;
        let out = self.inbox.sender();
        Runtime::spawn(async move {
            out.send(Event::Connected(client.viewer().await, token, save));
        });
    }

    /// Checks the token typed in the sign-in form and keeps it in the keychain.
    pub fn sign_in(&mut self) {
        let token = self.token_input.trim().to_string();
        if !token.is_empty() {
            self.connect(token, true);
        }
    }

    /// Signs in with the GitHub CLI's token (`gh auth token`).
    pub fn sign_in_with_cli(&mut self) {
        let Some(repo) = self.repo.clone().filter(|r| r.kind == HostKind::GitHub) else { return };
        self.connecting = true;
        self.error = None;
        let out = self.inbox.sender();
        Runtime::spawn(async move {
            out.send(Event::CliToken(rc_vcs::hosting::gh_cli_token(&repo.host).await));
        });
    }

    /// Forgets the token (keychain included).
    pub fn sign_out(&mut self) -> Result<(), String> {
        let repo = self.repo.clone();
        let (root, pinned) = (self.detected_root.clone(), self.pinned);
        let r = repo.as_ref().map(|r| self.store.delete(&r.account())).unwrap_or(Ok(()));
        self.reset();
        self.detected_root = root;
        self.pinned = pinned;
        self.repo = repo;
        r
    }

    pub fn refresh(&mut self) {
        let Some(client) = self.client.clone() else { return };
        self.loading = true;
        self.list_request += 1;
        let request = self.list_request;
        let out = self.inbox.sender();
        Runtime::spawn(async move {
            out.send(Event::List(request, client.list().await));
        });
        if let Some(id) = self.selected {
            self.load_details(id);
        }
    }

    pub fn select(&mut self, id: u64) {
        if self.selected != Some(id) {
            self.selected = Some(id);
            self.details = None;
            self.reply = None;
            self.confirm_merge = false;
            self.review_body.clear();
            self.new_comment.clear();
        }
        self.load_details(id);
    }

    fn load_details(&mut self, id: u64) {
        let (Some(client), Some(viewer)) = (self.client.clone(), self.viewer.clone()) else { return };
        self.details_loading = true;
        let out = self.inbox.sender();
        Runtime::spawn(async move {
            out.send(Event::Details(id, client.details(id, &viewer).await));
        });
    }

    /// Opens a changed file in the diff viewer (merge base ↔ source branch).
    pub fn open_diff(&mut self, file: &PrFile) {
        let (Some(client), Some(d)) = (self.client.clone(), self.details.as_ref()) else { return };
        let (base, head) = (d.base_commit.clone(), d.head_commit.clone());
        let file = file.clone();
        let title = format!("{} (#{})", file.path, d.pr.id);
        let out = self.inbox.sender();
        Runtime::spawn(async move {
            let old = file.old_path.clone().unwrap_or_else(|| file.path.clone());
            let left = if file.change == rc_vcs::hosting::FileChange::Added { Ok(None) } else { client.file_text(&old, &base).await };
            let right = if file.change == rc_vcs::hosting::FileChange::Deleted { Ok(None) } else { client.file_text(&file.path, &head).await };
            let r = left.and_then(|l| right.map(|r| (l.unwrap_or_default(), r.unwrap_or_default())));
            out.send(Event::Diff(title, file.path, r));
        });
    }

    fn action<F>(&mut self, what: &'static str, reload: Option<u64>, f: F)
    where
        F: std::future::Future<Output = Result<String, HostError>> + Send + 'static,
    {
        self.busy = Some(what);
        let out = self.inbox.sender();
        Runtime::spawn(async move {
            out.send(Event::Done(what, f.await, reload));
        });
    }

    fn current(&self) -> Option<(HostClient, PullRequest)> {
        Some((self.client.clone()?, self.details.as_ref()?.pr.clone()))
    }

    pub fn add_comment(&mut self) {
        let Some((client, pr)) = self.current() else { return };
        let body = self.new_comment.trim().to_string();
        if body.is_empty() {
            return;
        }
        self.action("comment", Some(pr.id), async move { client.comment(pr.id, &body).await.map(|_| String::new()) });
    }

    pub fn send_reply(&mut self) {
        let Some((client, pr)) = self.current() else { return };
        let Some((thread_id, text)) = self.reply.clone() else { return };
        let Some(thread) = self.details.as_ref().and_then(|d| d.threads.iter().find(|t| t.id == thread_id)).cloned() else { return };
        let body = text.trim().to_string();
        if body.is_empty() {
            return;
        }
        self.action("reply", Some(pr.id), async move { reply(&client, pr.id, &thread, &body).await });
    }

    pub fn review(&mut self, review: Review) {
        let (Some((client, pr)), Some(viewer)) = (self.current(), self.viewer.clone()) else { return };
        let body = self.review_body.trim().to_string();
        let notice = match review {
            Review::Approve => trf("Approved #{id}", &[("id", &pr.id)]),
            Review::RequestChanges => trf("Requested changes on #{id}", &[("id", &pr.id)]),
        };
        self.action("review", Some(pr.id), async move { client.review(pr.id, review, &body, &viewer).await.map(|_| notice) });
    }

    pub fn merge(&mut self) {
        let Some((client, pr)) = self.current() else { return };
        self.confirm_merge = false;
        let method = self.merge_method;
        let notice = trf("Merged #{id}", &[("id", &pr.id)]);
        self.action("merge", Some(pr.id), async move { client.merge(&pr, method).await.map(|_| notice) });
    }

    /// Opens the New Pull Request dialog for `current` (the checked-out branch).
    pub fn open_create(&mut self, current: Option<String>, branches: &[rc_vcs::Branch]) {
        let local: Vec<String> = branches.iter().filter(|b| b.kind == rc_vcs::BranchKind::Local).map(|b| b.name.clone()).collect();
        let mut targets: Vec<String> = branches.iter().filter_map(|b| b.remote_parts()).filter(|(r, n)| *r == "origin" && *n != "HEAD").map(|(_, n)| n.to_string()).collect();
        let default = self.viewer.as_ref().map(|v| v.default_branch.clone()).filter(|b| !b.is_empty()).unwrap_or_else(|| "main".into());
        targets.retain(|t| *t != default);
        targets.insert(0, default.clone());
        let source = current.unwrap_or_default();
        let title = source.rsplit('/').next().unwrap_or(&source).replace(['-', '_'], " ");
        let mut title_chars = title.chars();
        let title = title_chars.next().map(|c| c.to_uppercase().collect::<String>() + title_chars.as_str()).unwrap_or_default();
        let mut form = CreateForm { title, source, target: default, sources: local, targets, ..Default::default() };
        form.unpushed = !is_pushed(&form.source, branches);
        self.create = Some(form);
    }

    /// Updates whether the dialog's source branch is on the remote.
    pub fn check_pushed(&mut self, branches: &[rc_vcs::Branch]) {
        if let Some(f) = &mut self.create {
            f.unpushed = !is_pushed(&f.source, branches);
        }
    }

    pub fn create_pr(&mut self) {
        let Some(client) = self.client.clone() else { return };
        let Some(f) = &mut self.create else { return };
        f.busy = true;
        f.error = None;
        let new = NewPullRequest { title: f.title.trim().to_string(), description: f.description.trim().to_string(), source: f.source.clone(), target: f.target.clone(), draft: f.draft };
        let out = self.inbox.sender();
        Runtime::spawn(async move {
            out.send(Event::Created(client.create(&new).await));
        });
    }

    /// The pull requests matching the filter (number, title, author or branch).
    pub fn visible(&self) -> Vec<&PullRequest> {
        let f = self.filter.trim().to_lowercase();
        let f = f.trim_start_matches('#');
        self.list.iter().filter(|p| f.is_empty() || p.id.to_string().starts_with(f) || [&p.title, &p.author, &p.source_branch].iter().any(|s| s.to_lowercase().contains(f))).collect()
    }

    /// Uses `repo` as if detected from the workspace (tests against a mock server).
    #[cfg(test)]
    pub fn use_repo_for_test(&mut self, root: PathBuf, repo: HostRepo) {
        self.detected_root = Some(root);
        self.pinned = true;
        self.set_repo(Some(repo));
    }
}

async fn reply(client: &HostClient, id: u64, thread: &Thread, body: &str) -> Result<String, HostError> {
    client.reply(id, thread, body).await.map(|_| String::new())
}

pub fn is_pushed(branch: &str, branches: &[rc_vcs::Branch]) -> bool {
    branches.iter().any(|b| b.remote_parts().is_some_and(|(r, n)| r == "origin" && n == branch)) || branches.iter().any(|b| b.name == branch && b.upstream.is_some() && !b.upstream_gone)
}

#[cfg(test)]
mod tests;
