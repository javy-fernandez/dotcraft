//! Git integration: the repository of the workspace, its status, branches and history, the
//! gutter change marks of open files, commits and remote operations. Every git call runs on the
//! shared runtime; results come back through an inbox.

use crate::docs::Docs;
use rc_core::{Inbox, Runtime, Waker};
use rc_kit::i18n::{tr, trf};
use rc_vcs::{Blame, Branch, Commit, CommitOptions, DiffSpec, FileState, LineChangeKind, LineChanges, LogOptions, PullOptions, PushOptions, Repository, Revision, Status, StatusOptions};
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

enum VcsEvent {
    Repo(Option<Repository>),
    Status(Result<Status, String>),
    Branches(Vec<Branch>),
    Log(Vec<Commit>),
    Head(PathBuf, Option<String>),
    Blame(PathBuf, Result<Blame, String>),
    /// An operation finished: (what, error).
    Done(&'static str, Option<String>),
    CommitFiles(String, Vec<rc_vcs::ChangedFile>),
    /// Texts for the diff viewer: (title, path, left, right).
    Diff(String, PathBuf, String, String),
}

/// The changed files of the work tree, as shown by the Commit window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub path: String,
    pub state: FileState,
    pub untracked: bool,
}

/// A side-by-side diff to show in the editor area.
pub struct DiffView {
    pub title: String,
    pub path: PathBuf,
    pub left: rc_editor::EditorState,
    pub right: rc_editor::EditorState,
}

pub struct Vcs {
    pub repo: Option<Repository>,
    pub status: Option<Status>,
    pub branches: Vec<Branch>,
    pub log: Vec<Commit>,
    pub selected_commit: Option<String>,
    pub commit_files: Vec<rc_vcs::ChangedFile>,
    pub message: String,
    pub amend: bool,
    pub excluded: BTreeSet<String>,
    pub busy: Option<&'static str>,
    pub error: Option<String>,
    inbox: Inbox<VcsEvent>,
    waker: Waker,
    heads: HashMap<PathBuf, Option<String>>,
    head_pending: BTreeSet<PathBuf>,
    marks_version: HashMap<u64, u64>,
    status_requested: Option<Instant>,
    status_dirty: bool,
    pub blame_for: Option<PathBuf>,
    pub diff: Option<DiffView>,
    next_doc_id: u64,
}

impl Vcs {
    pub fn new(waker: Waker) -> Self {
        Vcs {
            repo: None,
            status: None,
            branches: Vec::new(),
            log: Vec::new(),
            selected_commit: None,
            commit_files: Vec::new(),
            message: String::new(),
            amend: false,
            excluded: BTreeSet::new(),
            busy: None,
            error: None,
            inbox: Inbox::new(waker.clone()),
            waker,
            heads: HashMap::new(),
            head_pending: BTreeSet::new(),
            marks_version: HashMap::new(),
            status_requested: None,
            status_dirty: false,
            blame_for: None,
            diff: None,
            next_doc_id: 1 << 40,
        }
    }

    pub fn open(&mut self, root: &Path) {
        *self = Vcs::new(self.waker.clone());
        let out = self.inbox.sender();
        let root = root.to_path_buf();
        Runtime::spawn(async move {
            out.send(VcsEvent::Repo(Repository::discover(&root).await.ok()));
        });
    }

    pub fn close(&mut self) {
        *self = Vcs::new(self.waker.clone());
    }

    pub fn branch_name(&self) -> Option<String> {
        self.status.as_ref().map(|s| s.branch.head.clone().unwrap_or_else(|| tr("Detached HEAD").to_string()))
    }

    /// The files of the work tree that changed (the Commit window's list).
    pub fn changes(&self) -> Vec<Change> {
        let Some(st) = &self.status else { return Vec::new() };
        st.entries
            .iter()
            .filter(|e| e.worktree != FileState::Ignored)
            .map(|e| {
                let untracked = e.worktree == FileState::Untracked;
                let state = if e.worktree != FileState::Unmodified { e.worktree } else { e.index };
                Change { path: e.path.clone(), state, untracked }
            })
            .collect()
    }

    /// A file changed on disk: refresh the status soon and forget its HEAD text when it is
    /// git's own state that changed.
    pub fn on_fs_change(&mut self, path: &Path) {
        if self.repo.is_none() {
            return;
        }
        let s = path.to_string_lossy();
        if s.contains("/.git/") {
            if s.ends_with("HEAD") || s.contains("/refs/") || s.ends_with("index") {
                self.heads.clear();
                self.marks_version.clear();
            }
        }
        self.status_dirty = true;
    }

    pub fn refresh(&mut self) {
        self.status_dirty = true;
        self.status_requested = None;
        self.heads.clear();
        self.marks_version.clear();
        self.load_log();
    }

    fn request_status(&mut self) {
        let Some(repo) = self.repo.clone() else { return };
        self.status_requested = Some(Instant::now());
        self.status_dirty = false;
        let out = self.inbox.sender();
        Runtime::spawn(async move {
            let st = repo.status(&StatusOptions::default()).await.map_err(|e| e.to_string());
            out.send(VcsEvent::Status(st));
            if let Ok(b) = repo.branches().await {
                out.send(VcsEvent::Branches(b));
            }
        });
    }

    pub fn load_log(&mut self) {
        let Some(repo) = self.repo.clone() else { return };
        let out = self.inbox.sender();
        Runtime::spawn(async move {
            let opts = LogOptions { all: true, max_count: 500, ..Default::default() };
            if let Ok(log) = repo.log(&opts).await {
                out.send(VcsEvent::Log(log));
            }
        });
    }

    pub fn select_commit(&mut self, hash: &str) {
        self.selected_commit = Some(hash.to_string());
        self.commit_files.clear();
        let Some(repo) = self.repo.clone() else { return };
        let out = self.inbox.sender();
        let h = hash.to_string();
        Runtime::spawn(async move {
            if let Ok(files) = repo.commit_files(&h).await {
                out.send(VcsEvent::CommitFiles(h, files));
            }
        });
    }

    // ------------------------------------------------------------------ operations

    fn op<F>(&mut self, what: &'static str, f: F)
    where
        F: std::future::Future<Output = Result<(), rc_vcs::GitError>> + Send + 'static,
    {
        self.busy = Some(what);
        self.error = None;
        let out = self.inbox.sender();
        Runtime::spawn(async move {
            let r = f.await;
            out.send(VcsEvent::Done(what, r.err().map(|e| {
                let (tpl, args) = e.user_message();
                let args: Vec<(&str, &dyn std::fmt::Display)> = args.iter().map(|(k, v)| (*k, v as &dyn std::fmt::Display)).collect();
                trf(tpl, &args)
            })));
        });
    }

    /// Commits the included changes (and pushes when `push`).
    pub fn commit(&mut self, push: bool) {
        let Some(repo) = self.repo.clone() else { return };
        let only: Vec<String> = self.changes().into_iter().map(|c| c.path).filter(|p| !self.excluded.contains(p)).collect();
        if only.is_empty() && !self.amend {
            self.error = Some(tr("Select the files to commit").into());
            return;
        }
        if self.message.trim().is_empty() {
            self.error = Some(tr("Write a commit message").into());
            return;
        }
        let opts = CommitOptions { message: self.message.trim().to_string(), amend: self.amend, only, ..Default::default() };
        self.op(if push { "commit_push" } else { "commit" }, async move {
            repo.commit(&opts).await?;
            if push {
                repo.push(&PushOptions::default(), None, None).await?;
            }
            Ok(())
        });
    }

    pub fn push(&mut self) {
        let Some(repo) = self.repo.clone() else { return };
        let upstream = self.status.as_ref().and_then(|s| s.branch.upstream.clone());
        let branch = self.status.as_ref().and_then(|s| s.branch.head.clone());
        self.op("push", async move {
            let opts = if upstream.is_none() { PushOptions { refspec: branch, set_upstream: true, ..Default::default() } } else { PushOptions::default() };
            repo.push(&opts, None, None).await
        });
    }

    pub fn pull(&mut self) {
        let Some(repo) = self.repo.clone() else { return };
        self.op("pull", async move { repo.pull(&PullOptions::default(), None, None).await });
    }

    pub fn fetch(&mut self) {
        let Some(repo) = self.repo.clone() else { return };
        self.op("fetch", async move { repo.fetch(&rc_vcs::FetchOptions { prune: true, ..Default::default() }, None, None).await });
    }

    pub fn checkout(&mut self, branch: &Branch) {
        let Some(repo) = self.repo.clone() else { return };
        let b = branch.clone();
        self.op("checkout", async move {
            if b.kind == rc_vcs::BranchKind::Remote {
                let local = b.name.split_once('/').map_or(b.name.clone(), |(_, n)| n.to_string());
                repo.checkout_remote_branch(&b.name, Some(&local)).await
            } else {
                repo.checkout_branch(&b.name).await
            }
        });
    }

    pub fn new_branch(&mut self, name: &str) {
        let Some(repo) = self.repo.clone() else { return };
        let n = name.to_string();
        self.op("branch", async move { repo.create_branch(&n, None, true).await });
    }

    /// Rollback: restores files from HEAD (new files are only unstaged).
    pub fn rollback(&mut self, paths: Vec<String>) {
        let Some(repo) = self.repo.clone() else { return };
        self.op("rollback", async move { repo.rollback(&paths).await });
    }

    // ------------------------------------------------------------------ diffs and blame

    /// Opens the diff of a working-tree file against HEAD.
    pub fn open_diff(&mut self, rel: &str) {
        let Some(repo) = self.repo.clone() else { return };
        let out = self.inbox.sender();
        let rel = rel.to_string();
        Runtime::spawn(async move {
            let abs = repo.abs_path(&rel);
            let left = repo.show_file(&Revision::Head, &rel).await.ok().flatten().map(|c| c.text().into_owned()).unwrap_or_default();
            let right = std::fs::read_to_string(&abs).unwrap_or_default();
            out.send(VcsEvent::Diff(format!("{rel} ({} ↔ {})", tr("HEAD"), tr("Working tree")), abs, left, right));
        });
    }

    /// Opens the diff of a file in a commit (parent ↔ commit).
    pub fn open_commit_diff(&mut self, hash: &str, rel: &str) {
        let Some(repo) = self.repo.clone() else { return };
        let out = self.inbox.sender();
        let (h, rel) = (hash.to_string(), rel.to_string());
        Runtime::spawn(async move {
            let left = repo.show_file(&Revision::Commit(format!("{h}^")), &rel).await.ok().flatten().map(|c| c.text().into_owned()).unwrap_or_default();
            let right = repo.show_file(&Revision::Commit(h.clone()), &rel).await.ok().flatten().map(|c| c.text().into_owned()).unwrap_or_default();
            let short = &h[..h.len().min(8)];
            out.send(VcsEvent::Diff(format!("{rel} ({short})"), repo.abs_path(&rel), left, right));
        });
    }

    pub fn toggle_blame(&mut self, path: &Path, text: String, docs: &mut Docs) {
        if self.blame_for.as_deref() == Some(path) {
            self.blame_for = None;
            if let Some(i) = docs.index_of(path) {
                docs.list[i].editor.annotations = None;
            }
            return;
        }
        let Some(repo) = self.repo.clone() else { return };
        let Some(rel) = repo.rel_path(path) else { return };
        self.blame_for = Some(path.to_path_buf());
        let out = self.inbox.sender();
        let p = path.to_path_buf();
        Runtime::spawn(async move {
            let b = repo.blame(&rel, Some(&text), None, None).await.map_err(|e| e.to_string());
            out.send(VcsEvent::Blame(p, b));
        });
    }

    // ------------------------------------------------------------------ per frame

    pub fn update(&mut self, docs: &mut Docs, dark: bool) -> Vec<String> {
        let mut notices = Vec::new();
        for ev in self.inbox.drain() {
            match ev {
                VcsEvent::Repo(r) => {
                    self.repo = r;
                    if self.repo.is_some() {
                        self.request_status();
                        self.load_log();
                    }
                }
                VcsEvent::Status(Ok(s)) => {
                    // Files that left the change list lose their exclusion.
                    let paths: BTreeSet<String> = s.entries.iter().map(|e| e.path.clone()).collect();
                    self.excluded.retain(|p| paths.contains(p));
                    self.status = Some(s);
                }
                VcsEvent::Status(Err(e)) => self.error = Some(e),
                VcsEvent::Branches(b) => self.branches = b,
                VcsEvent::Log(l) => self.log = l,
                VcsEvent::Head(p, t) => {
                    self.head_pending.remove(&p);
                    self.heads.insert(p, t);
                    self.marks_version.clear();
                }
                VcsEvent::Blame(p, Ok(b)) => {
                    if self.blame_for.as_deref() == Some(p.as_path())
                        && let Some(i) = docs.index_of(&p)
                    {
                        docs.list[i].editor.annotations = Some(
                            b.lines
                                .iter()
                                .map(|l| {
                                    let c = &b.commits[l.commit];
                                    if c.is_uncommitted() { tr("Not committed yet").to_string() } else { format!("{}  {}", format_date(c.author_time), short_author(&c.author)) }
                                })
                                .collect(),
                        );
                    }
                }
                VcsEvent::Blame(_, Err(e)) => {
                    self.blame_for = None;
                    notices.push(e);
                }
                VcsEvent::Done(what, err) => {
                    self.busy = None;
                    match err {
                        Some(e) => {
                            self.error = Some(e.clone());
                            notices.push(e);
                        }
                        None => {
                            if what == "commit" || what == "commit_push" {
                                self.message.clear();
                                self.amend = false;
                                notices.push(if what == "commit" { tr("Committed").to_string() } else { tr("Committed and pushed").to_string() });
                            } else if what == "push" {
                                notices.push(tr("Pushed").to_string());
                            } else if what == "pull" {
                                notices.push(tr("Project updated").to_string());
                            }
                            self.refresh();
                        }
                    }
                }
                VcsEvent::CommitFiles(h, f) => {
                    if self.selected_commit.as_deref() == Some(h.as_str()) {
                        self.commit_files = f;
                    }
                }
                VcsEvent::Diff(title, path, left, right) => self.diff = Some(self.make_diff(title, path, &left, &right, dark)),
            }
        }
        if self.repo.is_some() && self.status_dirty && self.status_requested.is_none_or(|t| t.elapsed() > Duration::from_millis(700)) {
            self.request_status();
        }
        self.update_marks(docs);
        notices
    }

    /// Gutter marks of the active document against its HEAD version.
    fn update_marks(&mut self, docs: &mut Docs) {
        let Some(repo) = self.repo.clone() else { return };
        let Some(i) = docs.active else { return };
        let doc = &mut docs.list[i];
        let path = doc.path.clone();
        if !self.heads.contains_key(&path) {
            if self.head_pending.insert(path.clone()) {
                let out = self.inbox.sender();
                Runtime::spawn(async move {
                    let text = match repo.rel_path(&path) {
                        Some(rel) => repo.show_file(&Revision::Head, &rel).await.ok().flatten().map(|c| c.text().into_owned()),
                        None => None,
                    };
                    out.send(VcsEvent::Head(path, text));
                });
            }
            return;
        }
        let version = doc.editor.buffer.version();
        if self.marks_version.get(&doc.editor.id) == Some(&version) {
            return;
        }
        self.marks_version.insert(doc.editor.id, version);
        doc.editor.line_changes.clear();
        let Some(Some(head)) = self.heads.get(&path) else {
            // Not in HEAD: a new file is all added (when it is inside the repository).
            if repo.rel_path(&path).is_some() && self.heads.get(&path).is_some_and(|h| h.is_none()) && !self.is_ignored(&path) {
                for l in 0..doc.editor.buffer.len_lines() {
                    doc.editor.line_changes.insert(l, rc_editor::LineChange::Added);
                }
            }
            return;
        };
        let changes = LineChanges::compute(head, &doc.editor.buffer.rope().to_string());
        for c in &changes.changes {
            match c.kind {
                LineChangeKind::Added => c.new.clone().for_each(|l| {
                    doc.editor.line_changes.insert(l, rc_editor::LineChange::Added);
                }),
                LineChangeKind::Modified => c.new.clone().for_each(|l| {
                    doc.editor.line_changes.insert(l, rc_editor::LineChange::Modified);
                }),
                LineChangeKind::Deleted => {
                    doc.editor.line_changes.insert(c.new.start.saturating_sub(1), rc_editor::LineChange::Deleted);
                }
            }
        }
    }

    fn is_ignored(&self, path: &Path) -> bool {
        let Some(repo) = &self.repo else { return true };
        let Some(rel) = repo.rel_path(path) else { return true };
        // Untracked files show in the status; files missing from it are clean or ignored.
        !self.status.as_ref().is_some_and(|s| s.entries.iter().any(|e| e.path == rel))
    }

    fn make_diff(&mut self, title: String, path: PathBuf, left: &str, right: &str, dark: bool) -> DiffView {
        let sbs = rc_vcs::SideBySide::compute(left, right);
        let ll: Vec<&str> = left.split('\n').collect();
        let rl: Vec<&str> = right.split('\n').collect();
        let (mut lt, mut rt) = (Vec::new(), Vec::new());
        let s = rc_kit::theme::scheme(dark);
        let removed = s.deleted_lines.gamma_multiply(0.22);
        let added = s.added_lines.gamma_multiply(if dark { 0.55 } else { 0.6 });
        let modified = s.modified_lines.gamma_multiply(if dark { 0.55 } else { 0.6 });
        let filler = rc_kit::theme::palette(dark).header_bg;
        let mut lbg = std::collections::BTreeMap::new();
        let left_numbers: Vec<Option<usize>> = sbs.rows.iter().map(|r| r.left).collect();
        let right_numbers: Vec<Option<usize>> = sbs.rows.iter().map(|r| r.right).collect();
        let mut rbg = std::collections::BTreeMap::new();
        for (row, r) in sbs.rows.iter().enumerate() {
            lt.push(r.left.and_then(|i| ll.get(i)).map(|s| s.trim_end_matches('\r')).unwrap_or(""));
            rt.push(r.right.and_then(|i| rl.get(i)).map(|s| s.trim_end_matches('\r')).unwrap_or(""));
            match r.kind {
                rc_vcs::RowKind::Equal => {}
                rc_vcs::RowKind::Added => {
                    lbg.insert(row, filler);
                    rbg.insert(row, added);
                }
                rc_vcs::RowKind::Deleted => {
                    lbg.insert(row, removed);
                    rbg.insert(row, filler);
                }
                rc_vcs::RowKind::Modified => {
                    lbg.insert(row, modified);
                    rbg.insert(row, modified);
                }
            }
        }
        let mut make = |text: String, bg, numbers| {
            self.next_doc_id += 1;
            let mut e = rc_editor::EditorState::new(self.next_doc_id, Some(path.clone()), &text);
            e.read_only = true;
            e.line_backgrounds = bg;
            e.display_numbers = Some(numbers);
            e.parse_blocking();
            e
        };
        DiffView { title, path: path.clone(), left: make(lt.join("\n"), lbg, left_numbers), right: make(rt.join("\n"), rbg, right_numbers) }
    }

    pub fn has_changes(&self) -> bool {
        self.status.as_ref().is_some_and(|s| !s.entries.is_empty())
    }

    pub fn diff_spec_worktree() -> DiffSpec {
        DiffSpec::WorktreeVsHead
    }
}

fn short_author(a: &str) -> String {
    a.split_whitespace().next().unwrap_or(a).to_string()
}

/// `2026-10-24` from Unix seconds (UTC).
pub fn format_date(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    // Civil from days (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    #[test]
    fn dates() {
        assert_eq!(super::format_date(0), "1970-01-01");
        assert_eq!(super::format_date(1_700_000_000), "2023-11-14");
    }
}
