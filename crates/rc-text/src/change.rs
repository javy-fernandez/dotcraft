//! Change sets: a batch of non-overlapping replacements expressed in the coordinates of the
//! document *before* the batch, so positions can be mapped through them and they can be
//! inverted for undo.

use ropey::Rope;

/// One replacement: chars `start..end` of the old document become `text`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub start: usize,
    pub end: usize,
    pub text: String,
}

impl Change {
    pub fn new(start: usize, end: usize, text: impl Into<String>) -> Self {
        Change { start, end, text: text.into() }
    }

    pub fn insert(at: usize, text: impl Into<String>) -> Self {
        Change::new(at, at, text)
    }

    pub fn delete(start: usize, end: usize) -> Self {
        Change::new(start, end, "")
    }

    fn new_len(&self) -> usize {
        self.text.chars().count()
    }
}

/// Which side a position sticks to when text is inserted exactly at it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Assoc {
    /// Stays before the inserted text.
    Before,
    /// Moves after the inserted text (a cursor that typed it).
    After,
}

/// Sorted, non-overlapping changes against one document version.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChangeSet {
    changes: Vec<Change>,
}

impl ChangeSet {
    /// Builds a change set; `changes` may come in any order but must not overlap (touching
    /// ranges are fine; two inserts at the same point keep their given order).
    pub fn new(mut changes: Vec<Change>) -> Self {
        changes.retain(|c| c.start != c.end || !c.text.is_empty());
        changes.sort_by_key(|c| (c.start, c.end));
        // Multi-cursor commands can produce overlapping edits (two carets deleting the same
        // char): they merge into one replacement covering both.
        let mut merged: Vec<Change> = Vec::with_capacity(changes.len());
        for c in changes {
            match merged.last_mut() {
                Some(last) if c.start < last.end => {
                    last.end = last.end.max(c.end);
                    last.text.push_str(&c.text);
                }
                _ => merged.push(c),
            }
        }
        ChangeSet { changes: merged }
    }

    pub fn single(change: Change) -> Self {
        ChangeSet::new(vec![change])
    }

    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    pub fn changes(&self) -> &[Change] {
        &self.changes
    }

    /// Whether every change only inserts text (no deletions).
    pub fn is_pure_insert(&self) -> bool {
        self.changes.iter().all(|c| c.start == c.end)
    }

    /// Whether every change only deletes text.
    pub fn is_pure_delete(&self) -> bool {
        self.changes.iter().all(|c| c.text.is_empty())
    }

    /// Applies the set, last change first so earlier offsets stay valid.
    pub fn apply(&self, rope: &mut Rope) {
        for c in self.changes.iter().rev() {
            if c.end > c.start {
                rope.remove(c.start..c.end);
            }
            if !c.text.is_empty() {
                rope.insert(c.start, &c.text);
            }
        }
    }

    /// The set that undoes this one; `before` is the document this set applies to.
    pub fn invert(&self, before: &Rope) -> ChangeSet {
        let mut delta: isize = 0;
        let changes = self
            .changes
            .iter()
            .map(|c| {
                let start = (c.start as isize + delta) as usize;
                let new_len = c.new_len();
                delta += new_len as isize - (c.end - c.start) as isize;
                Change::new(start, start + new_len, before.slice(c.start..c.end).to_string())
            })
            .collect();
        ChangeSet { changes }
    }

    /// Maps a position of the old document to the new one.
    pub fn map_pos(&self, pos: usize, assoc: Assoc) -> usize {
        let mut delta: isize = 0;
        for c in &self.changes {
            if pos < c.start {
                break;
            }
            let new_len = c.new_len() as isize;
            let start = c.start as isize + delta;
            if pos == c.start && c.start == c.end {
                // An insert exactly here: `Before` stays put, `After` moves past it (and
                // past any further inserts at the same point).
                if assoc == Assoc::Before {
                    break;
                }
            } else if pos < c.end {
                // At the start of, or inside, a replaced range: collapse to one of its edges.
                return match assoc {
                    Assoc::Before => start as usize,
                    Assoc::After => (start + new_len) as usize,
                };
            }
            delta += new_len - (c.end - c.start) as isize;
        }
        (pos as isize + delta) as usize
    }

    /// Length of the document after the set, given its length before.
    pub fn len_after(&self, len_before: usize) -> usize {
        self.changes.iter().fold(len_before as isize, |n, c| n + c.new_len() as isize - (c.end - c.start) as isize) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_and_invert_round_trip() {
        let before = Rope::from_str("hello world");
        let cs = ChangeSet::new(vec![Change::new(6, 11, "there"), Change::insert(0, ">> "), Change::delete(4, 5)]);
        let mut doc = before.clone();
        cs.apply(&mut doc);
        assert_eq!(doc.to_string(), ">> hell there");
        cs.invert(&before).apply(&mut doc);
        assert_eq!(doc, before);
    }

    #[test]
    fn positions_follow_inserts_by_association() {
        let cs = ChangeSet::single(Change::insert(3, "xy"));
        assert_eq!(cs.map_pos(3, Assoc::Before), 3);
        assert_eq!(cs.map_pos(3, Assoc::After), 5);
        assert_eq!(cs.map_pos(2, Assoc::After), 2);
        assert_eq!(cs.map_pos(4, Assoc::Before), 6);
    }

    #[test]
    fn positions_inside_deleted_ranges_collapse() {
        let cs = ChangeSet::single(Change::new(2, 6, "z"));
        assert_eq!(cs.map_pos(4, Assoc::Before), 2);
        assert_eq!(cs.map_pos(4, Assoc::After), 3);
        assert_eq!(cs.map_pos(2, Assoc::Before), 2);
        assert_eq!(cs.map_pos(6, Assoc::Before), 3);
        assert_eq!(cs.map_pos(8, Assoc::Before), 5);
    }
}
