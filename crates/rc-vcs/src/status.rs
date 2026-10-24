//! Working tree status from `git status --porcelain=v2 --branch -z`.

use crate::error::{GitError, Result};
use crate::repo::Repository;

/// State of a file on one side (index or work tree) of the status.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FileState {
    Unmodified,
    Modified,
    TypeChanged,
    Added,
    Deleted,
    Renamed,
    Copied,
    Untracked,
    Ignored,
    Unmerged,
}

impl FileState {
    fn from_code(c: u8) -> FileState {
        match c {
            b'M' => FileState::Modified,
            b'T' => FileState::TypeChanged,
            b'A' => FileState::Added,
            b'D' => FileState::Deleted,
            b'R' => FileState::Renamed,
            b'C' => FileState::Copied,
            b'U' => FileState::Unmerged,
            _ => FileState::Unmodified,
        }
    }
}

/// Kind of merge conflict of an unmerged path (the `XY` of porcelain status).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ConflictKind {
    BothDeleted,
    AddedByUs,
    DeletedByThem,
    AddedByThem,
    DeletedByUs,
    BothAdded,
    BothModified,
}

/// One changed, untracked, ignored or conflicted path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusEntry {
    /// Repository-relative, `/`-separated path.
    pub path: String,
    /// Source path of a rename or copy.
    pub orig_path: Option<String>,
    pub index: FileState,
    pub worktree: FileState,
    pub conflict: Option<ConflictKind>,
    /// Rename/copy similarity score (0–100).
    pub score: Option<u8>,
    pub submodule: bool,
}

impl StatusEntry {
    /// Has changes staged in the index.
    pub fn is_staged(&self) -> bool {
        self.conflict.is_none() && !matches!(self.index, FileState::Unmodified | FileState::Untracked | FileState::Ignored)
    }

    /// Has work tree changes not staged yet (untracked files included).
    pub fn is_unstaged(&self) -> bool {
        self.conflict.is_none() && self.worktree != FileState::Unmodified && self.worktree != FileState::Ignored
    }

    pub fn is_untracked(&self) -> bool {
        self.worktree == FileState::Untracked
    }
}

/// Current branch and its upstream.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BranchStatus {
    /// Branch name; `None` when HEAD is detached.
    pub head: Option<String>,
    /// Commit HEAD points to; `None` on an unborn branch (no commits yet).
    pub oid: Option<String>,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
}

/// Result of [`Repository::status`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
    pub branch: BranchStatus,
    pub entries: Vec<StatusEntry>,
    pub stash_count: u32,
}

impl Status {
    pub fn staged(&self) -> impl Iterator<Item = &StatusEntry> {
        self.entries.iter().filter(|e| e.is_staged())
    }

    pub fn unstaged(&self) -> impl Iterator<Item = &StatusEntry> {
        self.entries.iter().filter(|e| e.is_unstaged() && !e.is_untracked())
    }

    pub fn untracked(&self) -> impl Iterator<Item = &StatusEntry> {
        self.entries.iter().filter(|e| e.is_untracked())
    }

    pub fn ignored(&self) -> impl Iterator<Item = &StatusEntry> {
        self.entries.iter().filter(|e| e.worktree == FileState::Ignored)
    }

    /// Unmerged paths (merge conflicts).
    pub fn conflicts(&self) -> impl Iterator<Item = &StatusEntry> {
        self.entries.iter().filter(|e| e.conflict.is_some())
    }

    pub fn entry(&self, path: &str) -> Option<&StatusEntry> {
        self.entries.iter().find(|e| e.path == path)
    }

    /// Parses the output of `git status --porcelain=v2 --branch --show-stash -z`.
    pub fn parse(out: &[u8]) -> Result<Status> {
        let mut status = Status::default();
        let mut records = out.split(|b| *b == 0).filter(|r| !r.is_empty());
        let bad = |r: &[u8]| GitError::Parse(format!("status: {}", String::from_utf8_lossy(r)));
        while let Some(rec) = records.next() {
            let text = String::from_utf8_lossy(rec);
            let fields = |n: usize| -> Result<Vec<&str>> {
                let f: Vec<&str> = text.splitn(n, ' ').collect();
                if f.len() == n { Ok(f) } else { Err(bad(rec)) }
            };
            match rec[0] {
                b'#' => {
                    let f: Vec<&str> = text.splitn(3, ' ').collect();
                    let value = f.get(2).copied().unwrap_or("");
                    match f.get(1).copied() {
                        Some("branch.oid") => status.branch.oid = (value != "(initial)").then(|| value.to_string()),
                        Some("branch.head") => status.branch.head = (value != "(detached)").then(|| value.to_string()),
                        Some("branch.upstream") => status.branch.upstream = Some(value.to_string()),
                        Some("branch.ab") => {
                            for part in value.split(' ') {
                                if let Some(n) = part.strip_prefix('+') {
                                    status.branch.ahead = n.parse().unwrap_or(0);
                                } else if let Some(n) = part.strip_prefix('-') {
                                    status.branch.behind = n.parse().unwrap_or(0);
                                }
                            }
                        }
                        Some("stash") => status.stash_count = value.parse().unwrap_or(0),
                        _ => {}
                    }
                }
                b'1' => {
                    let f = fields(9)?;
                    let xy = f[1].as_bytes();
                    status.entries.push(StatusEntry {
                        path: f[8].to_string(),
                        orig_path: None,
                        index: FileState::from_code(xy[0]),
                        worktree: FileState::from_code(xy[1]),
                        conflict: None,
                        score: None,
                        submodule: f[2].starts_with('S'),
                    });
                }
                b'2' => {
                    let f = fields(10)?;
                    let xy = f[1].as_bytes();
                    let orig = records.next().ok_or_else(|| bad(rec))?;
                    status.entries.push(StatusEntry {
                        path: f[9].to_string(),
                        orig_path: Some(String::from_utf8_lossy(orig).into_owned()),
                        index: FileState::from_code(xy[0]),
                        worktree: FileState::from_code(xy[1]),
                        conflict: None,
                        score: f[8][1..].parse().ok(),
                        submodule: f[2].starts_with('S'),
                    });
                }
                b'u' => {
                    let f = fields(11)?;
                    let conflict = match f[1] {
                        "DD" => ConflictKind::BothDeleted,
                        "AU" => ConflictKind::AddedByUs,
                        "UD" => ConflictKind::DeletedByThem,
                        "UA" => ConflictKind::AddedByThem,
                        "DU" => ConflictKind::DeletedByUs,
                        "AA" => ConflictKind::BothAdded,
                        _ => ConflictKind::BothModified,
                    };
                    status.entries.push(StatusEntry {
                        path: f[10].to_string(),
                        orig_path: None,
                        index: FileState::Unmerged,
                        worktree: FileState::Unmerged,
                        conflict: Some(conflict),
                        score: None,
                        submodule: f[2].starts_with('S'),
                    });
                }
                b'?' | b'!' => {
                    let untracked = rec[0] == b'?';
                    status.entries.push(StatusEntry {
                        path: String::from_utf8_lossy(&rec[2..]).into_owned(),
                        orig_path: None,
                        index: if untracked { FileState::Untracked } else { FileState::Ignored },
                        worktree: if untracked { FileState::Untracked } else { FileState::Ignored },
                        conflict: None,
                        score: None,
                        submodule: false,
                    });
                }
                _ => return Err(bad(rec)),
            }
        }
        Ok(status)
    }
}

/// What [`Repository::status`] includes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusOptions {
    /// List every untracked file (`--untracked-files=all`); `false` omits them.
    pub untracked: bool,
    /// Also list ignored files.
    pub ignored: bool,
    /// Limit to these repository-relative paths (empty = whole work tree).
    pub paths: Vec<String>,
}

impl Default for StatusOptions {
    fn default() -> Self {
        StatusOptions { untracked: true, ignored: false, paths: Vec::new() }
    }
}

impl Repository {
    /// Branch, upstream, ahead/behind and every changed path.
    pub async fn status(&self, opts: &StatusOptions) -> Result<Status> {
        let out = self
            .cmd()
            .args(["status", "--porcelain=v2", "--branch", "--show-stash", "-z", "--find-renames"])
            .arg(if opts.untracked { "--untracked-files=all" } else { "--untracked-files=no" })
            .args(opts.ignored.then_some("--ignored=matching"))
            .arg("--")
            .args(&opts.paths)
            .output()
            .await?;
        Status::parse(&out.stdout)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_porcelain_v2() {
        let out = b"# branch.oid 1234\0# branch.head main\0# branch.upstream origin/main\0# branch.ab +2 -1\0# stash 3\0\
1 .M N... 100644 100644 100644 aaa aaa src/a b.cs\0\
2 R. N... 100644 100644 100644 aaa aaa R95 new name.cs\0old name.cs\0\
u UU N... 100644 100644 100644 100644 a b c conflict.cs\0\
? new file.txt\0! bin/x.dll\0";
        let s = Status::parse(out).unwrap();
        assert_eq!(s.branch, BranchStatus { head: Some("main".into()), oid: Some("1234".into()), upstream: Some("origin/main".into()), ahead: 2, behind: 1 });
        assert_eq!(s.stash_count, 3);
        assert_eq!(s.entries[0].path, "src/a b.cs");
        assert!(s.entries[0].is_unstaged() && !s.entries[0].is_staged());
        assert_eq!(s.entries[1].orig_path.as_deref(), Some("old name.cs"));
        assert_eq!(s.entries[1].score, Some(95));
        assert_eq!(s.entries[1].index, FileState::Renamed);
        assert_eq!(s.conflicts().count(), 1);
        assert_eq!(s.untracked().next().unwrap().path, "new file.txt");
        assert_eq!(s.ignored().next().unwrap().path, "bin/x.dll");
    }

    #[test]
    fn initial_and_detached() {
        let s = Status::parse(b"# branch.oid (initial)\0# branch.head (detached)\0").unwrap();
        assert_eq!(s.branch.oid, None);
        assert_eq!(s.branch.head, None);
    }
}
