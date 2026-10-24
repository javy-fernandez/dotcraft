//! Errors of git operations, classified from git's exit code and stderr (git always runs
//! with `LC_ALL=C`, so its messages are English and stable enough to match).

use std::path::PathBuf;

pub type Result<T, E = GitError> = std::result::Result<T, E>;

/// Why a git operation failed. `Display` is English; [`GitError::user_message`] gives a
/// template and arguments for `rc_kit::i18n::trf` (translations in `scripts/i18n/rc-vcs.tsv`).
#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error("Git executable not found")]
    GitNotFound,
    #[error("Git {found} is too old (DotCraft needs {required} or newer)")]
    UnsupportedVersion { found: String, required: String },
    #[error("{0} is not inside a Git repository")]
    NotARepository(PathBuf),
    #[error("Nothing to commit")]
    NothingToCommit,
    #[error("No local changes to stash")]
    NothingToStash,
    #[error("Resolve the merge conflicts first")]
    Conflicts(String),
    #[error("Authentication required")]
    AuthenticationRequired(String),
    #[error("HEAD is detached (not on a branch)")]
    DetachedHead,
    #[error("Local changes would be overwritten")]
    LocalChangesWouldBeOverwritten(Vec<String>),
    #[error("A branch named {0} already exists")]
    BranchExists(String),
    #[error("The branch {0} is not fully merged")]
    BranchNotFullyMerged(String),
    #[error("The current branch has no upstream branch")]
    NoUpstream,
    #[error("The push was rejected (fetch first)")]
    PushRejected(String),
    #[error("The patch does not apply")]
    PatchFailed(String),
    #[error("Not found: {0}")]
    NotFound(String),
    #[error("Set user.name and user.email in Git first")]
    IdentityUnknown,
    #[error("Another Git process is running in this repository")]
    Locked(String),
    #[error("Network error")]
    Network(String),
    #[error("Cancelled")]
    Cancelled,
    #[error("Unexpected Git output: {0}")]
    Parse(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("git {command} failed ({code:?}): {stderr}")]
    Failed { command: String, code: Option<i32>, stderr: String },
}

impl GitError {
    /// English template (with `{name}` placeholders) and its arguments, for
    /// `trf(template, args)` in the UI. Git's own output, when useful, is in [`GitError::details`].
    pub fn user_message(&self) -> (&'static str, Vec<(&'static str, String)>) {
        use GitError::*;
        match self {
            GitNotFound => ("Git executable not found", vec![]),
            UnsupportedVersion { found, required } => ("Git {found} is too old (DotCraft needs {required} or newer)", vec![("found", found.clone()), ("required", required.clone())]),
            NotARepository(p) => ("{path} is not inside a Git repository", vec![("path", p.display().to_string())]),
            NothingToCommit => ("Nothing to commit", vec![]),
            NothingToStash => ("No local changes to stash", vec![]),
            Conflicts(_) => ("Resolve the merge conflicts first", vec![]),
            AuthenticationRequired(_) => ("Authentication required", vec![]),
            DetachedHead => ("HEAD is detached (not on a branch)", vec![]),
            LocalChangesWouldBeOverwritten(_) => ("Local changes would be overwritten", vec![]),
            BranchExists(b) => ("A branch named {branch} already exists", vec![("branch", b.clone())]),
            BranchNotFullyMerged(b) => ("The branch {branch} is not fully merged", vec![("branch", b.clone())]),
            NoUpstream => ("The current branch has no upstream branch", vec![]),
            PushRejected(_) => ("The push was rejected (fetch first)", vec![]),
            PatchFailed(_) => ("The patch does not apply", vec![]),
            NotFound(what) => ("Not found: {what}", vec![("what", what.clone())]),
            IdentityUnknown => ("Set user.name and user.email in Git first", vec![]),
            Locked(_) => ("Another Git process is running in this repository", vec![]),
            Network(_) => ("Network error", vec![]),
            Cancelled => ("Cancelled", vec![]),
            Parse(_) | Io(_) | Failed { .. } => ("Git command failed", vec![]),
        }
    }

    /// Git's own message (stderr) or other detail worth showing under the summary.
    pub fn details(&self) -> Option<String> {
        use GitError::*;
        match self {
            Conflicts(s) | AuthenticationRequired(s) | PushRejected(s) | PatchFailed(s) | Locked(s) | Network(s) | Parse(s) => Some(s.clone()),
            LocalChangesWouldBeOverwritten(files) => Some(files.join("\n")),
            Io(e) => Some(e.to_string()),
            Failed { stderr, .. } => Some(stderr.clone()),
            _ => None,
        }
    }

    /// Classifies a failed git run from its arguments, exit code and output.
    pub fn classify(args: &[String], code: Option<i32>, stdout: &str, stderr: &str) -> GitError {
        let err = stderr.trim();
        let all = format!("{stderr}\n{stdout}");
        let has = |s: &str| all.contains(s);
        let sub = args.iter().find(|a| !a.starts_with('-') && !a.contains('=')).map(String::as_str).unwrap_or("");
        if has("not a git repository") {
            return GitError::NotARepository(PathBuf::from(err.rsplit(": ").next().unwrap_or("").trim_end_matches(')').trim()));
        }
        if has("index.lock") && has("File exists") {
            return GitError::Locked(err.to_string());
        }
        if has("Please tell me who you are") || has("empty ident name") {
            return GitError::IdentityUnknown;
        }
        if has("would be overwritten by") {
            return GitError::LocalChangesWouldBeOverwritten(indented_list(&all));
        }
        if has("CONFLICT")
            || has("needs merge")
            || has("you need to resolve your current index first")
            || has("unmerged files")
            || has("Committing is not possible because you have unmerged files")
            || has("is unmerged")
            || has("Automatic merge failed")
        {
            return GitError::Conflicts(all.trim().to_string());
        }
        if has("nothing to commit") || has("no changes added to commit") || has("nothing added to commit") {
            return GitError::NothingToCommit;
        }
        if has("No local changes to save") {
            return GitError::NothingToStash;
        }
        if has("Authentication failed")
            || has("could not read Username")
            || has("could not read Password")
            || has("terminal prompts disabled")
            || has("Permission denied (publickey")
            || has("HTTP Basic: Access denied")
            || has("The requested URL returned error: 401")
            || has("The requested URL returned error: 403")
        {
            return GitError::AuthenticationRequired(err.to_string());
        }
        if has("You are not currently on a branch") || has("HEAD detached") {
            return GitError::DetachedHead;
        }
        if has("has no upstream branch") || has("no tracking information") {
            return GitError::NoUpstream;
        }
        if has("[rejected]") || has("non-fast-forward") || has("fetch first") || has("failed to push some refs") {
            return GitError::PushRejected(err.to_string());
        }
        if let Some(name) = quoted_after(&all, "a branch named '").or_else(|| quoted_after(&all, "branch '").filter(|_| has("already exists"))) {
            return GitError::BranchExists(name);
        }
        if has("is not fully merged") {
            return GitError::BranchNotFullyMerged(quoted_after(&all, "branch '").unwrap_or_default());
        }
        if has("patch does not apply") || has("corrupt patch") || has("patch failed") || has("No valid patches in input") {
            return GitError::PatchFailed(err.to_string());
        }
        if has("Could not resolve host")
            || has("unable to access")
            || has("Connection refused")
            || has("Connection timed out")
            || has("Could not read from remote repository")
            || has("does not appear to be a git repository")
        {
            return GitError::Network(err.to_string());
        }
        if has("unknown revision")
            || has("bad revision")
            || has("invalid object name")
            || has("Not a valid object name")
            || has("invalid reference")
            || has("did not match any")
            || has("does not exist in")
            || has("exists on disk, but not in")
            || has("no such path")
            || has("not a valid ref")
            || has("does not have any commits yet")
            || has("No stash entries found")
            || has("is not a valid reference")
            || has("not found")
        {
            return GitError::NotFound(err.lines().next().unwrap_or(err).trim_start_matches("fatal: ").trim_start_matches("error: ").to_string());
        }
        GitError::Failed { command: if sub.is_empty() { args.join(" ") } else { sub.to_string() }, code, stderr: err.to_string() }
    }
}

/// The tab-indented file list git prints under "would be overwritten" messages.
fn indented_list(text: &str) -> Vec<String> {
    text.lines().filter(|l| l.starts_with('\t')).map(|l| l.trim().to_string()).collect()
}

fn quoted_after(text: &str, prefix: &str) -> Option<String> {
    let start = text.find(prefix)? + prefix.len();
    let end = text[start..].find('\'')?;
    Some(text[start..start + end].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(stderr: &str) -> GitError {
        GitError::classify(&["commit".into()], Some(1), "", stderr)
    }

    #[test]
    fn classifies_common_failures() {
        assert!(matches!(c("fatal: not a git repository (or any of the parent directories): .git"), GitError::NotARepository(_)));
        assert!(matches!(GitError::classify(&[], Some(1), "nothing to commit, working tree clean", ""), GitError::NothingToCommit));
        assert!(matches!(c("fatal: Authentication failed for 'https://x/'"), GitError::AuthenticationRequired(_)));
        assert!(matches!(c("fatal: a branch named 'dev' already exists"), GitError::BranchExists(b) if b == "dev"));
        assert!(matches!(c("error: the branch 'x' is not fully merged."), GitError::BranchNotFullyMerged(b) if b == "x"));
        assert!(matches!(c("error: patch failed: a.txt:1\nerror: a.txt: patch does not apply"), GitError::PatchFailed(_)));
        assert!(matches!(
            c("error: Your local changes to the following files would be overwritten by checkout:\n\ta.txt\nPlease commit"),
            GitError::LocalChangesWouldBeOverwritten(f) if f == vec!["a.txt".to_string()]
        ));
        assert!(matches!(c("fatal: You are not currently on a branch."), GitError::DetachedHead));
        assert!(matches!(c("error: something odd"), GitError::Failed { .. }));
    }
}
