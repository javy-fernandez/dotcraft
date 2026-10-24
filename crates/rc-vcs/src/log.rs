//! Commit history from `git log` with a fixed NUL-separated format, and the files changed by
//! a commit.

use crate::error::{GitError, Result};
use crate::repo::Repository;

/// A ref decorating a commit.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum RefLabel {
    /// `HEAD` (detached, or pointing to `branch` when the branch is also listed).
    Head,
    LocalBranch(String),
    RemoteBranch(String),
    Tag(String),
    /// Anything else (`refs/stash`, notes…), full name.
    Other(String),
}

/// One commit of the log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Commit {
    pub hash: String,
    pub parents: Vec<String>,
    pub author_name: String,
    pub author_email: String,
    /// Unix seconds.
    pub author_time: i64,
    pub committer_name: String,
    pub committer_email: String,
    pub commit_time: i64,
    pub refs: Vec<RefLabel>,
    /// Name of the branch HEAD points to, when HEAD is on this commit.
    pub head_branch: Option<String>,
    pub subject: String,
    pub body: String,
}

impl Commit {
    pub fn short_hash(&self) -> &str {
        &self.hash[..self.hash.len().min(8)]
    }

    pub fn is_merge(&self) -> bool {
        self.parents.len() > 1
    }
}

/// Order of the log.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum LogOrder {
    /// Children before parents, branches kept together (best for the graph).
    #[default]
    Topo,
    Date,
}

/// What [`Repository::log`] lists. One page is `skip..skip + max_count`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogOptions {
    /// Starting revisions (default `HEAD`); ignored entries when `all`.
    pub revs: Vec<String>,
    /// All refs (`--all`).
    pub all: bool,
    pub skip: usize,
    pub max_count: usize,
    /// Only commits touching these paths.
    pub paths: Vec<String>,
    /// Follow renames of a single path (`--follow`).
    pub follow: bool,
    pub author: Option<String>,
    /// Commits whose message matches (`--grep`, case-insensitive).
    pub grep: Option<String>,
    pub order: LogOrder,
}

impl Default for LogOptions {
    fn default() -> Self {
        LogOptions { revs: Vec::new(), all: false, skip: 0, max_count: 500, paths: Vec::new(), follow: false, author: None, grep: None, order: LogOrder::Topo }
    }
}

const FORMAT: &str = "--format=%x1e%H%x00%P%x00%an%x00%ae%x00%at%x00%cn%x00%ce%x00%ct%x00%D%x00%s%x00%b";

/// How a file changed in a commit (`--name-status`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChangedFile {
    /// `A`, `M`, `D`, `R`, `C`, `T`.
    pub status: char,
    pub path: String,
    /// Source of a rename/copy.
    pub orig_path: Option<String>,
}

impl Repository {
    /// One page of history.
    pub async fn log(&self, opts: &LogOptions) -> Result<Vec<Commit>> {
        let mut cmd = self.cmd().args(["log", "--decorate=full", "--no-color", FORMAT]).arg(format!("--skip={}", opts.skip)).arg(format!("--max-count={}", opts.max_count));
        cmd = cmd.arg(match opts.order {
            LogOrder::Topo => "--topo-order",
            LogOrder::Date => "--date-order",
        });
        if opts.follow {
            cmd = cmd.arg("--follow");
        }
        if let Some(a) = &opts.author {
            cmd = cmd.arg(format!("--author={a}"));
        }
        if let Some(g) = &opts.grep {
            cmd = cmd.args(["--regexp-ignore-case", "--extended-regexp"]).arg(format!("--grep={g}"));
        }
        if opts.all {
            cmd = cmd.arg("--all");
        } else {
            cmd = cmd.arg("--end-of-options");
            cmd = if opts.revs.is_empty() { cmd.arg("HEAD") } else { cmd.args(&opts.revs) };
        }
        let out = match cmd.arg("--").args(&opts.paths).output().await {
            Ok(out) => out,
            // An unborn branch has no history yet.
            Err(GitError::NotFound(_)) if opts.revs.is_empty() && !opts.all && self.resolve("HEAD").await.is_err() => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        parse_log(&out.stdout)
    }

    /// The files a commit changed against its first parent (everything for a root commit).
    pub async fn commit_files(&self, rev: &str) -> Result<Vec<ChangedFile>> {
        let out = self.cmd().args(["diff-tree", "--no-commit-id", "-r", "--root", "-m", "--first-parent", "--find-renames", "--name-status", "-z", "--end-of-options", rev]).output().await?;
        let mut files = Vec::new();
        let mut it = out.stdout.split(|b| *b == 0).filter(|s| !s.is_empty()).map(|s| String::from_utf8_lossy(s).into_owned());
        while let Some(status) = it.next() {
            let code = status.chars().next().unwrap_or('M');
            let first = it.next().ok_or_else(|| GitError::Parse(format!("diff-tree: {status}")))?;
            if code == 'R' || code == 'C' {
                let to = it.next().ok_or_else(|| GitError::Parse(format!("diff-tree: {status}")))?;
                files.push(ChangedFile { status: code, path: to, orig_path: Some(first) });
            } else {
                files.push(ChangedFile { status: code, path: first, orig_path: None });
            }
        }
        Ok(files)
    }

    /// The full message of a commit (to prefill "Amend").
    pub async fn commit_message(&self, rev: &str) -> Result<String> {
        Ok(self.cmd().args(["log", "-1", "--format=%B", "--end-of-options", rev]).text().await?.trim_end().to_string())
    }
}

/// Parses the output of `git log` run with this module's format.
pub fn parse_log(out: &[u8]) -> Result<Vec<Commit>> {
    let text = String::from_utf8_lossy(out);
    let mut commits = Vec::new();
    for rec in text.split('\x1e').skip(1) {
        let f: Vec<&str> = rec.splitn(11, '\0').collect();
        if f.len() != 11 {
            return Err(GitError::Parse(format!("log record: {rec:?}")));
        }
        let (refs, head_branch) = parse_refs(f[8]);
        commits.push(Commit {
            hash: f[0].to_string(),
            parents: f[1].split_whitespace().map(str::to_string).collect(),
            author_name: f[2].to_string(),
            author_email: f[3].to_string(),
            author_time: f[4].parse().unwrap_or(0),
            committer_name: f[5].to_string(),
            committer_email: f[6].to_string(),
            commit_time: f[7].parse().unwrap_or(0),
            refs,
            head_branch,
            subject: f[9].to_string(),
            body: f[10].trim_end().to_string(),
        });
    }
    Ok(commits)
}

/// Parses `%D` with `--decorate=full`: `HEAD -> refs/heads/main, refs/remotes/origin/main, tag: refs/tags/v1`.
pub fn parse_refs(d: &str) -> (Vec<RefLabel>, Option<String>) {
    let mut refs = Vec::new();
    let mut head_branch = None;
    for part in d.split(", ").filter(|p| !p.is_empty()) {
        let name = if let Some(target) = part.strip_prefix("HEAD -> ") {
            refs.push(RefLabel::Head);
            head_branch = Some(target.strip_prefix("refs/heads/").unwrap_or(target).to_string());
            target
        } else {
            part
        };
        let label = if name == "HEAD" {
            RefLabel::Head
        } else if let Some(t) = name.strip_prefix("tag: ") {
            RefLabel::Tag(t.strip_prefix("refs/tags/").unwrap_or(t).to_string())
        } else if let Some(b) = name.strip_prefix("refs/heads/") {
            RefLabel::LocalBranch(b.to_string())
        } else if let Some(b) = name.strip_prefix("refs/remotes/") {
            RefLabel::RemoteBranch(b.to_string())
        } else {
            RefLabel::Other(name.to_string())
        };
        if !(name == "HEAD" && refs.contains(&RefLabel::Head)) {
            refs.push(label);
        }
    }
    (refs, head_branch)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_records_and_refs() {
        let out = "\x1eaaa\0p1 p2\0Ana\0ana@x\x00100\0Bob\0bob@x\x00200\0HEAD -> refs/heads/main, refs/remotes/origin/main, tag: refs/tags/v1\0Merge it\0Body line\n\nmore\n\n\x1ebbb\0\0Ana\0ana@x\x0050\0Ana\0ana@x\x0050\0\0Root\0\n";
        let log = parse_log(out.as_bytes()).unwrap();
        assert_eq!(log.len(), 2);
        assert!(log[0].is_merge());
        assert_eq!(log[0].body, "Body line\n\nmore");
        assert_eq!(log[0].head_branch.as_deref(), Some("main"));
        assert_eq!(log[0].refs, vec![RefLabel::Head, RefLabel::LocalBranch("main".into()), RefLabel::RemoteBranch("origin/main".into()), RefLabel::Tag("v1".into())]);
        assert!(log[1].parents.is_empty());
        assert_eq!(log[1].subject, "Root");
    }
}
