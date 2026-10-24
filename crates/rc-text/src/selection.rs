//! Multi-cursor selections. Every cursor is a [`Range`] whose `head` is the caret and whose
//! `anchor` is where the selection started; an empty range is a plain caret.

use crate::change::{Assoc, ChangeSet};
use smallvec::{SmallVec, smallvec};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Range {
    pub anchor: usize,
    pub head: usize,
    /// Visual column kept while moving up/down through shorter lines.
    pub goal_col: Option<usize>,
}

impl Range {
    pub fn new(anchor: usize, head: usize) -> Self {
        Range { anchor, head, goal_col: None }
    }

    pub fn point(pos: usize) -> Self {
        Range::new(pos, pos)
    }

    pub fn start(&self) -> usize {
        self.anchor.min(self.head)
    }

    pub fn end(&self) -> usize {
        self.anchor.max(self.head)
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }

    pub fn len(&self) -> usize {
        self.end() - self.start()
    }

    pub fn contains(&self, pos: usize) -> bool {
        self.start() <= pos && pos < self.end()
    }

    /// Moves the caret; with `extend` the anchor stays, otherwise the range collapses.
    pub fn put_head(self, head: usize, extend: bool) -> Self {
        Range { anchor: if extend { self.anchor } else { head }, head, goal_col: None }
    }

    pub fn map(self, changes: &ChangeSet) -> Self {
        if self.is_empty() {
            let p = changes.map_pos(self.head, Assoc::After);
            return Range::point(p);
        }
        // A non-empty range does not grow over text inserted at its edges.
        let start = changes.map_pos(self.start(), Assoc::After);
        let end = changes.map_pos(self.end(), Assoc::Before).max(start);
        if self.anchor <= self.head { Range::new(start, end) } else { Range::new(end, start) }
    }
}

/// One or more ranges, sorted and non-overlapping; `primary` is the one that scrolls into
/// view and that single-cursor commands use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selection {
    ranges: SmallVec<[Range; 1]>,
    primary: usize,
}

impl Default for Selection {
    fn default() -> Self {
        Selection::point(0)
    }
}

impl Selection {
    pub fn point(pos: usize) -> Self {
        Selection::single(Range::point(pos))
    }

    pub fn single(range: Range) -> Self {
        Selection { ranges: smallvec![range], primary: 0 }
    }

    /// Builds a selection from any ranges; they are sorted and overlapping ones merged.
    pub fn new(ranges: impl IntoIterator<Item = Range>, primary: usize) -> Self {
        let ranges: SmallVec<[Range; 1]> = ranges.into_iter().collect();
        assert!(!ranges.is_empty(), "a selection needs at least one range");
        let primary_range = ranges[primary.min(ranges.len() - 1)];
        let mut sel = Selection { ranges, primary: 0 };
        sel.normalize(primary_range);
        sel
    }

    fn normalize(&mut self, primary_range: Range) {
        self.ranges.sort_by_key(|r| (r.start(), r.end()));
        let mut out: SmallVec<[Range; 1]> = SmallVec::new();
        let mut primary = 0;
        for r in self.ranges.drain(..) {
            let is_primary = r == primary_range;
            if let Some(last) = out.last_mut() {
                let overlaps = r.start() < last.end() || (r.start() == last.end() && (r.is_empty() || last.is_empty()));
                if overlaps {
                    let start = last.start().min(r.start());
                    let end = last.end().max(r.end());
                    let forward = last.head >= last.anchor;
                    *last = if forward { Range::new(start, end) } else { Range::new(end, start) };
                    if is_primary {
                        primary = out.len() - 1;
                    }
                    continue;
                }
            }
            if is_primary {
                primary = out.len();
            }
            out.push(r);
        }
        self.ranges = out;
        self.primary = primary;
    }

    pub fn ranges(&self) -> &[Range] {
        &self.ranges
    }

    pub fn len(&self) -> usize {
        self.ranges.len()
    }

    pub fn is_empty(&self) -> bool {
        false
    }

    pub fn primary(&self) -> Range {
        self.ranges[self.primary]
    }

    pub fn primary_index(&self) -> usize {
        self.primary
    }

    /// Adds a range and makes it primary.
    pub fn push(&self, range: Range) -> Self {
        let mut ranges = self.ranges.clone();
        ranges.push(range);
        let n = ranges.len() - 1;
        Selection::new(ranges, n)
    }

    /// Keeps only the primary range.
    pub fn into_primary(self) -> Self {
        Selection::single(self.primary())
    }

    pub fn transform(&self, mut f: impl FnMut(Range) -> Range) -> Self {
        Selection::new(self.ranges.iter().map(|&r| f(r)), self.primary)
    }

    pub fn map(&self, changes: &ChangeSet) -> Self {
        self.transform(|r| r.map(changes))
    }

    /// Clamps every range to a document of `len` chars.
    pub fn clamp(&self, len: usize) -> Self {
        self.transform(|r| Range { anchor: r.anchor.min(len), head: r.head.min(len), goal_col: r.goal_col })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::change::Change;

    #[test]
    fn overlapping_ranges_merge_and_keep_primary() {
        let s = Selection::new([Range::new(5, 8), Range::new(0, 2), Range::new(7, 10), Range::point(1)], 2);
        assert_eq!(s.ranges(), &[Range::new(0, 2), Range::new(5, 10)]);
        assert_eq!(s.primary(), Range::new(5, 10));
    }

    #[test]
    fn carets_move_after_text_typed_at_them() {
        let s = Selection::new([Range::point(1), Range::point(4)], 0);
        let cs = ChangeSet::new(vec![Change::insert(1, "ab"), Change::insert(4, "ab")]);
        assert_eq!(s.map(&cs).ranges(), &[Range::point(3), Range::point(8)]);
    }
}
