//! Explicit repository maintenance operations. Call from a worker, never the UI thread.
use crate::{CancelToken, Git, GitError, Repository, Result};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Worktree {
    pub path: PathBuf,
    pub branch: Option<String>,
    pub head: String,
    pub locked: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubmoduleState {
    AtRecordedCommit,
    Uninitialized,
    DifferentCommit,
    Conflict,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Submodule {
    pub path: PathBuf,
    pub commit: String,
    pub state: SubmoduleState,
    pub description: Option<String>,
}

fn parse_submodule_status(text: &str) -> Result<Vec<Submodule>> {
    text.lines().map(|line| {
        let (marker, rest) = line.get(..1).zip(line.get(1..)).ok_or_else(|| GitError::Parse("Empty submodule status row".into()))?;
        let state = match marker {
            " " => SubmoduleState::AtRecordedCommit,
            "-" => SubmoduleState::Uninitialized,
            "+" => SubmoduleState::DifferentCommit,
            "U" => SubmoduleState::Conflict,
            _ => return Err(GitError::Parse(format!("Unexpected submodule status: {line}"))),
        };
        let (commit, path) = rest.split_once(' ').ok_or_else(|| GitError::Parse(format!("Incomplete submodule status: {line}")))?;
        if !matches!(commit.len(), 40 | 64) || !commit.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(GitError::Parse(format!("Invalid submodule commit: {line}")));
        }
        let (path, description) = match path.strip_suffix(')').and_then(|path| path.rsplit_once(" (")) {
            Some((path, description)) => (path, Some(description.to_string())),
            None => (path, None),
        };
        if path.is_empty() || Path::new(path).is_absolute() || path.split('/').any(|part| part == "..") {
            return Err(GitError::Parse(format!("Invalid submodule path: {line}")));
        }
        Ok(Submodule { path: path.into(), commit: commit.to_string(), state, description })
    }).collect()
}

fn input(value: &str) -> Result<&str> {
    let value = value.trim();
    if value.is_empty() || value.starts_with('-') || value.contains(['\0', '\n', '\r']) {
        return Err(GitError::Parse("Enter a nonempty value without option prefixes or control characters".into()));
    }
    Ok(value)
}

fn destination(path: &Path) -> Result<()> {
    if !path.is_absolute() || path.file_name().is_none() || path.exists() {
        return Err(GitError::Parse("Choose a new absolute destination path".into()));
    }
    if !path.parent().is_some_and(Path::is_dir) {
        return Err(GitError::Parse("The destination parent directory must exist".into()));
    }
    Ok(())
}

impl Git {
    /// Clone to a new directory. Partial data is retained if cancelled, never deleted blindly.
    pub async fn clone_repository(source: &str, path: &Path) -> Result<()> {
        let source = input(source)?;
        if source.contains("::") || source.contains("://") && !["https://", "http://", "ssh://", "git://", "file://"].iter().any(|s| source.starts_with(s)) {
            return Err(GitError::Parse("Unsupported repository transport".into()));
        }
        destination(path)?;
        let git = Self::detect().await?;
        git.command(path.parent().unwrap()).args(["clone", "--progress", "--"]).arg(source).arg(path).output().await.map(drop)
    }
}

impl Repository {
    fn require_worktree_listing_version(&self) -> Result<()> {
        let required = crate::git::GitVersion { major: 2, minor: 36, patch: 0 };
        if self.git().version() < required {
            return Err(GitError::UnsupportedVersion { found: self.git().version().to_string(), required: required.to_string() });
        }
        Ok(())
    }

    /// Apply a single commit; merge commits and revision ranges are not accepted.
    pub async fn cherry_pick_commit(&self, revision: &str) -> Result<()> {
        let revision = input(revision)?;
        if self.state().await != crate::RepoState::Clean {
            return Err(GitError::Conflicts("Finish or abort the current repository operation first".into()));
        }
        let status = self.status(&Default::default()).await?;
        if !status.entries.is_empty() {
            return Err(GitError::LocalChangesWouldBeOverwritten(status.entries.iter().map(|e| e.path.clone()).collect()));
        }
        let commit = self.resolve(revision).await?;
        let parents = self.cmd().args(["rev-list", "--parents", "-n", "1"]).arg(&commit).text().await?;
        if parents.split_whitespace().count() > 2 {
            return Err(GitError::Parse("Cherry-picking merge commits requires a mainline choice and is not supported".into()));
        }
        self.cmd().args(["cherry-pick", "--no-edit"]).arg(commit).output().await.map(drop)
    }

    pub async fn cherry_pick_continue(&self) -> Result<()> {
        self.cmd().args(["cherry-pick", "--continue"]).output().await.map(drop)
    }

    pub async fn cherry_pick_abort(&self) -> Result<()> {
        self.cmd().args(["cherry-pick", "--abort"]).output().await.map(drop)
    }

    pub async fn worktrees(&self) -> Result<Vec<Worktree>> {
        self.require_worktree_listing_version()?;
        let text = self.cmd().args(["worktree", "list", "--porcelain", "-z"]).text().await?;
        let mut result = Vec::new();
        let mut current: Option<Worktree> = None;
        for field in text.split('\0') {
            if let Some(path) = field.strip_prefix("worktree ") {
                if let Some(value) = current.take() {
                    result.push(value);
                }
                current = Some(Worktree { path: path.into(), branch: None, head: String::new(), locked: false });
            } else if let Some(value) = &mut current {
                if let Some(head) = field.strip_prefix("HEAD ") {
                    value.head = head.into();
                }
                if let Some(branch) = field.strip_prefix("branch ") {
                    value.branch = Some(branch.trim_start_matches("refs/heads/").into());
                }
                if field == "locked" || field.starts_with("locked ") {
                    value.locked = true;
                }
            }
        }
        if let Some(value) = current {
            result.push(value);
        }
        Ok(result)
    }

    /// Create a new branch and worktree together; never reuse or force-reset a branch.
    pub async fn add_worktree(&self, path: &Path, branch: &str) -> Result<()> {
        self.require_worktree_listing_version()?;
        destination(path)?;
        let branch = input(branch)?;
        self.cmd().args(["check-ref-format", "--branch"]).arg(branch).output().await?;
        self.cmd().args(["worktree", "add", "-b"]).arg(branch).arg("--").arg(path).arg("HEAD").output().await.map(drop)
    }

    /// Only registered linked worktrees can be removed, without --force.
    pub async fn remove_worktree(&self, path: &Path) -> Result<()> {
        let path = dunce::canonicalize(path)?;
        let registered = self.worktrees().await?;
        if path == self.root() || !registered.iter().skip(1).any(|w| w.path == path) {
            return Err(GitError::Parse("Select a registered linked worktree".into()));
        }
        self.cmd().args(["worktree", "remove", "--"]).arg(path).output().await.map(drop)
    }

    pub async fn submodule_status(&self) -> Result<String> {
        self.cmd().args(["submodule", "status", "--recursive"]).text().await
    }

    /// Recursive status with Git's recorded commit and checkout state separated for callers.
    pub async fn submodules(&self) -> Result<Vec<Submodule>> {
        parse_submodule_status(&self.submodule_status().await?)
    }

    /// Explicitly initialize/update the recorded commits; never enable --remote or file transport.
    pub async fn update_submodules(&self) -> Result<()> {
        self.update_submodules_cancellable(None).await
    }

    /// Cancel an in-flight recursive update, including Git's spawned helper processes.
    pub async fn update_submodules_cancellable(&self, cancel: Option<&CancelToken>) -> Result<()> {
        self.cmd().args(["submodule", "update", "--init", "--recursive"]).cancel(cancel).output().await.map(drop)
    }

    pub async fn lfs_status(&self) -> Result<String> {
        let version = self.cmd().args(["lfs", "version"]).text().await?;
        let status = self.cmd().args(["lfs", "status"]).text().await?;
        let patterns = self.cmd().args(["lfs", "track"]).text().await?;
        Ok(format!("{version}\n{status}\n{patterns}"))
    }

    /// Track a pattern in .gitattributes. Existing Git history is not migrated.
    pub async fn lfs_track(&self, pattern: &str) -> Result<()> {
        let pattern = input(pattern)?;
        self.cmd().args(["lfs", "version"]).output().await?;
        self.cmd().args(["lfs", "track", "--"]).arg(pattern).output().await.map(drop)
    }

    pub async fn lfs_pull(&self) -> Result<()> {
        self.cmd().args(["lfs", "version"]).output().await?;
        self.cmd().args(["lfs", "pull"]).output().await.map(drop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_status_markers_and_paths_with_spaces() {
        let hash = "a".repeat(40);
        let rows = format!(" {hash} modules/shared library (heads/main)\n-{hash} modules/new library\n+{hash} modules/updated (new commits)\nU{hash} modules/conflicted\n");
        let modules = parse_submodule_status(&rows).unwrap();
        assert_eq!(modules.iter().map(|module| module.state).collect::<Vec<_>>(), [SubmoduleState::AtRecordedCommit, SubmoduleState::Uninitialized, SubmoduleState::DifferentCommit, SubmoduleState::Conflict]);
        assert_eq!(modules[0].path, Path::new("modules/shared library"));
        assert_eq!(modules[0].description.as_deref(), Some("heads/main"));
        assert_eq!(modules[1].path, Path::new("modules/new library"));
        assert_eq!(modules[1].description, None);
    }

    #[test]
    fn rejects_malformed_rows() {
        for row in ["", "?abcd path", "-abcd path", " abcdef ../outside", " abcdef /outside"] {
            let text = if row.is_empty() { "\n" } else { row };
            assert!(parse_submodule_status(text).is_err(), "{row:?}");
        }
    }
}
