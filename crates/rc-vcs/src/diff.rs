//! Unified diffs from `git diff`: parsing into files → hunks → lines, partial patches for
//! hunk/line staging, and file contents at a revision.

use crate::error::{GitError, Result};
use crate::repo::Repository;
use std::borrow::Cow;

/// What is compared.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum DiffSpec {
    /// Unstaged changes (`git diff`).
    WorktreeVsIndex,
    /// Staged changes (`git diff --cached`).
    IndexVsHead,
    /// All local changes (`git diff HEAD`).
    WorktreeVsHead,
    /// Between two revisions (`git diff from to`).
    Revisions { from: String, to: String },
    /// Changes introduced by one commit (against its first parent; root commits against
    /// the empty tree).
    Commit(String),
}

/// Options of [`Repository::diff`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffOptions {
    /// Context lines around changes (`-U`). Hunk staging needs at least 1.
    pub context: u32,
    /// `--ignore-all-space` (such diffs cannot be staged as patches).
    pub ignore_whitespace: bool,
    /// Detect renames (`--find-renames`).
    pub find_renames: bool,
    /// Limit to these repository-relative paths.
    pub paths: Vec<String>,
}

impl Default for DiffOptions {
    fn default() -> Self {
        DiffOptions { context: 3, ignore_whitespace: false, find_renames: true, paths: Vec::new() }
    }
}

/// How a file changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DiffStatus {
    Added,
    Deleted,
    Modified,
    Renamed,
    Copied,
}

/// Kind of a diff line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DiffLineKind {
    Context,
    Added,
    Removed,
}

/// One line of a hunk with its 1-based line numbers on each side.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    /// The line without its `+`/`-`/` ` prefix and newline (lossy UTF-8).
    pub text: String,
    pub old_line: Option<u32>,
    pub new_line: Option<u32>,
    /// Followed by `\ No newline at end of file`.
    pub no_newline: bool,
}

/// One `@@ -a,b +c,d @@` block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hunk {
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
    /// Text after the second `@@` (enclosing function or section).
    pub section: String,
    pub lines: Vec<DiffLine>,
}

/// The diff of one file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileDiff {
    /// Path before the change (`None` for added files).
    pub old_path: Option<String>,
    /// Path after the change (`None` for deleted files).
    pub new_path: Option<String>,
    pub status: DiffStatus,
    pub similarity: Option<u8>,
    pub old_mode: Option<String>,
    pub new_mode: Option<String>,
    pub binary: bool,
    /// Raw header lines (`diff --git` … `+++`), reused when building patches.
    pub header: Vec<String>,
    pub hunks: Vec<Hunk>,
}

impl FileDiff {
    /// The path to show: the new path, or the old one for deletions.
    pub fn path(&self) -> &str {
        self.new_path.as_deref().or(self.old_path.as_deref()).unwrap_or("")
    }

    /// The whole file diff as a patch.
    pub fn patch(&self) -> String {
        let mut out = self.header_text();
        for h in &self.hunks {
            write_hunk(&mut out, h.old_start, h.new_start, h.section.as_str(), h.lines.iter().map(|l| (l.kind, l)));
        }
        out
    }

    /// A patch with only hunk `hunk`, to apply to the side the diff was taken from (forward,
    /// `reverse = false`) or to the side it leads to (`reverse = true`, applied with `-R`).
    pub fn hunk_patch(&self, hunk: usize, reverse: bool) -> Option<String> {
        let h = self.hunks.get(hunk)?;
        let all: Vec<usize> = (0..h.lines.len()).collect();
        self.lines_patch(hunk, &all, reverse)
    }

    /// A patch with only the selected lines (indices into `hunks[hunk].lines`) of one hunk.
    /// Unselected changes are left out so they stay as they are: for a forward patch an
    /// unselected `-` becomes context and an unselected `+` is dropped; for a reverse patch
    /// (unstage / discard, applied with `-R`) an unselected `+` becomes context and an
    /// unselected `-` is dropped. `None` when nothing selected is a change.
    pub fn lines_patch(&self, hunk: usize, selected: &[usize], reverse: bool) -> Option<String> {
        let h = self.hunks.get(hunk)?;
        let mut lines = Vec::with_capacity(h.lines.len());
        let mut any = false;
        for (i, l) in h.lines.iter().enumerate() {
            let chosen = selected.contains(&i);
            let kind = match (l.kind, chosen, reverse) {
                (DiffLineKind::Context, _, _) => Some(DiffLineKind::Context),
                (k, true, _) => {
                    any = true;
                    Some(k)
                }
                (DiffLineKind::Removed, false, false) | (DiffLineKind::Added, false, true) => Some(DiffLineKind::Context),
                (DiffLineKind::Added, false, false) | (DiffLineKind::Removed, false, true) => None,
            };
            if let Some(k) = kind {
                lines.push((k, l));
            }
        }
        if !any {
            return None;
        }
        // The target only has this hunk's changes (forward) or all of them (reverse), so
        // both sides start where the target's text starts.
        let start = if reverse { h.new_start } else { h.old_start };
        let mut out = self.header_text();
        write_hunk(&mut out, start, start, &h.section, lines.into_iter());
        Some(out)
    }

    fn header_text(&self) -> String {
        let mut out = String::new();
        for l in &self.header {
            out.push_str(l);
            out.push('\n');
        }
        out
    }
}

fn write_hunk<'a>(out: &mut String, old_start: u32, new_start: u32, section: &str, lines: impl Iterator<Item = (DiffLineKind, &'a DiffLine)>) {
    let lines: Vec<(DiffLineKind, &DiffLine)> = lines.collect();
    let old_n = lines.iter().filter(|(k, _)| *k != DiffLineKind::Added).count() as u32;
    let new_n = lines.iter().filter(|(k, _)| *k != DiffLineKind::Removed).count() as u32;
    // A side with no lines is numbered after the line it follows.
    let os = if old_n == 0 { old_start.saturating_sub(1) } else { old_start.max(1) };
    let ns = if new_n == 0 { new_start.saturating_sub(1) } else { new_start.max(1) };
    out.push_str(&format!("@@ -{os},{old_n} +{ns},{new_n} @@"));
    if !section.is_empty() {
        out.push(' ');
        out.push_str(section);
    }
    out.push('\n');
    for (k, l) in lines {
        out.push(match k {
            DiffLineKind::Context => ' ',
            DiffLineKind::Added => '+',
            DiffLineKind::Removed => '-',
        });
        out.push_str(&l.text);
        out.push('\n');
        if l.no_newline {
            out.push_str("\\ No newline at end of file\n");
        }
    }
}

/// Parses `git diff` output (with `a/` and `b/` prefixes).
pub fn parse_diff(out: &[u8]) -> Result<Vec<FileDiff>> {
    let mut files: Vec<FileDiff> = Vec::new();
    let mut in_hunks = false;
    let (mut old_no, mut new_no) = (0u32, 0u32);
    let text = String::from_utf8_lossy(out);
    let mut lines: Vec<&str> = text.split('\n').collect();
    if lines.last() == Some(&"") {
        lines.pop();
    }
    for line in lines {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            let (a, b) = split_git_paths(rest);
            files.push(FileDiff {
                old_path: a,
                new_path: b,
                status: DiffStatus::Modified,
                similarity: None,
                old_mode: None,
                new_mode: None,
                binary: false,
                header: vec![line.to_string()],
                hunks: Vec::new(),
            });
            in_hunks = false;
            continue;
        }
        let Some(file) = files.last_mut() else { continue };
        if let Some(rest) = line.strip_prefix("@@ ") {
            let (old, new, section) = parse_hunk_header(rest).ok_or_else(|| GitError::Parse(format!("hunk header: {line}")))?;
            old_no = old.0;
            new_no = new.0;
            file.hunks.push(Hunk { old_start: old.0, old_lines: old.1, new_start: new.0, new_lines: new.1, section, lines: Vec::new() });
            in_hunks = true;
            continue;
        }
        if in_hunks {
            let hunk = file.hunks.last_mut().expect("hunk");
            let (kind, body) = match line.as_bytes().first() {
                Some(b'+') => (DiffLineKind::Added, &line[1..]),
                Some(b'-') => (DiffLineKind::Removed, &line[1..]),
                Some(b' ') => (DiffLineKind::Context, &line[1..]),
                Some(b'\\') => {
                    if let Some(last) = hunk.lines.last_mut() {
                        last.no_newline = true;
                    }
                    continue;
                }
                None => (DiffLineKind::Context, ""),
                _ => continue,
            };
            let (o, n) = match kind {
                DiffLineKind::Context => (Some(old_no), Some(new_no)),
                DiffLineKind::Added => (None, Some(new_no)),
                DiffLineKind::Removed => (Some(old_no), None),
            };
            if o.is_some() {
                old_no += 1;
            }
            if n.is_some() {
                new_no += 1;
            }
            hunk.lines.push(DiffLine { kind, text: body.to_string(), old_line: o, new_line: n, no_newline: false });
            continue;
        }
        file.header.push(line.to_string());
        if let Some(m) = line.strip_prefix("new file mode ") {
            file.status = DiffStatus::Added;
            file.new_mode = Some(m.to_string());
            file.old_path = None;
        } else if let Some(m) = line.strip_prefix("deleted file mode ") {
            file.status = DiffStatus::Deleted;
            file.old_mode = Some(m.to_string());
            file.new_path = None;
        } else if let Some(m) = line.strip_prefix("old mode ") {
            file.old_mode = Some(m.to_string());
        } else if let Some(m) = line.strip_prefix("new mode ") {
            file.new_mode = Some(m.to_string());
        } else if let Some(p) = line.strip_prefix("rename from ") {
            file.status = DiffStatus::Renamed;
            file.old_path = Some(unquote(p));
        } else if let Some(p) = line.strip_prefix("rename to ") {
            file.new_path = Some(unquote(p));
        } else if let Some(p) = line.strip_prefix("copy from ") {
            file.status = DiffStatus::Copied;
            file.old_path = Some(unquote(p));
        } else if let Some(p) = line.strip_prefix("copy to ") {
            file.new_path = Some(unquote(p));
        } else if let Some(s) = line.strip_prefix("similarity index ") {
            file.similarity = s.trim_end_matches('%').parse().ok();
        } else if let Some(p) = line.strip_prefix("--- ") {
            file.old_path = side_path(p, "a/");
        } else if let Some(p) = line.strip_prefix("+++ ") {
            file.new_path = side_path(p, "b/");
        } else if line.starts_with("Binary files ") || line == "GIT binary patch" {
            file.binary = true;
        } else if let Some(m) = line.strip_prefix("index ")
            && let Some((_, mode)) = m.split_once(' ')
        {
            file.old_mode.get_or_insert_with(|| mode.to_string());
            file.new_mode.get_or_insert_with(|| mode.to_string());
        }
    }
    Ok(files)
}

type HunkHeader = ((u32, u32), (u32, u32), String);

fn parse_hunk_header(rest: &str) -> Option<HunkHeader> {
    let (ranges, section) = rest.split_once(" @@").unwrap_or((rest, ""));
    let mut parts = ranges.split(' ');
    let range = |s: &str| -> Option<(u32, u32)> {
        let (start, len) = s.split_once(',').unwrap_or((s, "1"));
        Some((start.parse().ok()?, len.parse().ok()?))
    };
    let old = range(parts.next()?.strip_prefix('-')?)?;
    let new = range(parts.next()?.strip_prefix('+')?)?;
    Some((old, new, section.trim_start().to_string()))
}

fn side_path(p: &str, prefix: &str) -> Option<String> {
    let p = p.strip_suffix('\t').unwrap_or(p);
    if p == "/dev/null" {
        return None;
    }
    let p = unquote(p);
    Some(p.strip_prefix(prefix).map(str::to_string).unwrap_or(p))
}

/// Splits the paths of a `diff --git a/x b/x` line (used when there is no `---`/`+++`).
fn split_git_paths(rest: &str) -> (Option<String>, Option<String>) {
    if rest.starts_with('"') {
        let end = quoted_end(rest);
        let a = unquote(&rest[..end]);
        let b = unquote(rest[end..].trim_start());
        return (a.strip_prefix("a/").map(str::to_string), b.strip_prefix("b/").map(str::to_string));
    }
    if let Some(bq) = rest.find(" \"b/") {
        let a = &rest[..bq];
        return (a.strip_prefix("a/").map(str::to_string), unquote(&rest[bq + 1..]).strip_prefix("b/").map(str::to_string));
    }
    // Unquoted "a/X b/X": both halves are the same path when there is no rename.
    if rest.len() >= 5 && (rest.len() - 5).is_multiple_of(2) {
        let n = (rest.len() - 5) / 2;
        if rest.is_char_boundary(2 + n) && rest[2..2 + n] == rest[n + 5..] && rest.starts_with("a/") {
            let p = rest[2..2 + n].to_string();
            return (Some(p.clone()), Some(p));
        }
    }
    match rest.split_once(" b/") {
        Some((a, b)) => (Some(a.trim_start_matches("a/").to_string()), Some(b.to_string())),
        None => (None, None),
    }
}

fn quoted_end(s: &str) -> usize {
    let bytes = s.as_bytes();
    let mut i = 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'"' => return i + 1,
            _ => i += 1,
        }
    }
    bytes.len()
}

/// Undoes git's C-style path quoting (`"a/caf\303\251 \"x\""`).
pub fn unquote(s: &str) -> String {
    let Some(inner) = s.strip_prefix('"').and_then(|s| s.strip_suffix('"')) else { return s.to_string() };
    let mut out = Vec::new();
    let b = inner.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 1 < b.len() {
            i += 1;
            match b[i] {
                b'n' => out.push(b'\n'),
                b't' => out.push(b'\t'),
                b'r' => out.push(b'\r'),
                b'a' => out.push(7),
                b'b' => out.push(8),
                b'f' => out.push(12),
                b'v' => out.push(11),
                b'0'..=b'7' => {
                    let oct = std::str::from_utf8(&b[i..(i + 3).min(b.len())]).ok().and_then(|o| u8::from_str_radix(o, 8).ok());
                    match oct {
                        Some(v) => {
                            out.push(v);
                            i += 2;
                        }
                        None => out.push(b[i]),
                    }
                }
                c => out.push(c),
            }
        } else {
            out.push(b[i]);
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Where a file's content is read from.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Revision {
    Head,
    /// The staged version (stage 0).
    Index,
    /// A merge stage: 1 = base, 2 = ours, 3 = theirs.
    Stage(u8),
    /// Any commit-ish (hash, branch, tag, `HEAD~2`…).
    Commit(String),
}

impl Revision {
    fn object(&self, path: &str) -> String {
        match self {
            Revision::Head => format!("HEAD:{path}"),
            Revision::Index => format!(":0:{path}"),
            Revision::Stage(n) => format!(":{n}:{path}"),
            Revision::Commit(c) => format!("{c}:{path}"),
        }
    }
}

/// A file's bytes at some revision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileContent {
    pub bytes: Vec<u8>,
    /// Contains a NUL byte in its first 8,000 bytes (git's heuristic).
    pub binary: bool,
}

impl FileContent {
    pub fn new(bytes: Vec<u8>) -> FileContent {
        let binary = bytes[..bytes.len().min(8000)].contains(&0);
        FileContent { bytes, binary }
    }

    /// The content as text (invalid UTF-8 replaced).
    pub fn text(&self) -> Cow<'_, str> {
        String::from_utf8_lossy(&self.bytes)
    }
}

impl Repository {
    /// The diff of `spec`, parsed. Untracked files are not part of `WorktreeVsIndex`; see
    /// [`Repository::diff_untracked`].
    pub async fn diff(&self, spec: &DiffSpec, opts: &DiffOptions) -> Result<Vec<FileDiff>> {
        let mut args: Vec<String> = Vec::new();
        let common = |args: &mut Vec<String>| {
            args.extend(["--no-color", "--no-ext-diff", "--no-textconv", "--src-prefix=a/", "--dst-prefix=b/", "--submodule=short"].map(String::from));
            args.push(format!("-U{}", opts.context));
            if opts.find_renames {
                args.push("--find-renames".into());
            }
            if opts.ignore_whitespace {
                args.push("--ignore-all-space".into());
            }
        };
        match spec {
            DiffSpec::Commit(c) => {
                let parents = self.cmd().args(["rev-list", "--parents", "-n", "1", "--end-of-options", c]).text().await?;
                let first_parent = parents.split_whitespace().nth(1).map(str::to_string);
                match first_parent {
                    Some(p) => {
                        args.push("diff".into());
                        common(&mut args);
                        args.extend(["--end-of-options".to_string(), p, c.clone()]);
                    }
                    None => {
                        args.extend(["diff-tree", "-p", "-r", "--root", "--no-commit-id"].map(String::from));
                        common(&mut args);
                        args.extend(["--end-of-options".to_string(), c.clone()]);
                    }
                }
            }
            _ => {
                args.push("diff".into());
                common(&mut args);
                match spec {
                    DiffSpec::WorktreeVsIndex => {}
                    DiffSpec::IndexVsHead => args.push("--cached".into()),
                    DiffSpec::WorktreeVsHead => {
                        // An unborn branch has no HEAD: everything staged is new.
                        if self.resolve("HEAD").await.is_ok() {
                            args.push("HEAD".into());
                        } else {
                            args.push("--cached".into());
                        }
                    }
                    DiffSpec::Revisions { from, to } => args.extend(["--end-of-options".to_string(), from.clone(), to.clone()]),
                    DiffSpec::Commit(_) => unreachable!(),
                }
            }
        }
        args.push("--".into());
        args.extend(opts.paths.iter().cloned());
        let out = self.cmd().args(&args).output().await?;
        parse_diff(&out.stdout)
    }

    /// Diff of an untracked file against nothing (everything added).
    pub async fn diff_untracked(&self, path: &str, context: u32) -> Result<Option<FileDiff>> {
        let out = self
            .cmd()
            .args(["diff", "--no-color", "--no-ext-diff", "--no-index", "--src-prefix=a/", "--dst-prefix=b/"])
            .arg(format!("-U{context}"))
            .args(["--", "/dev/null", path])
            .ok_code(1)
            .output()
            .await?;
        Ok(parse_diff(&out.stdout)?.into_iter().next())
    }

    /// The content of `path` at `rev`, with checkout filters applied (line endings as in the
    /// work tree). `None` when the path does not exist there (new file, unborn branch).
    pub async fn show_file(&self, rev: &Revision, path: &str) -> Result<Option<FileContent>> {
        match self.cmd().args(["cat-file", "--filters"]).arg(rev.object(path)).output().await {
            Ok(out) => Ok(Some(FileContent::new(out.stdout))),
            Err(GitError::NotFound(_)) => Ok(None),
            Err(GitError::Failed { stderr, .. }) if stderr.contains("Not a valid object name") || stderr.contains("does not exist") => Ok(None),
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "diff --git a/src/a.cs b/src/a.cs
index 1111111..2222222 100644
--- a/src/a.cs
+++ b/src/a.cs
@@ -1,4 +1,4 @@ class A
 one
-two
+TWO
 three
 four
@@ -10,2 +10,3 @@
 ten
+ten and a half
 eleven
\\ No newline at end of file
diff --git a/old name.txt b/new name.txt
similarity index 90%
rename from old name.txt
rename to new name.txt
diff --git a/img.png b/img.png
new file mode 100644
index 0000000..3333333
Binary files /dev/null and b/img.png differ
diff --git \"a/caf\\303\\251.txt\" \"b/caf\\303\\251.txt\"
deleted file mode 100644
index 4444444..0000000
--- \"a/caf\\303\\251.txt\"
+++ /dev/null
@@ -1 +0,0 @@
-x
";

    #[test]
    fn parses_files_hunks_and_lines() {
        let files = parse_diff(SAMPLE.as_bytes()).unwrap();
        assert_eq!(files.len(), 4);
        let a = &files[0];
        assert_eq!(a.path(), "src/a.cs");
        assert_eq!(a.status, DiffStatus::Modified);
        assert_eq!(a.hunks.len(), 2);
        assert_eq!(a.hunks[0].section, "class A");
        assert_eq!(a.hunks[0].lines[1], DiffLine { kind: DiffLineKind::Removed, text: "two".into(), old_line: Some(2), new_line: None, no_newline: false });
        assert_eq!(a.hunks[0].lines[2].new_line, Some(2));
        assert_eq!(a.hunks[1].lines[1].new_line, Some(11));
        assert!(a.hunks[1].lines[2].no_newline);
        assert_eq!(a.patch().lines().count(), SAMPLE.lines().take_while(|l| !l.starts_with("diff --git a/old")).count());
        let r = &files[1];
        assert_eq!((r.status, r.old_path.as_deref(), r.new_path.as_deref(), r.similarity), (DiffStatus::Renamed, Some("old name.txt"), Some("new name.txt"), Some(90)));
        assert!(files[2].binary);
        assert_eq!((files[2].status, files[2].path()), (DiffStatus::Added, "img.png"));
        assert_eq!((files[3].status, files[3].path()), (DiffStatus::Deleted, "café.txt"));
    }

    #[test]
    fn partial_patches() {
        let files = parse_diff(SAMPLE.as_bytes()).unwrap();
        let a = &files[0];
        let p = a.hunk_patch(1, false).unwrap();
        assert!(p.contains("@@ -10,2 +10,3 @@\n ten\n+ten and a half\n eleven\n\\ No newline at end of file\n"), "{p}");
        // Select only the "+TWO" line: "-two" becomes context.
        let p = a.lines_patch(0, &[2], false).unwrap();
        assert!(p.ends_with("@@ -1,4 +1,5 @@ class A\n one\n two\n+TWO\n three\n four\n"), "{p}");
        // Reverse: only "-two"; the unselected "+TWO" stays as context.
        let p = a.lines_patch(0, &[1], true).unwrap();
        assert!(p.ends_with("@@ -1,5 +1,4 @@ class A\n one\n-two\n TWO\n three\n four\n"), "{p}");
        assert!(a.lines_patch(0, &[0], false).is_none());
    }

    #[test]
    fn unquotes_paths() {
        assert_eq!(unquote("\"a/caf\\303\\251 \\\"x\\\"\""), "a/café \"x\"");
        assert_eq!(unquote("plain"), "plain");
        assert_eq!(split_git_paths("a/x y b/x y"), (Some("x y".into()), Some("x y".into())));
    }
}
