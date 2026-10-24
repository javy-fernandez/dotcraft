//! Changes to the index and work tree: staging (files, hunks, lines), unstaging,
//! discarding, committing and resolving merge conflicts.

use crate::diff::{FileContent, FileDiff, Revision};
use crate::error::{GitError, Result};
use crate::repo::Repository;

/// Options of [`Repository::commit`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CommitOptions {
    pub message: String,
    /// Replace the last commit (`--amend`).
    pub amend: bool,
    /// Add a `Signed-off-by` trailer (`--signoff`).
    pub signoff: bool,
    /// Commit only these paths, as they are in the work tree (untracked ones are added);
    /// other staged changes stay staged. Empty = commit the index.
    pub only: Vec<String>,
    pub allow_empty: bool,
    /// Override the author (`Name <email>`).
    pub author: Option<String>,
}

/// The three versions of a conflicted file (`None` where a side does not have it).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConflictVersions {
    pub base: Option<FileContent>,
    pub ours: Option<FileContent>,
    pub theirs: Option<FileContent>,
}

impl Repository {
    /// Stages paths: modifications, new files and deletions (`git add -A`).
    pub async fn stage(&self, paths: &[String]) -> Result<()> {
        if paths.is_empty() {
            return Ok(());
        }
        self.cmd().args(["add", "-A", "--"]).args(paths).output().await.map(drop)
    }

    /// Stages every change of the work tree.
    pub async fn stage_all(&self) -> Result<()> {
        self.cmd().args(["add", "-A"]).output().await.map(drop)
    }

    /// Removes paths from the index, keeping work tree changes (works without commits too).
    pub async fn unstage(&self, paths: &[String]) -> Result<()> {
        if paths.is_empty() {
            return Ok(());
        }
        self.cmd().args(["reset", "-q", "--"]).args(paths).output().await.map(drop)
    }

    /// Applies a patch to the index (`git apply --cached`), forward or reversed.
    pub async fn apply_to_index(&self, patch: &str, reverse: bool) -> Result<()> {
        self.apply(patch, true, reverse).await
    }

    /// Applies a patch to the work tree (`git apply`), forward or reversed.
    pub async fn apply_to_worktree(&self, patch: &str, reverse: bool) -> Result<()> {
        self.apply(patch, false, reverse).await
    }

    async fn apply(&self, patch: &str, cached: bool, reverse: bool) -> Result<()> {
        self.cmd()
            .args(["apply", "--whitespace=nowarn"])
            .args(cached.then_some("--cached"))
            .args(reverse.then_some("--reverse"))
            .arg("-")
            .stdin(patch.as_bytes().to_vec())
            .output()
            .await
            .map(drop)
    }

    /// Stages one hunk of an unstaged diff (`DiffSpec::WorktreeVsIndex`).
    pub async fn stage_hunk(&self, file: &FileDiff, hunk: usize) -> Result<()> {
        let patch = file.hunk_patch(hunk, false).ok_or_else(|| GitError::NotFound(format!("hunk {hunk}")))?;
        self.apply_to_index(&patch, false).await
    }

    /// Stages selected lines (indices into the hunk's lines) of an unstaged diff.
    pub async fn stage_lines(&self, file: &FileDiff, hunk: usize, lines: &[usize]) -> Result<()> {
        let patch = file.lines_patch(hunk, lines, false).ok_or_else(|| GitError::NotFound(format!("hunk {hunk}")))?;
        self.apply_to_index(&patch, false).await
    }

    /// Unstages one hunk of a staged diff (`DiffSpec::IndexVsHead`).
    pub async fn unstage_hunk(&self, file: &FileDiff, hunk: usize) -> Result<()> {
        let patch = file.hunk_patch(hunk, true).ok_or_else(|| GitError::NotFound(format!("hunk {hunk}")))?;
        self.apply_to_index(&patch, true).await
    }

    /// Unstages selected lines of a staged diff.
    pub async fn unstage_lines(&self, file: &FileDiff, hunk: usize, lines: &[usize]) -> Result<()> {
        let patch = file.lines_patch(hunk, lines, true).ok_or_else(|| GitError::NotFound(format!("hunk {hunk}")))?;
        self.apply_to_index(&patch, true).await
    }

    /// Reverts one hunk of an unstaged diff in the work tree ("Rollback hunk").
    pub async fn discard_hunk(&self, file: &FileDiff, hunk: usize) -> Result<()> {
        let patch = file.hunk_patch(hunk, true).ok_or_else(|| GitError::NotFound(format!("hunk {hunk}")))?;
        self.apply_to_worktree(&patch, true).await
    }

    /// Reverts selected lines of an unstaged diff in the work tree.
    pub async fn discard_lines(&self, file: &FileDiff, hunk: usize, lines: &[usize]) -> Result<()> {
        let patch = file.lines_patch(hunk, lines, true).ok_or_else(|| GitError::NotFound(format!("hunk {hunk}")))?;
        self.apply_to_worktree(&patch, true).await
    }

    /// Throws away unstaged changes of tracked paths (restores them from the index).
    pub async fn discard_unstaged(&self, paths: &[String]) -> Result<()> {
        if paths.is_empty() {
            return Ok(());
        }
        self.cmd().args(["restore", "--worktree", "--"]).args(paths).output().await.map(drop)
    }

    /// "Rollback": restores paths to HEAD in both index and work tree. Paths that HEAD does
    /// not have (newly added files) are only unstaged; their files are kept.
    pub async fn rollback(&self, paths: &[String]) -> Result<()> {
        if paths.is_empty() {
            return Ok(());
        }
        let in_head: Vec<String> = match self.resolve("HEAD").await {
            Ok(_) => {
                let out = self.cmd().args(["ls-tree", "-r", "--name-only", "-z", "HEAD", "--"]).args(paths).output().await?;
                out.stdout.split(|b| *b == 0).filter(|s| !s.is_empty()).map(|s| String::from_utf8_lossy(s).into_owned()).collect()
            }
            Err(_) => Vec::new(),
        };
        let (tracked, added): (Vec<String>, Vec<String>) = paths.iter().cloned().partition(|p| in_head.iter().any(|h| h == p || h.starts_with(&format!("{p}/"))));
        if !added.is_empty() {
            self.cmd().args(["rm", "--cached", "-r", "-q", "--ignore-unmatch", "--"]).args(&added).output().await?;
        }
        if !tracked.is_empty() {
            self.cmd().args(["restore", "--source=HEAD", "--staged", "--worktree", "--"]).args(&tracked).output().await?;
        }
        Ok(())
    }

    /// Deletes untracked files (`git clean -f`); ignored files are left alone.
    pub async fn delete_untracked(&self, paths: &[String]) -> Result<()> {
        if paths.is_empty() {
            return Ok(());
        }
        self.cmd().args(["clean", "-f", "-q", "--"]).args(paths).output().await.map(drop)
    }

    /// Creates a commit and returns its hash.
    pub async fn commit(&self, opts: &CommitOptions) -> Result<String> {
        if !opts.only.is_empty() {
            // `--only` refuses untracked paths: add them first.
            self.stage(&opts.only).await?;
        }
        let mut cmd = self.cmd().args(["commit", "--quiet", "--file=-", "--cleanup=strip"]).stdin(opts.message.as_bytes().to_vec());
        if opts.amend {
            cmd = cmd.arg("--amend");
        }
        if opts.signoff {
            cmd = cmd.arg("--signoff");
        }
        if opts.allow_empty {
            cmd = cmd.arg("--allow-empty");
        }
        if let Some(a) = &opts.author {
            cmd = cmd.arg(format!("--author={a}"));
        }
        if !opts.only.is_empty() {
            cmd = cmd.args(["--only", "--"]).args(&opts.only);
        }
        cmd.output().await?;
        self.resolve("HEAD").await
    }

    /// Merges `rev` into the current branch. Conflicts fail with [`GitError::Conflicts`] and
    /// leave the merge in progress (see [`Repository::abort_merge`]).
    pub async fn merge(&self, rev: &str, no_ff: bool) -> Result<()> {
        self.cmd().args(["merge", "--no-edit"]).args(no_ff.then_some("--no-ff")).args(["--end-of-options", rev]).output().await.map(drop)
    }

    pub async fn abort_merge(&self) -> Result<()> {
        self.cmd().args(["merge", "--abort"]).output().await.map(drop)
    }

    /// Paths with merge conflicts.
    pub async fn conflicted_paths(&self) -> Result<Vec<String>> {
        let out = self.cmd().args(["diff", "--name-only", "--diff-filter=U", "-z"]).output().await?;
        let mut paths: Vec<String> = out.stdout.split(|b| *b == 0).filter(|s| !s.is_empty()).map(|s| String::from_utf8_lossy(s).into_owned()).collect();
        paths.dedup();
        Ok(paths)
    }

    /// Base, ours and theirs of a conflicted path (for the merge tool).
    pub async fn conflict_versions(&self, path: &str) -> Result<ConflictVersions> {
        Ok(ConflictVersions {
            base: self.show_file(&Revision::Stage(1), path).await?,
            ours: self.show_file(&Revision::Stage(2), path).await?,
            theirs: self.show_file(&Revision::Stage(3), path).await?,
        })
    }

    /// Resolves conflicts by taking one side for whole files (`checkout --ours/--theirs`),
    /// then marks them resolved.
    pub async fn resolve_with(&self, paths: &[String], theirs: bool) -> Result<()> {
        self.cmd().arg("checkout").arg(if theirs { "--theirs" } else { "--ours" }).arg("--").args(paths).output().await?;
        self.mark_resolved(paths).await
    }

    /// Marks conflicted paths as resolved with their current work tree content.
    pub async fn mark_resolved(&self, paths: &[String]) -> Result<()> {
        self.stage(paths).await
    }
}
