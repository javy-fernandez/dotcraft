//! Git integration: the repository of the workspace, its status, branches and history, the
//! gutter change marks of open files, commits and remote operations. Every git call runs on the
//! shared runtime; results come back through an inbox.

use crate::docs::Docs;
use rc_core::{Inbox, Runtime, Waker};
use rc_kit::i18n::{tr, trf};
use rc_vcs::{Blame, Branch, Commit, CommitOptions, FileState, LineChangeKind, LineChanges, LogOptions, PullOptions, PushOptions, Repository, Revision, Status, StatusOptions};
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[path = "git_tools.rs"]
pub mod tools;

enum VcsEvent {
    ToolReport(String, Option<PathBuf>),
    Repo(Option<Repository>),
    Status(Result<Status, String>),
    Branches(Vec<Branch>),
    Stashes(Result<Vec<rc_vcs::Stash>, String>),
    Log(Vec<Commit>),
    Head(PathBuf, Option<String>),
    Blame(PathBuf, Result<Blame, String>),
    /// An operation finished: (what, error).
    Done(&'static str, Option<String>),
    CommitFiles(String, Vec<rc_vcs::ChangedFile>),
    /// Texts for the diff viewer: (title, path, left, right).
    Diff(String, PathBuf, String, String),
    State(rc_vcs::RepoState),
    /// A branch delete was refused because the branch is not fully merged.
    Unmerged(String),
    Compare(String, Result<(Vec<Commit>, Vec<Commit>), String>),
    /// The rebase in progress (none when the repository is not rebasing).
    Rebase(Option<rc_vcs::RebaseStatus>),
    /// The plan of an interactive rebase from a commit.
    RebasePlan(String, Result<rc_vcs::RebasePlan, String>),
    /// A rebase step stopped (conflicts, edit…): replaces the success notice.
    RebaseStopped(rc_vcs::RebaseStatus),
}

/// The commits that differ between a branch and the current one.
#[derive(Clone, Debug, Default)]
pub struct BranchCompare {
    pub branch: String,
    /// In `branch` but not in the current branch.
    pub incoming: Vec<Commit>,
    /// In the current branch but not in `branch`.
    pub outgoing: Vec<Commit>,
    pub loading: bool,
    pub error: Option<String>,
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
    pub tools: tools::RepositoryTools,
    pub repo: Option<Repository>,
    pub status: Option<Status>,
    pub branches: Vec<Branch>,
    pub stashes: Vec<rc_vcs::Stash>,
    pub show_stashes: bool,
    pub stash_message: String,
    pub stash_include_untracked: bool,
    pub stash_keep_index: bool,
    pub stash_restore_index: bool,
    /// The stash awaiting confirmation and whether the action is drop (otherwise pop).
    pub stash_confirmation: Option<(rc_vcs::Stash, bool)>,
    pub log: Vec<Commit>,
    pub selected_commit: Option<String>,
    pub commit_files: Vec<rc_vcs::ChangedFile>,
    pub message: String,
    pub amend: bool,
    pub excluded: BTreeSet<String>,
    pub busy: Option<&'static str>,
    operation: Option<tokio::task::AbortHandle>,
    /// Shown as a notification when the running operation succeeds.
    success: Option<String>,
    /// Merge, rebase… in progress.
    pub state: rc_vcs::RepoState,
    /// A local branch whose delete needs a second, forced confirmation.
    pub unmerged: Option<String>,
    pub compare: Option<BranchCompare>,
    /// The rebase in progress, while [`Vcs::state`] is rebasing.
    pub rebase: Option<rc_vcs::RebaseStatus>,
    /// The loaded plan of the interactive rebase dialog: (start commit, plan or error).
    pub rebase_plan: Option<(String, Result<rc_vcs::RebasePlan, String>)>,
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
            tools: Default::default(),
            repo: None,
            status: None,
            branches: Vec::new(),
            stashes: Vec::new(),
            show_stashes: false,
            stash_message: String::new(),
            stash_include_untracked: false,
            stash_keep_index: false,
            stash_restore_index: false,
            stash_confirmation: None,
            log: Vec::new(),
            selected_commit: None,
            commit_files: Vec::new(),
            message: String::new(),
            amend: false,
            excluded: BTreeSet::new(),
            busy: None,
            operation: None,
            success: None,
            state: rc_vcs::RepoState::Clean,
            unmerged: None,
            compare: None,
            rebase: None,
            rebase_plan: None,
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
        if s.contains("/.git/") && (s.ends_with("HEAD") || s.contains("/refs/") || s.ends_with("index")) {
            self.heads.clear();
            self.marks_version.clear();
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
            let state = repo.state().await;
            out.send(VcsEvent::State(state));
            out.send(VcsEvent::Status(st));
            if let Ok(b) = repo.branches().await {
                out.send(VcsEvent::Branches(b));
            }
            out.send(VcsEvent::Stashes(repo.stashes().await.map_err(|e| e.to_string())));
            let rebase = if state == rc_vcs::RepoState::Rebasing { repo.rebase_status().await.ok().flatten() } else { None };
            out.send(VcsEvent::Rebase(rebase));
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
        if self.busy.is_some() {
            return;
        }
        self.busy = Some(what);
        self.error = None;
        let out = self.inbox.sender();
        let task = Runtime::spawn(f);
        self.operation = Some(task.abort_handle());
        Runtime::spawn(async move {
            let r = match task.await {
                Ok(result) => result,
                Err(error) if error.is_cancelled() => Err(rc_vcs::GitError::Cancelled),
                Err(error) => Err(rc_vcs::GitError::Parse(error.to_string())),
            };
            out.send(VcsEvent::Done(what, r.err().map(|e| error_text(&e))));
        });
    }

    pub fn repository_tool(&mut self, action: tools::ToolAction) {
        if self.busy.is_some() {
            return;
        }
        self.tools.report.clear();
        self.tools.cloned = None;
        let repo = self.repo.clone();
        let out = self.inbox.sender();
        self.op("repository_tool", async move {
            match action.run(repo).await {
                Ok((report, cloned)) => {
                    out.send(VcsEvent::ToolReport(report, cloned));
                    Ok(())
                }
                Err(error) => {
                    out.send(VcsEvent::ToolReport(error.details().unwrap_or_else(|| error.to_string()).chars().take(65_536).collect(), None));
                    Err(error)
                }
            }
        });
    }

    /// Cancels the current operation. The completion event releases the busy state only
    /// after its future has been dropped, which also kills its active Git process.
    pub fn cancel_operation(&self) {
        if let Some(operation) = &self.operation {
            operation.abort();
        }
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

    /// Fetches every remote, pruning remote-tracking branches deleted on the remote.
    pub fn fetch(&mut self) {
        let Some(repo) = self.repo.clone() else { return };
        self.op_notice("fetch", tr("Fetched").to_string(), async move { repo.fetch(&rc_vcs::FetchOptions { prune: true, ..Default::default() }, None, None).await });
    }

    /// [`Vcs::op`] that shows `notice` when it succeeds.
    fn op_notice<F>(&mut self, what: &'static str, notice: String, f: F)
    where
        F: std::future::Future<Output = Result<(), rc_vcs::GitError>> + Send + 'static,
    {
        if self.busy.is_some() {
            return;
        }
        self.op(what, f);
        self.success = Some(notice);
    }

    /// Checks out a local branch, or a remote one as a local tracking branch.
    pub fn checkout(&mut self, branch: &Branch) {
        let Some(repo) = self.repo.clone() else { return };
        let b = branch.clone();
        let local = if b.kind == rc_vcs::BranchKind::Remote { b.remote_parts().map_or(b.name.as_str(), |(_, n)| n) } else { b.name.as_str() };
        let notice = trf("Checked out {branch}", &[("branch", &local)]);
        self.op_notice("checkout", notice, async move {
            repo.ensure_no_conflicts().await?;
            if b.kind == rc_vcs::BranchKind::Remote { repo.checkout_tracking(&b.name).await } else { repo.checkout_branch(&b.name).await }
        });
    }

    /// Creates a branch at `start` (default HEAD), optionally checking it out.
    pub fn new_branch_from(&mut self, name: &str, start: Option<&str>, checkout: bool) {
        let Some(repo) = self.repo.clone() else { return };
        let (n, start) = (name.trim().to_string(), start.map(str::to_string));
        let notice = trf("Created branch {branch}", &[("branch", &n)]);
        self.op_notice("branch", notice, async move {
            repo.check_branch_name(&n).await?;
            if checkout {
                repo.ensure_no_conflicts().await?;
            }
            repo.create_branch(&n, start.as_deref(), checkout).await
        });
    }

    pub fn rename_branch(&mut self, old: &str, new: &str) {
        let Some(repo) = self.repo.clone() else { return };
        let (old, new) = (old.to_string(), new.trim().to_string());
        let notice = trf("Renamed {old} to {new}", &[("old", &old), ("new", &new)]);
        self.op_notice("branch_rename", notice, async move {
            repo.check_branch_name(&new).await?;
            repo.ensure_no_conflicts().await?;
            repo.rename_branch(&old, &new).await
        });
    }

    /// Deletes a local branch. Without `force`, a branch that is not fully merged is kept and
    /// [`Vcs::unmerged`] asks for a forced delete.
    pub fn delete_branch(&mut self, name: &str, force: bool) {
        let Some(repo) = self.repo.clone() else { return };
        let n = name.to_string();
        let out = self.inbox.sender();
        let notice = trf("Deleted branch {branch}", &[("branch", &n)]);
        self.op_notice("branch_delete", notice, async move {
            repo.ensure_no_conflicts().await?;
            match repo.delete_branch(&n, force).await {
                Err(rc_vcs::GitError::BranchNotFullyMerged(_)) if !force => {
                    out.send(VcsEvent::Unmerged(n));
                    Ok(())
                }
                r => r,
            }
        });
    }

    /// Deletes a branch on its remote (`origin/feature`): affects the server.
    pub fn delete_remote_branch(&mut self, remote_branch: &str) {
        let Some(repo) = self.repo.clone() else { return };
        let n = remote_branch.to_string();
        let notice = trf("Deleted remote branch {branch}", &[("branch", &n)]);
        self.op_notice("remote_delete", notice, async move {
            repo.ensure_no_conflicts().await?;
            repo.delete_remote_branch(&n, None).await
        });
    }

    /// Merges `branch` into the current branch.
    pub fn merge_branch(&mut self, branch: &str) {
        let Some(repo) = self.repo.clone() else { return };
        let n = branch.to_string();
        let notice = trf("Merged {branch}", &[("branch", &n)]);
        self.op_notice("merge", notice, async move {
            repo.ensure_no_conflicts().await?;
            repo.merge(&n, false).await
        });
    }

    /// Rebases the current branch onto `branch`.
    pub fn rebase_onto(&mut self, branch: &str) {
        let Some(repo) = self.repo.clone() else { return };
        let n = branch.to_string();
        let notice = trf("Rebased onto {branch}", &[("branch", &n)]);
        self.op_notice("rebase", notice, async move {
            repo.ensure_no_conflicts().await?;
            repo.rebase(&n).await
        });
    }

    /// Pushes a local branch to its upstream, or sets one on the default remote.
    pub fn push_branch(&mut self, branch: &str) {
        let Some(repo) = self.repo.clone() else { return };
        let n = branch.to_string();
        let notice = trf("Pushed {branch}", &[("branch", &n)]);
        self.op_notice("push_branch", notice, async move { repo.push_branch(&n, None, None).await.map(drop) });
    }

    /// Aborts the merge or rebase in progress.
    pub fn abort_operation(&mut self) {
        let Some(repo) = self.repo.clone() else { return };
        let rebase = self.state == rc_vcs::RepoState::Rebasing;
        self.op_notice("abort", if rebase { tr("Rebase aborted") } else { tr("Merge aborted") }.to_string(), async move {
            if rebase { repo.abort_rebase().await } else { repo.abort_merge().await }
        });
    }

    /// Loads the plan of an interactive rebase from `from` (included) up to HEAD.
    pub fn load_rebase_plan(&mut self, from: &str) {
        let Some(repo) = self.repo.clone() else { return };
        let from = from.to_string();
        self.rebase_plan = None;
        let out = self.inbox.sender();
        Runtime::spawn(async move {
            let plan = repo.interactive_rebase_plan(&from).await.map_err(|e| error_text(&e));
            out.send(VcsEvent::RebasePlan(from, plan));
        });
    }

    /// Runs an interactive rebase; it may stop for conflicts or edits ([`Vcs::rebase`]).
    pub fn interactive_rebase(&mut self, plan: rc_vcs::RebasePlan) {
        let Some(repo) = self.repo.clone() else { return };
        self.rebase_op("rebase_interactive", async move {
            repo.ensure_no_conflicts().await?;
            repo.interactive_rebase(&plan).await
        });
    }

    /// Continues the stopped rebase (after resolving conflicts or editing a commit).
    pub fn continue_rebase(&mut self) {
        let Some(repo) = self.repo.clone() else { return };
        self.rebase_op("rebase_continue", async move { repo.continue_rebase().await });
    }

    /// Skips the commit the rebase stopped at.
    pub fn skip_rebase(&mut self) {
        let Some(repo) = self.repo.clone() else { return };
        self.rebase_op("rebase_skip", async move { repo.skip_rebase().await });
    }

    fn rebase_op<F>(&mut self, what: &'static str, f: F)
    where
        F: std::future::Future<Output = Result<rc_vcs::RebaseOutcome, rc_vcs::GitError>> + Send + 'static,
    {
        let out = self.inbox.sender();
        self.op_notice(what, tr("Rebase finished").to_string(), async move {
            if let rc_vcs::RebaseOutcome::Stopped(st) = f.await? {
                out.send(VcsEvent::RebaseStopped(st));
            }
            Ok(())
        });
    }

    /// Resolves a conflicted file by taking one side whole (`theirs`: the commit being
    /// applied; otherwise the result rebased so far, or the current branch in a merge).
    pub fn resolve_conflict(&mut self, path: &str, theirs: bool) {
        let Some(repo) = self.repo.clone() else { return };
        let paths = vec![path.to_string()];
        self.op("resolve", async move { repo.resolve_with(&paths, theirs).await });
    }

    /// Marks a conflicted file resolved with its current content.
    pub fn mark_resolved(&mut self, path: &str) {
        let Some(repo) = self.repo.clone() else { return };
        let paths = vec![path.to_string()];
        self.op("resolve", async move { repo.mark_resolved(&paths).await });
    }

    /// Loads the commits that differ between `branch` and the current branch.
    pub fn compare_with_current(&mut self, branch: &str) {
        let Some(repo) = self.repo.clone() else { return };
        let n = branch.to_string();
        self.compare = Some(BranchCompare { branch: n.clone(), loading: true, ..Default::default() });
        let out = self.inbox.sender();
        Runtime::spawn(async move {
            let r = async {
                let incoming = repo.commits_between("HEAD", &n, 200).await?;
                let outgoing = repo.commits_between(&n, "HEAD", 200).await?;
                Ok::<_, rc_vcs::GitError>((incoming, outgoing))
            }
            .await
            .map_err(|e| error_text(&e));
            out.send(VcsEvent::Compare(n, r));
        });
    }

    /// Rollback: restores files from HEAD (new files are only unstaged).
    pub fn rollback(&mut self, paths: Vec<String>) {
        let Some(repo) = self.repo.clone() else { return };
        self.op("rollback", async move { repo.rollback(&paths).await });
    }

    pub fn stash_push(&mut self) {
        let Some(repo) = self.repo.clone() else { return };
        let opts = rc_vcs::StashOptions {
            message: (!self.stash_message.trim().is_empty()).then(|| self.stash_message.trim().to_string()),
            include_untracked: self.stash_include_untracked,
            keep_index: self.stash_keep_index,
            ..Default::default()
        };
        self.op("stash_push", async move { repo.stash_push(&opts).await });
    }

    /// Resolve the selected stash against a fresh list so a stale UI index cannot select
    /// a different entry after an external stash operation.
    pub fn stash_apply(&mut self, stash: &rc_vcs::Stash, pop: bool) {
        let Some(repo) = self.repo.clone() else { return };
        let hash = stash.hash.clone();
        let restore_index = self.stash_restore_index;
        self.op(if pop { "stash_pop" } else { "stash_apply" }, async move {
            let entry = repo.stashes().await?.into_iter().find(|s| s.hash == hash).ok_or_else(|| rc_vcs::GitError::NotFound(hash))?;
            if pop { repo.stash_pop(entry.index, restore_index).await } else { repo.stash_apply(entry.index, restore_index).await }
        });
    }

    pub fn stash_drop(&mut self, stash: &rc_vcs::Stash) {
        let Some(repo) = self.repo.clone() else { return };
        let hash = stash.hash.clone();
        self.op("stash_drop", async move {
            let entry = repo.stashes().await?.into_iter().find(|s| s.hash == hash).ok_or_else(|| rc_vcs::GitError::NotFound(hash))?;
            repo.stash_drop(entry.index).await
        });
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
                VcsEvent::Stashes(Ok(s)) => self.stashes = s,
                VcsEvent::Stashes(Err(e)) => self.error = Some(e),
                VcsEvent::Log(l) => self.log = l,
                VcsEvent::ToolReport(report, cloned) => {
                    self.tools.report = report;
                    self.tools.cloned = cloned;
                }
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
                VcsEvent::State(st) => self.state = st,
                VcsEvent::Rebase(r) => self.rebase = r,
                VcsEvent::RebasePlan(from, plan) => self.rebase_plan = Some((from, plan)),
                VcsEvent::RebaseStopped(st) => {
                    self.success = Some(rebase_stop_notice(&st));
                    self.rebase = Some(st);
                }
                VcsEvent::Unmerged(name) => {
                    self.success = None;
                    self.unmerged = Some(name);
                }
                VcsEvent::Compare(branch, r) => {
                    if let Some(c) = self.compare.as_mut().filter(|c| c.branch == branch) {
                        c.loading = false;
                        match r {
                            Ok((incoming, outgoing)) => (c.incoming, c.outgoing) = (incoming, outgoing),
                            Err(e) => c.error = Some(e),
                        }
                    }
                }
                VcsEvent::Done(what, err) => {
                    self.busy = None;
                    self.operation = None;
                    let success = self.success.take();
                    match err {
                        Some(e) => {
                            self.error = Some(e.clone());
                            notices.push(e);
                            // A failed merge, rebase or stash can leave conflicts behind.
                            if what.starts_with("stash_") || matches!(what, "merge" | "checkout" | "pull") || what.starts_with("rebase") {
                                self.refresh();
                            }
                        }
                        None => {
                            if what == "stash_push" {
                                self.stash_message.clear();
                            }
                            if what == "commit" || what == "commit_push" {
                                self.message.clear();
                                self.amend = false;
                                notices.push(if what == "commit" { tr("Committed").to_string() } else { tr("Committed and pushed").to_string() });
                            } else if what == "push" {
                                notices.push(tr("Pushed").to_string());
                            } else if what == "pull" {
                                notices.push(tr("Project updated").to_string());
                            } else if let Some(n) = success {
                                notices.push(n);
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

    /// A side-by-side diff of two texts in the editor area (Local History uses it too).
    pub fn show_diff(&mut self, title: String, path: PathBuf, left: &str, right: &str, dark: bool) {
        self.diff = Some(self.make_diff(title, path, left, right, dark));
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
}

impl Drop for Vcs {
    fn drop(&mut self) {
        self.cancel_operation();
    }
}

/// The translated message of a Git error.
/// The notice shown when a rebase step stops.
pub fn rebase_stop_notice(st: &rc_vcs::RebaseStatus) -> String {
    let commit = st.commit.as_ref().map(|(h, s)| format!("{} {s}", &h[..h.len().min(8)])).unwrap_or_default();
    match st.stop {
        rc_vcs::RebaseStop::Conflicts => tr("Rebase stopped: resolve the conflicts, then continue").to_string(),
        rc_vcs::RebaseStop::Edit => trf("Rebase stopped to edit {commit}", &[("commit", &commit)]),
        _ => tr("Rebase stopped").to_string(),
    }
}

fn error_text(e: &rc_vcs::GitError) -> String {
    let (tpl, args) = e.user_message();
    let args: Vec<(&str, &dyn std::fmt::Display)> = args.iter().map(|(k, v)| (*k, v as &dyn std::fmt::Display)).collect();
    trf(tpl, &args)
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
    fn cancelling_and_closing_stop_background_git_operations() {
        use super::*;
        use std::sync::{Arc, mpsc};

        struct NotifyOnDrop(mpsc::Sender<()>);
        impl Drop for NotifyOnDrop {
            fn drop(&mut self) {
                let _ = self.0.send(());
            }
        }

        for close in [false, true] {
            let mut vcs = Vcs::new(Waker::noop());
            let (started, start) = mpsc::channel();
            let (dropped, drop_event) = mpsc::channel();
            vcs.op("stash_push", async move {
                let _guard = NotifyOnDrop(dropped);
                started.send(()).unwrap();
                std::future::pending::<Result<(), rc_vcs::GitError>>().await
            });
            start.recv_timeout(Duration::from_secs(3)).unwrap();
            if close {
                vcs.close();
                assert!(vcs.busy.is_none());
            } else {
                vcs.cancel_operation();
            }
            drop_event.recv_timeout(Duration::from_secs(3)).unwrap();
            if !close {
                let mut docs = Docs::new(rc_syntax::ParserPool::new(Arc::new(|| {})));
                let deadline = Instant::now() + Duration::from_secs(3);
                while vcs.busy.is_some() && Instant::now() < deadline {
                    vcs.update(&mut docs, true);
                    std::thread::sleep(Duration::from_millis(1));
                }
                assert!(vcs.busy.is_none());
                assert!(vcs.error.is_some());
            }
        }
    }

    #[test]
    fn dates() {
        assert_eq!(super::format_date(0), "1970-01-01");
        assert_eq!(super::format_date(1_700_000_000), "2023-11-14");
    }
}
