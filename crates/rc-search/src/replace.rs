//! Replace in Files: replacements for every match of a [`Query`] (with `$1`-style groups in
//! regex mode), either written to disk atomically or returned as [`TextEdit`]s for open
//! buffers so the editor applies them as one undoable change. Line breaks inserted by the
//! replacement follow the text's line endings; a UTF-8 BOM is kept.

use crate::query::{CharOffsets, Query};
use std::fmt;
use std::io::Write;
use std::ops::Range;
use std::path::{Path, PathBuf};

const BOM: &[u8] = b"\xEF\xBB\xBF";

/// Replace the chars in `range` with `text`. Edits come sorted and never overlap; apply them
/// from last to first (or shift offsets) so earlier ranges stay valid.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextEdit {
    pub range: Range<usize>,
    pub text: String,
}

/// Where a file's replacements go.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReplaceTarget {
    /// Rewrite the file on disk.
    Disk,
    /// The file is open: compute edits on this buffer text instead.
    Buffer(String),
}

/// What happened to one file.
#[derive(Debug)]
pub enum ReplaceOutcome {
    /// The file was rewritten with this many replacements (0: untouched).
    Written(usize),
    /// Edits for the open buffer.
    Edits(Vec<TextEdit>),
    Failed(ReplaceError),
}

#[derive(Debug)]
pub enum ReplaceError {
    Io(std::io::Error),
    /// The file is not valid UTF-8 (it is left untouched).
    NotUtf8,
}

impl fmt::Display for ReplaceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReplaceError::Io(e) => e.fmt(f),
            ReplaceError::NotUtf8 => f.write_str("file is not valid UTF-8"),
        }
    }
}

impl std::error::Error for ReplaceError {}

impl From<std::io::Error> for ReplaceError {
    fn from(e: std::io::Error) -> Self {
        ReplaceError::Io(e)
    }
}

/// Every match of `query` in `text` with its replacement (byte ranges).
fn replacements(text: &str, query: &Query, replacement: &str) -> Vec<(Range<usize>, String)> {
    let crlf = text.find('\n').is_some_and(|i| i > 0 && text.as_bytes()[i - 1] == b'\r');
    let hay = text.as_bytes();
    let mut out = Vec::new();
    let mut at = 0;
    let mut buf = Vec::new();
    while let Some(caps) = query.captures_at(hay, at) {
        let Some(m) = caps.get(0) else { break };
        buf.clear();
        query.expand(&caps, replacement, &mut buf);
        let mut s = String::from_utf8_lossy(&buf).into_owned();
        if crlf && s.contains('\n') {
            s = s.replace("\r\n", "\n").replace('\n', "\r\n");
        }
        out.push((m.range(), s));
        at = m.end();
    }
    out
}

/// Edits (char ranges) replacing every match in `text`.
pub fn compute_edits(text: &str, query: &Query, replacement: &str) -> Vec<TextEdit> {
    let mut conv = CharOffsets::new(text);
    replacements(text, query, replacement)
        .into_iter()
        .map(|(r, s)| TextEdit { range: conv.at(r.start)..conv.at(r.end), text: s })
        .collect()
}

/// `text` with every match replaced, and the number of replacements.
pub fn replace_text(text: &str, query: &Query, replacement: &str) -> (String, usize) {
    let reps = replacements(text, query, replacement);
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for (r, s) in &reps {
        out.push_str(&text[last..r.start]);
        out.push_str(s);
        last = r.end;
    }
    out.push_str(&text[last..]);
    (out, reps.len())
}

/// Replaces every match in the file and writes it back atomically (temporary file in the
/// same folder, then rename), keeping its permissions and BOM. Returns the number of
/// replacements; the file is not touched when there are none.
pub fn replace_in_file(path: &Path, query: &Query, replacement: &str) -> Result<usize, ReplaceError> {
    let bytes = std::fs::read(path)?;
    let (bom, body) = match bytes.strip_prefix(BOM) {
        Some(rest) => (true, rest),
        None => (false, &bytes[..]),
    };
    let text = std::str::from_utf8(body).map_err(|_| ReplaceError::NotUtf8)?;
    let (new_text, count) = replace_text(text, query, replacement);
    if count == 0 || new_text == text {
        return Ok(count);
    }
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let mut tmp = tempfile::Builder::new().prefix(".dotcraft-replace").tempfile_in(dir)?;
    if bom {
        tmp.write_all(BOM)?;
    }
    tmp.write_all(new_text.as_bytes())?;
    tmp.as_file().sync_all()?;
    if let Ok(meta) = std::fs::metadata(path) {
        let _ = tmp.as_file().set_permissions(meta.permissions());
    }
    tmp.persist(path).map_err(|e| ReplaceError::Io(e.error))?;
    Ok(count)
}

/// Replaces in many files, choosing per file between disk and an open buffer.
pub fn replace_in_files(items: impl IntoIterator<Item = (PathBuf, ReplaceTarget)>, query: &Query, replacement: &str) -> Vec<(PathBuf, ReplaceOutcome)> {
    items
        .into_iter()
        .map(|(path, target)| {
            let outcome = match target {
                ReplaceTarget::Buffer(text) => ReplaceOutcome::Edits(compute_edits(&text, query, replacement)),
                ReplaceTarget::Disk => match replace_in_file(&path, query, replacement) {
                    Ok(n) => ReplaceOutcome::Written(n),
                    Err(e) => ReplaceOutcome::Failed(e),
                },
            };
            (path, outcome)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::QueryOptions;

    fn regex(p: &str) -> Query {
        Query::new(QueryOptions { pattern: p.into(), regex: true, case_sensitive: true, whole_word: false }).unwrap()
    }

    #[test]
    fn groups_expand_in_regex_mode() {
        let q = regex(r"(\w+)\.Get(\w+)\(\)");
        let (out, n) = replace_text("a.GetName(); b.GetAge();", &q, "$1.$2");
        assert_eq!((out.as_str(), n), ("a.Name; b.Age;", 2));
        let q = regex(r"(?P<x>\d+)");
        assert_eq!(replace_text("v1 v22", &q, "<${x}>$$").0, "v<1>$ v<22>$");
    }

    #[test]
    fn plain_mode_inserts_replacement_literally() {
        let q = Query::new(QueryOptions::text("cost")).unwrap();
        assert_eq!(replace_text("Cost cost", &q, "$1").0, "$1 $1");
    }

    #[test]
    fn edits_use_char_offsets() {
        let q = Query::new(QueryOptions::text("é")).unwrap();
        let edits = compute_edits("ñé ñé", &q, "e");
        assert_eq!(edits, [TextEdit { range: 1..2, text: "e".into() }, TextEdit { range: 4..5, text: "e".into() }]);
    }

    #[test]
    fn inserted_line_breaks_follow_crlf() {
        let q = regex(r";");
        assert_eq!(replace_text("a;b\r\nc", &q, r";\n").0, "a;\r\nb\r\nc");
        assert_eq!(replace_text("a;b\nc", &q, r";\n").0, "a;\nb\nc");
    }
}
