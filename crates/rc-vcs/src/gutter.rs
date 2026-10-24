//! Changed-line markers for the editor gutter: the HEAD (or index) content against the
//! in-memory buffer, computed in-process so it can rerun on every edit.

use crate::textdiff;
use std::ops::Range;

/// What happened to a block of lines of the buffer compared with the base text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LineChangeKind {
    /// Lines that do not exist in the base.
    Added,
    /// Lines that replace other base lines.
    Modified,
    /// Base lines removed before buffer line `new.start` (`new` is empty).
    Deleted,
}

/// One changed block: base lines `old` became buffer lines `new` (0-based, half-open).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct LineChange {
    pub kind: LineChangeKind,
    pub old: Range<usize>,
    pub new: Range<usize>,
}

/// The mark painted next to one buffer line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GutterMark {
    Added,
    Modified,
    /// Base lines were removed right after this line.
    DeletedAfter,
    /// Base lines were removed before the first line.
    DeletedBefore,
}

impl LineChange {
    /// The base lines this block replaced, each ended by `\n`: the text that puts them back
    /// in place of buffer lines `new` ("Rollback lines").
    pub fn original_text(&self, base: &str) -> String {
        let lines = split_lines(base);
        let mut out = String::new();
        for line in &lines[self.old.clone()] {
            out.push_str(line);
            out.push('\n');
        }
        out
    }

    /// The base lines this block replaced (without line endings).
    pub fn original_lines<'a>(&self, base: &'a str) -> Vec<&'a str> {
        split_lines(base)[self.old.clone()].to_vec()
    }

    /// The buffer line that carries this block's mark.
    pub fn anchor_line(&self) -> usize {
        match self.kind {
            LineChangeKind::Deleted => self.new.start.saturating_sub(1),
            _ => self.new.start,
        }
    }
}

/// Every changed block of a buffer, in line order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LineChanges {
    pub changes: Vec<LineChange>,
}

impl LineChanges {
    /// Compares `base` (e.g. the file at HEAD) with the `current` buffer text. Line endings
    /// (`\n` / `\r\n`) and a missing final newline are ignored.
    pub fn compute(base: &str, current: &str) -> LineChanges {
        Self::compute_lines(&split_lines(base), &split_lines(current))
    }

    /// Same as [`LineChanges::compute`] for texts already split into lines.
    pub fn compute_lines(base: &[&str], current: &[&str]) -> LineChanges {
        let changes = textdiff::diff(base, current)
            .into_iter()
            .map(|c| {
                let kind = if c.old.is_empty() {
                    LineChangeKind::Added
                } else if c.new.is_empty() {
                    LineChangeKind::Deleted
                } else {
                    LineChangeKind::Modified
                };
                LineChange { kind, old: c.old, new: c.new }
            })
            .collect();
        LineChanges { changes }
    }

    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    /// The mark of buffer line `line`, if any (binary search).
    pub fn mark_at(&self, line: usize) -> Option<GutterMark> {
        self.change_at(line).map(|c| mark_of(c, line))
    }

    /// The block whose mark sits on `line` (for the gutter popup and rollback).
    pub fn change_at(&self, line: usize) -> Option<&LineChange> {
        let i = self.changes.partition_point(|c| c.new.end.max(c.anchor_line() + 1) <= line);
        self.changes[i..].iter().take(2).find(|c| c.new.contains(&line) || (c.kind == LineChangeKind::Deleted && c.anchor_line() == line))
    }

    /// One mark per line for `line_count` buffer lines.
    pub fn marks(&self, line_count: usize) -> Vec<Option<GutterMark>> {
        let mut out = vec![None; line_count];
        for c in &self.changes {
            if c.kind == LineChangeKind::Deleted {
                let line = c.anchor_line();
                if line < line_count && out[line].is_none() {
                    out[line] = Some(mark_of(c, line));
                }
            } else {
                for line in c.new.clone().filter(|l| *l < line_count) {
                    out[line] = Some(mark_of(c, line));
                }
            }
        }
        out
    }

    /// The first block starting after `line` ("Next change").
    pub fn next_after(&self, line: usize) -> Option<&LineChange> {
        self.changes.iter().find(|c| c.anchor_line() > line)
    }

    /// The last block starting before `line` ("Previous change").
    pub fn previous_before(&self, line: usize) -> Option<&LineChange> {
        self.changes.iter().rev().find(|c| c.anchor_line() < line)
    }
}

fn mark_of(c: &LineChange, line: usize) -> GutterMark {
    match c.kind {
        LineChangeKind::Added => GutterMark::Added,
        LineChangeKind::Modified => GutterMark::Modified,
        LineChangeKind::Deleted if c.new.start == 0 && line == 0 => GutterMark::DeletedBefore,
        LineChangeKind::Deleted => GutterMark::DeletedAfter,
    }
}

/// Splits text into lines without their `\n` / `\r\n`; a final newline does not start an
/// extra line, and an empty text has no lines.
pub fn split_lines(text: &str) -> Vec<&str> {
    let body = text.strip_suffix('\n').unwrap_or(text);
    if text.is_empty() {
        return Vec::new();
    }
    body.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use GutterMark::*;

    #[test]
    fn added_modified_deleted() {
        let base = "a\nb\nc\nd\ne\n";
        let cur = "a\nB\nc\nnew\nd\n";
        let lc = LineChanges::compute(base, cur);
        assert_eq!(lc.marks(5), vec![None, Some(Modified), None, Some(Added), Some(DeletedAfter)]);
        let modified = lc.change_at(1).unwrap();
        assert_eq!(modified.original_text(base), "b\n");
        let deleted = lc.change_at(4).unwrap();
        assert_eq!(deleted.kind, LineChangeKind::Deleted);
        assert_eq!(deleted.original_lines(base), vec!["e"]);
        assert_eq!(lc.next_after(1).unwrap().new, 3..4);
        assert_eq!(lc.previous_before(3).unwrap().new, 1..2);
    }

    #[test]
    fn deleted_at_top_and_line_endings() {
        let lc = LineChanges::compute("x\r\na\r\nb", "a\nb\n");
        assert_eq!(lc.marks(2), vec![Some(DeletedBefore), None]);
        assert_eq!(lc.mark_at(0), Some(DeletedBefore));
        assert!(LineChanges::compute("a\r\nb\r\n", "a\nb").is_empty());
        assert_eq!(split_lines(""), Vec::<&str>::new());
        assert_eq!(split_lines("\n"), vec![""]);
    }

    #[test]
    fn new_file_is_all_added() {
        let lc = LineChanges::compute("", "a\nb\n");
        assert_eq!(lc.marks(2), vec![Some(Added), Some(Added)]);
    }

    proptest::proptest! {
        #[test]
        fn rollback_of_every_change_restores_base(
            base in proptest::collection::vec(0u8..5, 0..30),
            cur in proptest::collection::vec(0u8..5, 0..30),
        ) {
            let b: Vec<String> = base.iter().map(|x| format!("l{x}")).collect();
            let c: Vec<String> = cur.iter().map(|x| format!("l{x}")).collect();
            let bt: String = b.iter().map(|l| format!("{l}\n")).collect();
            let br: Vec<&str> = b.iter().map(String::as_str).collect();
            let cr: Vec<&str> = c.iter().map(String::as_str).collect();
            let lc = LineChanges::compute_lines(&br, &cr);
            // Undo the blocks bottom-up, the way the editor applies "Rollback".
            let mut lines: Vec<String> = c.clone();
            for ch in lc.changes.iter().rev() {
                let orig: Vec<String> = ch.original_lines(&bt).into_iter().map(String::from).collect();
                lines.splice(ch.new.clone(), orig);
            }
            proptest::prop_assert_eq!(lines, b);
            let marks = lc.marks(c.len());
            for (line, m) in marks.iter().enumerate() {
                proptest::prop_assert_eq!(*m, lc.mark_at(line));
            }
        }
    }
}
