//! The text buffer: a rope with a growing version, grouped undo/redo, line endings and the
//! edit records that incremental parsers (tree-sitter) and language servers (LSP) consume.

use crate::change::{Change, ChangeSet};
use crate::selection::Selection;
use ropey::Rope;
use std::time::{Duration, Instant};

/// Line ending written back to disk. In memory the text always uses `\n`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum LineEnding {
    #[default]
    Lf,
    CrLf,
}

impl LineEnding {
    pub fn as_str(self) -> &'static str {
        match self {
            LineEnding::Lf => "\n",
            LineEnding::CrLf => "\r\n",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            LineEnding::Lf => "LF",
            LineEnding::CrLf => "CRLF",
        }
    }

    /// The majority ending of `text` (LF when there are no line breaks).
    pub fn detect(text: &str) -> LineEnding {
        let lf = text.bytes().filter(|&b| b == b'\n').count();
        let crlf = text.matches("\r\n").count();
        if crlf * 2 > lf { LineEnding::CrLf } else { LineEnding::Lf }
    }
}

/// A position as LSP counts it: zero-based line and UTF-16 code units within the line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Utf16Pos {
    pub line: u32,
    pub character: u32,
}

/// A tree-sitter style point: zero-based row and *byte* column.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct BytePoint {
    pub row: usize,
    pub column: usize,
}

/// One applied replacement, described in every coordinate system a consumer may need. The
/// edits of one change set are listed in the order they were applied (last offset first), so
/// each one is valid against the document left by the previous one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    pub start_char: usize,
    pub old_end_char: usize,
    pub new_end_char: usize,
    pub start_byte: usize,
    pub old_end_byte: usize,
    pub new_end_byte: usize,
    pub start_point: BytePoint,
    pub old_end_point: BytePoint,
    pub new_end_point: BytePoint,
    /// LSP range of the replaced text (before the edit).
    pub lsp_start: Utf16Pos,
    pub lsp_old_end: Utf16Pos,
    pub text: String,
}

/// What kind of typing produced a transaction, so consecutive keystrokes of the same kind
/// undo together.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditKind {
    /// Typed characters (letters, digits, punctuation).
    Insert,
    /// Backspace / delete of single characters.
    Delete,
    /// Anything else: paste, line operations, refactorings… always its own undo step.
    Other,
}

#[derive(Clone, Debug)]
struct Revision {
    /// Forward change sets, applied in order.
    forward: Vec<ChangeSet>,
    /// Inverse change sets, in the same order (undo applies them in reverse).
    inverse: Vec<ChangeSet>,
    sel_before: Selection,
    sel_after: Selection,
    kind: EditKind,
    at: Instant,
    /// Set when the caret moved away: the next keystroke starts a new step.
    sealed: bool,
}

/// Keystrokes closer than this join the previous undo step.
const GROUP_WINDOW: Duration = Duration::from_millis(1200);

#[derive(Clone, Debug)]
pub struct Buffer {
    rope: Rope,
    version: u64,
    line_ending: LineEnding,
    undo: Vec<Revision>,
    redo: Vec<Revision>,
    /// Version at the last save (or load), to tell whether the buffer is modified.
    saved_version: u64,
    /// Version history position: bumps on every change, undo and redo included.
    saved_undo_depth: Option<usize>,
}

impl Default for Buffer {
    fn default() -> Self {
        Buffer::new("")
    }
}

impl Buffer {
    /// A buffer for `text`; `\r\n` and lone `\r` become `\n` and the majority ending is kept.
    pub fn new(text: &str) -> Self {
        let line_ending = LineEnding::detect(text);
        let normalized = if text.contains('\r') { text.replace("\r\n", "\n").replace('\r', "\n") } else { text.to_owned() };
        Buffer {
            rope: Rope::from_str(&normalized),
            version: 0,
            line_ending,
            undo: Vec::new(),
            redo: Vec::new(),
            saved_version: 0,
            saved_undo_depth: Some(0),
        }
    }

    pub fn rope(&self) -> &Rope {
        &self.rope
    }

    /// A cheap (O(1)) immutable copy for background work.
    pub fn snapshot(&self) -> Rope {
        self.rope.clone()
    }

    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn len_chars(&self) -> usize {
        self.rope.len_chars()
    }

    pub fn len_lines(&self) -> usize {
        self.rope.len_lines()
    }

    pub fn line_ending(&self) -> LineEnding {
        self.line_ending
    }

    pub fn set_line_ending(&mut self, le: LineEnding) {
        self.line_ending = le;
        self.saved_undo_depth = None;
    }

    /// The text with the file's line endings, ready to write to disk.
    pub fn to_disk_string(&self) -> String {
        let s = self.rope.to_string();
        match self.line_ending {
            LineEnding::Lf => s,
            LineEnding::CrLf => s.replace('\n', "\r\n"),
        }
    }

    pub fn is_modified(&self) -> bool {
        self.saved_undo_depth != Some(self.undo.len())
    }

    pub fn mark_saved(&mut self) {
        self.saved_version = self.version;
        self.saved_undo_depth = Some(self.undo.len());
    }

    pub fn saved_version(&self) -> u64 {
        self.saved_version
    }

    /// Text of line `line` without its trailing `\n`.
    pub fn line_text(&self, line: usize) -> String {
        let mut s = self.rope.line(line).to_string();
        if s.ends_with('\n') {
            s.pop();
        }
        s
    }

    /// Number of chars of `line` without the line break.
    pub fn line_len(&self, line: usize) -> usize {
        line_len(&self.rope, line)
    }

    // ----------------------------------------------------------------- applying changes

    /// Applies `changes` as one step of kind `kind` and returns the edits made. Keystrokes of
    /// the same kind that follow each other quickly join the previous undo step.
    pub fn apply(&mut self, changes: &ChangeSet, sel_before: &Selection, sel_after: &Selection, kind: EditKind) -> Vec<Edit> {
        if changes.is_empty() {
            return Vec::new();
        }
        let inverse = changes.invert(&self.rope);
        let edits = self.apply_raw(changes);
        self.redo.clear();
        if self.saved_undo_depth.is_some_and(|d| d > self.undo.len()) {
            self.saved_undo_depth = None;
        }
        let now = Instant::now();
        let joins = kind != EditKind::Other
            && self.undo.last().is_some_and(|r| {
                r.kind == kind && !r.sealed && now.duration_since(r.at) < GROUP_WINDOW && r.sel_after == *sel_before
            })
            // A line break typed after a word starts a new step, like IntelliJ.
            && !(kind == EditKind::Insert && changes.changes().iter().any(|c| c.text.contains('\n')))
            && self.saved_undo_depth != Some(self.undo.len());
        if joins {
            let r = self.undo.last_mut().expect("checked above");
            r.forward.push(changes.clone());
            r.inverse.push(inverse);
            r.sel_after = sel_after.clone();
            r.at = now;
        } else {
            self.undo.push(Revision {
                forward: vec![changes.clone()],
                inverse: vec![inverse],
                sel_before: sel_before.clone(),
                sel_after: sel_after.clone(),
                kind,
                at: now,
                sealed: false,
            });
        }
        edits
    }

    /// Ends the current undo step (the caret moved, the editor lost focus…).
    pub fn seal_undo(&mut self) {
        if let Some(r) = self.undo.last_mut() {
            r.sealed = true;
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Undoes the last step: returns the edits made and the selection to restore.
    pub fn undo(&mut self) -> Option<(Vec<Edit>, Selection)> {
        let mut rev = self.undo.pop()?;
        rev.sealed = true;
        let mut edits = Vec::new();
        for cs in rev.inverse.iter().rev() {
            edits.extend(self.apply_raw(cs));
        }
        let sel = rev.sel_before.clone().clamp(self.rope.len_chars());
        self.redo.push(rev);
        Some((edits, sel))
    }

    pub fn redo(&mut self) -> Option<(Vec<Edit>, Selection)> {
        let rev = self.redo.pop()?;
        let mut edits = Vec::new();
        for cs in &rev.forward {
            edits.extend(self.apply_raw(cs));
        }
        let sel = rev.sel_after.clone().clamp(self.rope.len_chars());
        self.undo.push(rev);
        Some((edits, sel))
    }

    /// Replaces the whole text (reload from disk) as an undoable step.
    pub fn replace_all(&mut self, text: &str, sel: &Selection) -> Vec<Edit> {
        let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
        let cs = ChangeSet::single(Change::new(0, self.rope.len_chars(), normalized));
        let after = Selection::point(0);
        self.apply(&cs, sel, &after, EditKind::Other)
    }

    fn apply_raw(&mut self, changes: &ChangeSet) -> Vec<Edit> {
        let mut edits = Vec::with_capacity(changes.changes().len());
        for c in changes.changes().iter().rev() {
            let rope = &self.rope;
            let start_byte = rope.char_to_byte(c.start);
            let old_end_byte = rope.char_to_byte(c.end);
            let start_point = byte_point(rope, c.start);
            let old_end_point = byte_point(rope, c.end);
            let lsp_start = char_to_utf16(rope, c.start);
            let lsp_old_end = char_to_utf16(rope, c.end);
            if c.end > c.start {
                self.rope.remove(c.start..c.end);
            }
            if !c.text.is_empty() {
                self.rope.insert(c.start, &c.text);
            }
            let new_end_char = c.start + c.text.chars().count();
            edits.push(Edit {
                start_char: c.start,
                old_end_char: c.end,
                new_end_char,
                start_byte,
                old_end_byte,
                new_end_byte: start_byte + c.text.len(),
                start_point,
                old_end_point,
                new_end_point: byte_point(&self.rope, new_end_char),
                lsp_start,
                lsp_old_end,
                text: c.text.clone(),
            });
        }
        self.version += 1;
        edits
    }

    // ----------------------------------------------------------------- coordinates

    pub fn char_to_utf16(&self, pos: usize) -> Utf16Pos {
        char_to_utf16(&self.rope, pos)
    }

    pub fn utf16_to_char(&self, pos: Utf16Pos) -> usize {
        utf16_to_char(&self.rope, pos)
    }

    /// (line, column in chars) of a char offset.
    pub fn line_col(&self, pos: usize) -> (usize, usize) {
        let pos = pos.min(self.rope.len_chars());
        let line = self.rope.char_to_line(pos);
        (line, pos - self.rope.line_to_char(line))
    }

    /// Char offset of (line, column), clamped to the document and to the line.
    pub fn pos_at(&self, line: usize, col: usize) -> usize {
        let line = line.min(self.rope.len_lines().saturating_sub(1));
        self.rope.line_to_char(line) + col.min(line_len(&self.rope, line))
    }
}

/// Chars of `line` without the line break.
pub fn line_len(rope: &Rope, line: usize) -> usize {
    let slice = rope.line(line);
    let n = slice.len_chars();
    if n > 0 && slice.char(n - 1) == '\n' { n - 1 } else { n }
}

pub fn byte_point(rope: &Rope, pos: usize) -> BytePoint {
    let row = rope.char_to_line(pos);
    BytePoint { row, column: rope.char_to_byte(pos) - rope.line_to_byte(row) }
}

pub fn char_to_utf16(rope: &Rope, pos: usize) -> Utf16Pos {
    let pos = pos.min(rope.len_chars());
    let line = rope.char_to_line(pos);
    let line_start = rope.line_to_char(line);
    let character = rope.char_to_utf16_cu(pos) - rope.char_to_utf16_cu(line_start);
    Utf16Pos { line: line as u32, character: character as u32 }
}

/// Inverse of [`char_to_utf16`]; positions past the end of a line clamp to it, and a column
/// that falls in the middle of a surrogate pair rounds down.
pub fn utf16_to_char(rope: &Rope, pos: Utf16Pos) -> usize {
    let line = (pos.line as usize).min(rope.len_lines().saturating_sub(1));
    let line_start = rope.line_to_char(line);
    let base = rope.char_to_utf16_cu(line_start);
    let line_end = line_start + line_len(rope, line);
    let max_cu = rope.char_to_utf16_cu(line_end) - base;
    let cu = (pos.character as usize).min(max_cu);
    rope.utf16_cu_to_char(base + cu)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn type_at(buf: &mut Buffer, sel: &mut Selection, text: &str, kind: EditKind) {
        let pos = sel.primary().head;
        let cs = ChangeSet::single(Change::insert(pos, text));
        let after = sel.map(&cs);
        buf.apply(&cs, sel, &after, kind);
        *sel = after;
    }

    #[test]
    fn line_endings_are_normalized_and_restored() {
        let b = Buffer::new("a\r\nb\r\nc");
        assert_eq!(b.rope().to_string(), "a\nb\nc");
        assert_eq!(b.line_ending(), LineEnding::CrLf);
        assert_eq!(b.to_disk_string(), "a\r\nb\r\nc");
    }

    #[test]
    fn typing_groups_into_one_undo_step_until_a_newline() {
        let mut b = Buffer::new("");
        let mut sel = Selection::point(0);
        for ch in ["f", "o", "o"] {
            type_at(&mut b, &mut sel, ch, EditKind::Insert);
        }
        type_at(&mut b, &mut sel, "\n", EditKind::Insert);
        type_at(&mut b, &mut sel, "b", EditKind::Insert);
        assert_eq!(b.rope().to_string(), "foo\nb");
        let (_, s) = b.undo().unwrap();
        assert_eq!(b.rope().to_string(), "foo");
        assert_eq!(s.primary().head, 3);
        b.undo().unwrap();
        assert_eq!(b.rope().to_string(), "");
        assert!(!b.is_modified());
        b.redo().unwrap();
        b.redo().unwrap();
        assert_eq!(b.rope().to_string(), "foo\nb");
    }

    #[test]
    fn utf16_positions_count_surrogate_pairs() {
        let b = Buffer::new("a😀b\nñx");
        assert_eq!(b.char_to_utf16(2), Utf16Pos { line: 0, character: 3 });
        assert_eq!(b.utf16_to_char(Utf16Pos { line: 0, character: 3 }), 2);
        assert_eq!(b.char_to_utf16(5), Utf16Pos { line: 1, character: 1 });
        assert_eq!(b.utf16_to_char(Utf16Pos { line: 1, character: 99 }), 6);
    }

    #[test]
    fn edits_report_bytes_and_points_for_tree_sitter() {
        let mut b = Buffer::new("ña\nb");
        let cs = ChangeSet::single(Change::new(1, 2, "xyz"));
        let edits = b.apply(&cs, &Selection::point(0), &Selection::point(0), EditKind::Other);
        let e = &edits[0];
        assert_eq!((e.start_byte, e.old_end_byte, e.new_end_byte), (2, 3, 5));
        assert_eq!(e.new_end_point, BytePoint { row: 0, column: 5 });
        assert_eq!(b.rope().to_string(), "ñxyz\nb");
    }
}
