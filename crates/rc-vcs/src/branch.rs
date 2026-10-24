//! Local and remote-tracking branches: listing, creating, checking out, renaming, deleting.

use crate::error::{GitError, Result};
use crate::repo::Repository;

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
