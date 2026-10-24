//! Per-line authorship from `git blame --porcelain`, optionally against the unsaved buffer
//! text (`--contents -`) so the lines match what the editor shows.

use crate::cancel::CancelToken;
use crate::error::{GitError, Result};
use crate::repo::Repository;
use std::collections::HashMap;

/// A commit referenced by blame lines.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BlameCommit {
    pub hash: String,
    pub author: String,
    pub author_email: String,
    /// Unix seconds.
    pub author_time: i64,
    /// `+0200`.
    pub author_tz: String,
    pub committer: String,
    pub committer_time: i64,
    pub summary: String,
    /// File name in that commit (differs after renames).
    pub filename: String,
    /// Parent commit and file name, to blame further back ("Annotate previous revision").
    pub previous: Option<(String, String)>,
    /// The repository's first commit (or the boundary of a limited blame).
    pub boundary: bool,
}

impl BlameCommit {
    /// Lines changed in the buffer / work tree that are not committed yet.
    pub fn is_uncommitted(&self) -> bool {
        self.hash.bytes().all(|b| b == b'0')
    }
}

/// The blame of one line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlameLine {
    /// Index into [`Blame::commits`].
    pub commit: usize,
    /// 1-based line number in that commit's version of the file.
    pub orig_line: u32,
}

/// Blame of a whole file: one entry per line of the blamed text.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Blame {
    pub commits: Vec<BlameCommit>,
    pub lines: Vec<BlameLine>,
}

impl Blame {
    /// The commit of 0-based line `line`.
    pub fn commit_of(&self, line: usize) -> Option<&BlameCommit> {
        self.lines.get(line).map(|l| &self.commits[l.commit])
    }

    /// Parses `git blame --porcelain` output.
    pub fn parse(out: &[u8]) -> Result<Blame> {
        let text = String::from_utf8_lossy(out);
        let mut blame = Blame::default();
        let mut index: HashMap<String, usize> = HashMap::new();
        let mut current: Option<usize> = None;
        let mut pending: Vec<(u32, u32)> = Vec::new();
        for line in text.split('\n') {
            if line.starts_with('\t') {
                // The content line ends the entry.
                let commit = current.ok_or_else(|| GitError::Parse("blame: content before header".into()))?;
                let (orig, fin) = pending.pop().ok_or_else(|| GitError::Parse("blame: content without header".into()))?;
                let at = fin as usize - 1;
                if blame.lines.len() <= at {
                    blame.lines.resize(at + 1, BlameLine { commit, orig_line: 0 });
                }
                blame.lines[at] = BlameLine { commit, orig_line: orig };
                continue;
            }
            let mut words = line.splitn(2, ' ');
            let key = words.next().unwrap_or("");
            let value = words.next().unwrap_or("");
            if key.len() >= 40 && key.bytes().all(|b| b.is_ascii_hexdigit()) {
                let mut nums = value.split(' ').map(|n| n.parse::<u32>().unwrap_or(0));
                let (orig, fin) = (nums.next().unwrap_or(0), nums.next().unwrap_or(0));
                if fin == 0 {
                    return Err(GitError::Parse(format!("blame header: {line}")));
                }
                let i = *index.entry(key.to_string()).or_insert_with(|| {
                    blame.commits.push(BlameCommit { hash: key.to_string(), ..Default::default() });
                    blame.commits.len() - 1
                });
                current = Some(i);
                pending.push((orig, fin));
                continue;
            }
            let Some(c) = current.map(|i| &mut blame.commits[i]) else { continue };
            match key {
                "author" => c.author = value.to_string(),
                "author-mail" => c.author_email = value.trim_start_matches('<').trim_end_matches('>').to_string(),
                "author-time" => c.author_time = value.parse().unwrap_or(0),
                "author-tz" => c.author_tz = value.to_string(),
                "committer" => c.committer = value.to_string(),
                "committer-time" => c.committer_time = value.parse().unwrap_or(0),
                "summary" => c.summary = value.to_string(),
                "filename" => c.filename = value.to_string(),
                "boundary" => c.boundary = true,
                "previous" => {
                    if let Some((h, f)) = value.split_once(' ') {
                        c.previous = Some((h.to_string(), f.to_string()));
                    }
                }
                _ => {}
            }
        }
        Ok(blame)
    }
}

impl Repository {
    /// Blames `path`. With `contents` (the editor buffer), lines changed in it are reported as
    /// uncommitted and line numbers match the buffer; with `rev`, the file at that revision is
    /// blamed instead of the work tree.
    pub async fn blame(&self, path: &str, contents: Option<&str>, rev: Option<&str>, cancel: Option<&CancelToken>) -> Result<Blame> {
        let mut cmd = self.cmd().args(["blame", "--porcelain"]).cancel(cancel);
        if let Some(text) = contents {
            cmd = cmd.args(["--contents", "-"]).stdin(text.as_bytes().to_vec());
        }
        if let Some(rev) = rev {
            cmd = cmd.arg(rev);
        }
        let out = cmd.arg("--").arg(path).output().await?;
        Blame::parse(&out.stdout)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_porcelain() {
        let h1 = "a".repeat(40);
        let h0 = "0".repeat(40);
        let out = format!(
            "{h1} 1 1 2\nauthor Ana\nauthor-mail <ana@x>\nauthor-time 100\nauthor-tz +0200\ncommitter Ana\ncommitter-time 100\nsummary First\nboundary\nfilename a.txt\n\tone\n\
{h1} 2 2\n\ttwo\n{h0} 3 3 1\nauthor Not Committed Yet\nauthor-mail <not.committed.yet>\nauthor-time 200\nauthor-tz +0000\ncommitter Not Committed Yet\ncommitter-time 200\nsummary Version of a.txt from a.txt\nprevious {h1} a.txt\nfilename a.txt\n\tthree\n"
        );
        let b = Blame::parse(out.as_bytes()).unwrap();
        assert_eq!(b.lines.len(), 3);
        assert_eq!(b.commits.len(), 2);
        assert_eq!(b.commit_of(1).unwrap().author, "Ana");
        assert_eq!(b.commit_of(1).unwrap().author_email, "ana@x");
        assert!(b.commit_of(0).unwrap().boundary);
        assert!(b.commit_of(2).unwrap().is_uncommitted());
        assert_eq!(b.lines[1].orig_line, 2);
    }
}
