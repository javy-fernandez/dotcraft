//! Document synchronisation and conversions between editor coordinates and LSP's.

use lsp_types::{Position, Range, TextDocumentContentChangeEvent, Uri};
use percent_encoding::{AsciiSet, CONTROLS, percent_decode_str, utf8_percent_encode};
use rc_text::{Edit, Utf16Pos};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;

/// Characters escaped in a `file://` URI path (like VS Code: keep `/` and `:` readable).
const PATH: &AsciiSet = &CONTROLS.add(b' ').add(b'"').add(b'#').add(b'%').add(b'<').add(b'>').add(b'?').add(b'`').add(b'{').add(b'}').add(b'[').add(b']').add(b'^').add(b'|');

/// `file:///Users/me/A B.cs` from a path (Windows: `file:///C:/x.cs`).
pub fn path_to_uri(path: &Path) -> Uri {
    let s = path.to_string_lossy().replace('\\', "/");
    let s = if s.starts_with('/') { s } else { format!("/{s}") };
    let encoded = utf8_percent_encode(&s, PATH).to_string();
    Uri::from_str(&format!("file://{encoded}")).unwrap_or_else(|_| Uri::from_str("file:///").expect("valid"))
}

pub fn uri_to_path(uri: &Uri) -> Option<PathBuf> {
    let s = uri.as_str();
    let rest = s.strip_prefix("file://")?;
    let decoded = percent_decode_str(rest).decode_utf8().ok()?.into_owned();
    // `file:///C:/x` → `C:/x` on Windows.
    if cfg!(windows) && decoded.len() > 2 && decoded.as_bytes()[2] == b':' {
        return Some(PathBuf::from(&decoded[1..]));
    }
    Some(PathBuf::from(decoded))
}

pub fn lsp_pos(p: Utf16Pos) -> Position {
    Position { line: p.line, character: p.character }
}

pub fn utf16_pos(p: Position) -> Utf16Pos {
    Utf16Pos { line: p.line, character: p.character }
}

/// Incremental content changes for edits already applied to the buffer, in order.
pub fn changes_for(edits: &[Edit]) -> Vec<TextDocumentContentChangeEvent> {
    edits
        .iter()
        .map(|e| TextDocumentContentChangeEvent {
            range: Some(Range { start: lsp_pos(e.lsp_start), end: lsp_pos(e.lsp_old_end) }),
            range_length: None,
            text: e.text.clone(),
        })
        .collect()
}

/// Versions of the documents open in the server.
#[derive(Default)]
pub struct OpenDocs {
    versions: HashMap<PathBuf, i32>,
}

impl OpenDocs {
    pub fn is_open(&self, path: &Path) -> bool {
        self.versions.contains_key(path)
    }

    /// Marks `path` open; returns its first version.
    pub fn open(&mut self, path: &Path) -> i32 {
        self.versions.insert(path.to_path_buf(), 1);
        1
    }

    /// Bumps and returns the version for a `didChange`.
    pub fn bump(&mut self, path: &Path) -> Option<i32> {
        let v = self.versions.get_mut(path)?;
        *v += 1;
        Some(*v)
    }

    pub fn version(&self, path: &Path) -> Option<i32> {
        self.versions.get(path).copied()
    }

    pub fn close(&mut self, path: &Path) -> bool {
        self.versions.remove(path).is_some()
    }

    pub fn paths(&self) -> Vec<PathBuf> {
        self.versions.keys().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uris_round_trip_with_spaces_and_unicode() {
        let p = Path::new("/Users/me/My Projects/Café/A#1.cs");
        let u = path_to_uri(p);
        assert_eq!(u.as_str(), "file:///Users/me/My%20Projects/Caf%C3%A9/A%231.cs");
        assert_eq!(uri_to_path(&u).unwrap(), p);
    }

    #[test]
    fn edits_become_ranged_changes() {
        let mut b = rc_text::Buffer::new("ab\ncd");
        let cs = rc_text::ChangeSet::new(vec![rc_text::Change::new(1, 1, "X"), rc_text::Change::new(4, 5, "")]);
        let edits = b.apply(&cs, &rc_text::Selection::point(0), &rc_text::Selection::point(0), rc_text::EditKind::Other);
        let ch = changes_for(&edits);
        // Applied last-first: the deletion on line 1, then the insertion on line 0.
        assert_eq!(ch[0].range.unwrap().start, Position { line: 1, character: 1 });
        assert_eq!(ch[1].range.unwrap().start, Position { line: 0, character: 1 });
        assert_eq!(ch[1].text, "X");
    }
}
