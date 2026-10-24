//! A discovered repository: its work tree root, git dir and the git executable used.

use crate::error::{GitError, Result};
use crate::git::{Git, GitCommand};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// An operation in progress that the user must finish or abort.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RepoState {
    Clean,
    Merging,
    Rebasing,
    CherryPicking,
    Reverting,
    Bisecting,
}

/// A git work tree. Cheap to clone; every method runs git in the work tree root.
#[derive(Clone, Debug)]
pub struct Repository {
    git: Arc<Git>,
    root: PathBuf,
    git_dir: PathBuf,
}

impl Repository {
    /// Finds the repository containing `path` (a file or directory) with the shared git.
    pub async fn discover(path: &Path) -> Result<Repository> {
        Self::discover_with(Git::detect().await?, path).await
    }

    /// Finds the repository containing `path` using a specific git executable.
    pub async fn discover_with(git: Arc<Git>, path: &Path) -> Result<Repository> {
        let mut dir = path.to_path_buf();
        while !dir.is_dir() {
            match dir.parent() {
                Some(p) => dir = p.to_path_buf(),
                None => return Err(GitError::NotARepository(path.to_path_buf())),
            }
        }
        let out = git.command(&dir).args(["rev-parse", "--show-toplevel", "--absolute-git-dir"]).text().await.map_err(|e| match e {
            GitError::NotARepository(_) => GitError::NotARepository(path.to_path_buf()),
            e => e,
        })?;
        let mut lines = out.lines();
        let (Some(root), Some(git_dir)) = (lines.next(), lines.next()) else {
            // A bare repository or the inside of `.git` has no work tree.
            return Err(GitError::NotARepository(path.to_path_buf()));
        };
        Ok(Repository { git, root: PathBuf::from(root), git_dir: PathBuf::from(git_dir) })
    }

    /// The work tree root (as git reports it, symlinks resolved).
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The `.git` directory (absolute; for worktrees, the per-worktree git dir).
    pub fn git_dir(&self) -> &Path {
        &self.git_dir
    }

    pub fn git(&self) -> &Arc<Git> {
        &self.git
    }

    /// A git command run in the work tree root.
    pub fn cmd(&self) -> GitCommand<'_> {
        self.git.command(&self.root)
    }

    /// Absolute path of a repository-relative path (`/`-separated).
    pub fn abs_path(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }

    /// Repository-relative, `/`-separated path of an absolute path inside the work tree.
    pub fn rel_path(&self, abs: &Path) -> Option<String> {
        let rel: PathBuf = match abs.strip_prefix(&self.root) {
            Ok(rel) => rel.to_path_buf(),
            Err(_) => {
                // The caller's path may not have symlinks resolved (/var vs /private/var).
                let canon = abs.canonicalize().ok()?;
                let root = self.root.canonicalize().ok()?;
                canon.strip_prefix(&root).ok()?.to_path_buf()
            }
        };
        Some(rel.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/"))
    }

    /// Whether a merge, rebase, cherry-pick… is in progress.
    pub async fn state(&self) -> RepoState {
        let d = self.git_dir.clone();
        let exists = |name: &str| d.join(name).exists();
        if exists("rebase-merge") || exists("rebase-apply") {
            RepoState::Rebasing
        } else if exists("MERGE_HEAD") {
            RepoState::Merging
        } else if exists("CHERRY_PICK_HEAD") {
            RepoState::CherryPicking
        } else if exists("REVERT_HEAD") {
            RepoState::Reverting
        } else if exists("BISECT_LOG") {
            RepoState::Bisecting
        } else {
            RepoState::Clean
        }
    }

    /// The full hash `rev` resolves to (`HEAD`, a branch, a short hash…).
    pub async fn resolve(&self, rev: &str) -> Result<String> {
        Ok(self.cmd().args(["rev-parse", "--verify", "--quiet", "--end-of-options"]).arg(format!("{rev}^{{commit}}")).text().await.map_err(|e| match e {
            GitError::Failed { .. } => GitError::NotFound(rev.to_string()),
            e => e,
        })?
        .trim()
        .to_string())
    }

    /// The current branch name, or `None` when HEAD is detached.
    pub async fn current_branch(&self) -> Result<Option<String>> {
        let out = self.cmd().args(["symbolic-ref", "--quiet", "--short", "HEAD"]).ok_code(1).output().await?;
        Ok((out.code == Some(0)).then(|| out.stdout_text().trim().to_string()))
    }
}
