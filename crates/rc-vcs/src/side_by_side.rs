//! Side-by-side diff model: aligned left/right rows with word-level changes inside modified
//! lines. The viewer renders the two texts in two editors and uses the rows to align them.

use crate::gutter::{LineChange, LineChanges, split_lines};
use crate::textdiff;
use std::ops::Range;

/// How a row of the side-by-side view differs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RowKind {
    Equal,
    /// Only the right side has a line.
    Added,
    /// Only the left side has a line.
    Deleted,
    /// Both sides have a line and they differ.
    Modified,
}

/// One aligned row. Line numbers are 0-based indices into each side's lines; `None` is a
/// filler. Inline ranges are byte ranges of the changed words within that side's line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub kind: RowKind,
    pub left: Option<usize>,
    pub right: Option<usize>,
    pub left_inline: Vec<Range<usize>>,
    pub right_inline: Vec<Range<usize>>,
}

/// The aligned rows of two texts plus their changed blocks.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SideBySide {
    pub rows: Vec<Row>,
    /// Changed blocks (left = old lines, right = new lines) with the index of their first row.
    pub blocks: Vec<(LineChange, usize)>,
}

impl SideBySide {
    /// Aligns `old` (left) and `new` (right) line by line.
    pub fn compute(old: &str, new: &str) -> SideBySide {
        let (ol, nl) = (split_lines(old), split_lines(new));
        let changes = LineChanges::compute_lines(&ol, &nl).changes;
        let mut rows = Vec::with_capacity(ol.len().max(nl.len()));
        let mut blocks = Vec::with_capacity(changes.len());
        let (mut i, mut j) = (0, 0);
        let equal = |rows: &mut Vec<Row>, i: usize, j: usize| {
            rows.push(Row { kind: RowKind::Equal, left: Some(i), right: Some(j), left_inline: Vec::new(), right_inline: Vec::new() })
        };
        for c in changes {
            while i < c.old.start {
                equal(&mut rows, i, j);
                i += 1;
                j += 1;
            }
            blocks.push((c.clone(), rows.len()));
            let len = c.old.len().max(c.new.len());
            for k in 0..len {
                let l = (k < c.old.len()).then(|| c.old.start + k);
                let r = (k < c.new.len()).then(|| c.new.start + k);
                let row = match (l, r) {
                    (Some(l), Some(r)) => {
                        let (li, ri) = inline_changes(ol[l], nl[r]);
                        Row { kind: RowKind::Modified, left: Some(l), right: Some(r), left_inline: li, right_inline: ri }
                    }
                    (Some(l), None) => Row { kind: RowKind::Deleted, left: Some(l), right: None, left_inline: Vec::new(), right_inline: Vec::new() },
                    (None, r) => Row { kind: RowKind::Added, left: None, right: r, left_inline: Vec::new(), right_inline: Vec::new() },
                };
                rows.push(row);
            }
            i = c.old.end;
            j = c.new.end;
        }
        while i < ol.len() {
            equal(&mut rows, i, j);
            i += 1;
            j += 1;
        }
        SideBySide { rows, blocks }
    }

    /// The row showing left line `line` (for scroll sync).
    pub fn row_of_left(&self, line: usize) -> Option<usize> {
        self.rows.iter().position(|r| r.left == Some(line))
    }

    /// The row showing right line `line` (for scroll sync).
    pub fn row_of_right(&self, line: usize) -> Option<usize> {
        self.rows.iter().position(|r| r.right == Some(line))
    }
}

/// Word-level differences between two versions of a line: the changed byte ranges of each.
/// Words are runs of letters/digits/`_`, runs of whitespace, or single other characters.
pub fn inline_changes(old: &str, new: &str) -> (Vec<Range<usize>>, Vec<Range<usize>>) {
    let (to, tn) = (tokens(old), tokens(new));
    let ws: Vec<&str> = to.iter().map(|r| &old[r.clone()]).collect();
    let wn: Vec<&str> = tn.iter().map(|r| &new[r.clone()]).collect();
    let mut left = Vec::new();
    let mut right = Vec::new();
    for c in textdiff::diff(&ws, &wn) {
        if !c.old.is_empty() {
            push_merged(&mut left, to[c.old.start].start..to[c.old.end - 1].end);
        }
        if !c.new.is_empty() {
            push_merged(&mut right, tn[c.new.start].start..tn[c.new.end - 1].end);
        }
    }
    (left, right)
}

fn push_merged(out: &mut Vec<Range<usize>>, r: Range<usize>) {
    match out.last_mut() {
        Some(last) if last.end == r.start => last.end = r.end,
        _ => out.push(r),
    }
}

fn tokens(s: &str) -> Vec<Range<usize>> {
    #[derive(PartialEq)]
    enum Class {
        Word,
        Space,
        Other,
    }
    let class = |c: char| {
        if c.is_alphanumeric() || c == '_' {
            Class::Word
        } else if c.is_whitespace() {
            Class::Space
        } else {
            Class::Other
        }
    };
    let mut out: Vec<Range<usize>> = Vec::new();
    let mut prev: Option<Class> = None;
    for (i, c) in s.char_indices() {
        let k = class(c);
        let joins = prev.as_ref() == Some(&k) && k != Class::Other;
        match out.last_mut() {
            Some(last) if joins => last.end = i + c.len_utf8(),
            _ => out.push(i..i + c.len_utf8()),
        }
        prev = Some(k);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_are_aligned() {
        let sbs = SideBySide::compute("a\nb\nc\n", "a\nB\nx\nc\n");
        let kinds: Vec<RowKind> = sbs.rows.iter().map(|r| r.kind).collect();
        assert_eq!(kinds, vec![RowKind::Equal, RowKind::Modified, RowKind::Added, RowKind::Equal]);
        assert_eq!(sbs.rows[2].left, None);
        assert_eq!(sbs.rows[3].left, Some(2));
        assert_eq!(sbs.rows[3].right, Some(3));
        assert_eq!(sbs.blocks.len(), 1);
        assert_eq!(sbs.blocks[0].1, 1);
        assert_eq!(sbs.row_of_right(3), Some(3));
    }

    #[test]
    fn inline_words() {
        let (l, r) = inline_changes("let total = price * qty;", "let total = price * quantity + tax;");
        assert_eq!(&"let total = price * qty;"[l[0].clone()], "qty");
        let new = "let total = price * quantity + tax;";
        assert_eq!(&new[r[0].clone()], "quantity + tax");
        let (l, r) = inline_changes("same", "same");
        assert!(l.is_empty() && r.is_empty());
        let (_, r) = inline_changes("año", "años");
        assert_eq!(&"años"[r[0].clone()], "años");
    }
}
