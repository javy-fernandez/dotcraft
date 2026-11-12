//! Local and remote-tracking branches: listing, creating, checking out, renaming, deleting,
//! merging, rebasing, comparing and pushing them.

use crate::cancel::CancelToken;
use crate::error::{GitError, Result};
use crate::git::ProgressSink;
use crate::log::{Commit, LogOptions};
use crate::remote::PushOptions;
use crate::repo::{RepoState, Repository};

/// Where a branch lives.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BranchKind {
    Local,
    /// A remote-tracking branch (`origin/main`).
    Remote,
}

/// A branch from `git for-each-ref`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Branch {
    /// Short name: `main`, `origin/main`.
    pub name: String,
    /// Full ref: `refs/heads/main`.
    pub full_ref: String,
    pub kind: BranchKind,
    /// Commit the branch points to.
    pub target: String,
    /// Checked out in this work tree.
    pub is_head: bool,
    /// Upstream short name (local branches).
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    /// The upstream was deleted on the remote.
    pub upstream_gone: bool,
    /// Unix seconds of the tip's commit.
    pub commit_time: i64,
    pub subject: String,
}

impl Branch {
    /// For a remote branch: (`origin`, `main`).
    pub fn remote_parts(&self) -> Option<(&str, &str)> {
        (self.kind == BranchKind::Remote).then(|| self.name.split_once('/')).flatten()
    }
}

const FORMAT: &str = "--format=%(refname)%00%(objectname)%00%(HEAD)%00%(upstream:short)%00%(upstream:track,nobracket)%00%(committerdate:unix)%00%(symref)%00%(contents:subject)";

impl Repository {
    /// Local branches then remote-tracking ones, by name (symbolic `origin/HEAD` skipped).
    pub async fn branches(&self) -> Result<Vec<Branch>> {
        let out = self.cmd().args(["for-each-ref", FORMAT, "refs/heads", "refs/remotes"]).text().await?;
        Ok(parse_branches(&out))
    }

    /// Creates `name` at `start` (default HEAD), optionally checking it out.
    pub async fn create_branch(&self, name: &str, start: Option<&str>, checkout: bool) -> Result<()> {
        let mut cmd = if checkout { self.cmd().args(["switch", "--quiet", "--no-track", "-c", name]) } else { self.cmd().args(["branch", "--no-track", "--end-of-options", name]) };
        if let Some(s) = start {
            cmd = cmd.arg(s);
        }
        cmd.output().await.map(drop)
    }

    /// Checks out a local branch.
    pub async fn checkout_branch(&self, name: &str) -> Result<()> {
        self.cmd().args(["switch", "--quiet", "--no-guess", name]).output().await.map(drop)
    }

    /// Checks out a remote-tracking branch (`origin/feature`) as a new local branch tracking
    /// it (named `local_name`, default the remote branch name).
    pub async fn checkout_remote_branch(&self, remote_branch: &str, local_name: Option<&str>) -> Result<()> {
        let local = local_name.unwrap_or_else(|| remote_branch.split_once('/').map_or(remote_branch, |(_, b)| b));
        self.cmd().args(["switch", "--quiet", "-c", local, "--track", remote_branch]).output().await.map(drop)
    }

    /// Checks out a commit with a detached HEAD.
    pub async fn checkout_detached(&self, rev: &str) -> Result<()> {
        self.cmd().args(["switch", "--quiet", "--detach", rev]).output().await.map(drop)
    }

    /// Renames a local branch.
    pub async fn rename_branch(&self, old: &str, new: &str) -> Result<()> {
        self.cmd().args(["branch", "-m", "--end-of-options", old, new]).output().await.map(drop)
    }

    /// Deletes a local branch; without `force`, unmerged branches fail with
    /// [`GitError::BranchNotFullyMerged`].
    pub async fn delete_branch(&self, name: &str, force: bool) -> Result<()> {
        self.cmd().args(["branch", if force { "-D" } else { "-d" }, "--end-of-options", name]).output().await.map(drop).map_err(|e| match e {
            GitError::Failed { stderr, .. } if stderr.contains("not found") => GitError::NotFound(name.to_string()),
            e => e,
        })
    }

    /// Deletes a remote-tracking branch locally only (`git branch -dr origin/x`); to delete
    /// it on the remote use [`Repository::push`] with [`crate::PushOptions::delete`].
    pub async fn delete_remote_tracking_branch(&self, name: &str) -> Result<()> {
        self.cmd().args(["branch", "-d", "-r", "--end-of-options", name]).output().await.map(drop)
    }

    /// Fails with [`GitError::InvalidBranchName`] unless `name` is a valid new branch name.
    pub async fn check_branch_name(&self, name: &str) -> Result<()> {
        if name.is_empty() || name.starts_with('-') {
            return Err(GitError::InvalidBranchName(name.to_string()));
        }
        self.cmd().args(["check-ref-format", "--branch", name]).output().await.map(drop).map_err(|_| GitError::InvalidBranchName(name.to_string()))
    }

    /// Refuses branch operations while a merge, rebase, cherry-pick… is in progress or files
    /// are conflicted.
    pub async fn ensure_no_conflicts(&self) -> Result<()> {
        let conflicted = self.conflicted_paths().await?;
        if self.state().await != RepoState::Clean || !conflicted.is_empty() {
            return Err(GitError::Conflicts(conflicted.join("\n")));
        }
        Ok(())
    }

    /// Checks out a remote-tracking branch (`origin/feature`): the local branch of the same
    /// name when it already tracks it, otherwise a new tracking branch.
    pub async fn checkout_tracking(&self, remote_branch: &str) -> Result<()> {
        let local = remote_branch.split_once('/').map_or(remote_branch, |(_, b)| b);
        if let Some(existing) = self.branches().await?.into_iter().find(|b| b.kind == BranchKind::Local && b.name == local) {
            if existing.upstream.as_deref() == Some(remote_branch) {
                return self.checkout_branch(local).await;
            }
            return Err(GitError::BranchExists(local.to_string()));
        }
        self.checkout_remote_branch(remote_branch, Some(local)).await
    }

    /// Rebases the current branch onto `onto`. Conflicts fail with [`GitError::Conflicts`] and
    /// leave the rebase in progress (see [`Repository::abort_rebase`]).
    pub async fn rebase(&self, onto: &str) -> Result<()> {
        self.cmd().args(["rebase", "--end-of-options", onto]).output().await.map(drop)
    }

    pub async fn abort_rebase(&self) -> Result<()> {
        self.cmd().args(["rebase", "--abort"]).output().await.map(drop)
    }

    /// Applies a commit onto the current branch. Conflicts leave the cherry-pick in progress
    /// until resolved or aborted.
    pub async fn cherry_pick(&self, rev: &str, cancel: Option<&CancelToken>) -> Result<()> {
        self.ensure_no_conflicts().await?;
        let hash = self.resolve(rev).await?;
        self.cmd().args(["cherry-pick", "--no-edit", &hash]).cancel(cancel).output().await.map(drop)
    }

    pub async fn abort_cherry_pick(&self) -> Result<()> {
        self.cmd().args(["cherry-pick", "--abort"]).output().await.map(drop)
    }

    /// Commits reachable from `other` but not from `base` (up to `max`), newest first.
    pub async fn commits_between(&self, base: &str, other: &str, max: usize) -> Result<Vec<Commit>> {
        self.log(&LogOptions { revs: vec![format!("{base}..{other}")], max_count: max, ..Default::default() }).await
    }

    /// Pushes a local branch: to its upstream when it has one, otherwise to the default remote
    /// (`remote.pushDefault`, `origin` or the only remote) setting the upstream. Returns the
    /// remote it pushed to.
    pub async fn push_branch(&self, name: &str, progress: Option<ProgressSink>, cancel: Option<&CancelToken>) -> Result<String> {
        let config = |key: String| async move { self.cmd().args(["config", "--get", &key]).ok_code(1).text().await.map(|s| s.trim().to_string()) };
        let remote = config(format!("branch.{name}.remote")).await?;
        let merge = config(format!("branch.{name}.merge")).await?;
        if !remote.is_empty() && remote != "." && !merge.is_empty() {
            let target = merge.strip_prefix("refs/heads/").unwrap_or(&merge);
            let opts = PushOptions { remote: Some(remote.clone()), refspec: Some(format!("refs/heads/{name}:refs/heads/{target}")), ..Default::default() };
            self.push(&opts, progress, cancel).await?;
            return Ok(remote);
        }
        let remotes = self.remotes().await?;
        let default = config("remote.pushDefault".into()).await?;
        let remote = if !default.is_empty() {
            default
        } else if remotes.iter().any(|r| r.name == "origin") {
            "origin".to_string()
        } else {
            match remotes.as_slice() {
                [only] => only.name.clone(),
                [] => return Err(GitError::NoRemote),
                _ => return Err(GitError::NoUpstream),
            }
        };
        let opts = PushOptions { remote: Some(remote.clone()), refspec: Some(name.to_string()), set_upstream: true, ..Default::default() };
        self.push(&opts, progress, cancel).await?;
        Ok(remote)
    }

    /// Deletes a branch on its remote (`git push origin --delete feature` for `origin/feature`),
    /// which also removes the remote-tracking branch.
    pub async fn delete_remote_branch(&self, remote_branch: &str, cancel: Option<&CancelToken>) -> Result<()> {
        let (remote, branch) = remote_branch.split_once('/').ok_or_else(|| GitError::NotFound(remote_branch.to_string()))?;
        let opts = PushOptions { remote: Some(remote.to_string()), refspec: Some(branch.to_string()), delete: true, ..Default::default() };
        self.push(&opts, None, cancel).await
    }

    /// Sets the upstream of a local branch.
    pub async fn set_upstream(&self, branch: &str, upstream: &str) -> Result<()> {
        self.cmd().args(["branch", "--set-upstream-to", upstream, "--end-of-options", branch]).output().await.map(drop)
    }
}

/// Parses `git for-each-ref` output in this module's format.
pub fn parse_branches(out: &str) -> Vec<Branch> {
    let mut branches = Vec::new();
    for line in out.lines() {
        let f: Vec<&str> = line.splitn(8, '\0').collect();
        if f.len() != 8 || !f[6].is_empty() {
            continue;
        }
        let (kind, name) = if let Some(n) = f[0].strip_prefix("refs/heads/") {
            (BranchKind::Local, n)
        } else if let Some(n) = f[0].strip_prefix("refs/remotes/") {
            (BranchKind::Remote, n)
        } else {
            continue;
        };
        let (mut ahead, mut behind, mut gone) = (0, 0, false);
        for part in f[4].split(", ") {
            if let Some(n) = part.strip_prefix("ahead ") {
                ahead = n.parse().unwrap_or(0);
            } else if let Some(n) = part.strip_prefix("behind ") {
                behind = n.parse().unwrap_or(0);
            } else if part == "gone" {
                gone = true;
            }
        }
        branches.push(Branch {
            name: name.to_string(),
            full_ref: f[0].to_string(),
            kind,
            target: f[1].to_string(),
            is_head: f[2] == "*",
            upstream: (!f[3].is_empty()).then(|| f[3].to_string()),
            ahead,
            behind,
            upstream_gone: gone,
            commit_time: f[5].parse().unwrap_or(0),
            subject: f[7].to_string(),
        });
    }
    branches
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_refs() {
        let out = "refs/heads/main\0abc\0*\0origin/main\0ahead 2, behind 1\x00100\0\0Subject\n\
refs/heads/old\0def\0 \0origin/old\0gone\x00100\0\0S\n\
refs/remotes/origin/HEAD\0abc\0 \0\0\x00100\0refs/remotes/origin/main\0Subject\n\
refs/remotes/origin/main\0abc\0 \0\0\x00100\0\0Subject\n";
        let b = parse_branches(out);
        assert_eq!(b.len(), 3);
        assert!(b[0].is_head && b[0].ahead == 2 && b[0].behind == 1);
        assert!(b[1].upstream_gone);
        assert_eq!(b[2].remote_parts(), Some(("origin", "main")));
    }
}
