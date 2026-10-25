//! Editing and movement commands. They are pure: they read the rope and the selection and
//! return the change set to apply plus the selection after it (or just a new selection for
//! movements), so they can be tested without any UI.

use crate::buffer::line_len;
use crate::change::{Change, ChangeSet};
use crate::selection::{Range, Selection};
use ropey::Rope;

/// The result of an editing command.
#[derive(Clone, Debug)]
pub struct Edited {
    pub changes: ChangeSet,
    pub selection: Selection,
}

/// Indentation settings of a document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Indent {
    /// One level: four spaces, or `"\t"`.
    pub unit: String,
    /// Columns a tab advances to.
    pub tab_width: usize,
}

impl Default for Indent {
    fn default() -> Self {
        Indent { unit: "    ".into(), tab_width: 4 }
    }
}

// ----------------------------------------------------------------- characters & words

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CharClass {
    Word,
    Space,
    Newline,
    Punct,
}

pub fn char_class(c: char) -> CharClass {
    if c == '\n' {
        CharClass::Newline
    } else if c.is_whitespace() {
        CharClass::Space
    } else if c.is_alphanumeric() || c == '_' {
        CharClass::Word
    } else {
        CharClass::Punct
    }
}

fn char_at(rope: &Rope, pos: usize) -> Option<char> {
    (pos < rope.len_chars()).then(|| rope.char(pos))
}

fn char_before(rope: &Rope, pos: usize) -> Option<char> {
    (pos > 0).then(|| rope.char(pos - 1))
}

/// Start of the word to the left of `pos` (IntelliJ: skips spaces, then one class run; a
/// line break is its own stop).
pub fn word_left(rope: &Rope, mut pos: usize) -> usize {
    if char_before(rope, pos) == Some('\n') {
        return pos - 1;
    }
    while char_before(rope, pos).is_some_and(|c| char_class(c) == CharClass::Space) {
        pos -= 1;
    }
    if let Some(c) = char_before(rope, pos) {
        let class = char_class(c);
        if class == CharClass::Newline {
            return pos;
        }
        while char_before(rope, pos).is_some_and(|c| char_class(c) == class) {
            pos -= 1;
        }
    }
    pos
}

/// End of the word to the right of `pos`.
pub fn word_right(rope: &Rope, mut pos: usize) -> usize {
    if char_at(rope, pos) == Some('\n') {
        return pos + 1;
    }
    if let Some(c) = char_at(rope, pos) {
        let class = char_class(c);
        if class != CharClass::Space {
            while char_at(rope, pos).is_some_and(|c| char_class(c) == class) {
                pos += 1;
            }
        }
    }
    while char_at(rope, pos).is_some_and(|c| char_class(c) == CharClass::Space) {
        pos += 1;
    }
    pos
}

/// The identifier around `pos` (start, end), empty when there is none.
pub fn word_at(rope: &Rope, pos: usize) -> (usize, usize) {
    let is_word = |c: char| char_class(c) == CharClass::Word;
    let mut start = pos;
    while char_before(rope, start).is_some_and(is_word) {
        start -= 1;
    }
    let mut end = pos;
    while char_at(rope, end).is_some_and(is_word) {
        end += 1;
    }
    (start, end)
}

// ----------------------------------------------------------------- columns

/// Visual column of `pos` (tabs expanded).
pub fn visual_col(rope: &Rope, pos: usize, tab_width: usize) -> usize {
    let line = rope.char_to_line(pos);
    let start = rope.line_to_char(line);
    let mut col = 0;
    for c in rope.slice(start..pos).chars() {
        col = if c == '\t' { (col / tab_width + 1) * tab_width } else { col + 1 };
    }
    col
}

/// Char offset on `line` closest to visual column `vcol`.
pub fn pos_at_visual_col(rope: &Rope, line: usize, vcol: usize, tab_width: usize) -> usize {
    let start = rope.line_to_char(line);
    let len = line_len(rope, line);
    let mut col = 0;
    for (i, c) in rope.slice(start..start + len).chars().enumerate() {
        let next = if c == '\t' { (col / tab_width + 1) * tab_width } else { col + 1 };
        if next > vcol {
            return start + if vcol - col > (next - col) / 2 { i + 1 } else { i };
        }
        col = next;
    }
    start + len
}

/// Leading whitespace of `line`.
pub fn line_indent(rope: &Rope, line: usize) -> String {
    rope.line(line).chars().take_while(|c| *c == ' ' || *c == '\t').collect()
}

// ----------------------------------------------------------------- movement

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion {
    Left,
    Right,
    Up,
    Down,
    WordLeft,
    WordRight,
    /// Home: first non-blank char, or column 0 when already there (smart home).
    LineStart,
    LineEnd,
    DocStart,
    DocEnd,
    PageUp(usize),
    PageDown(usize),
}

pub fn move_selection(rope: &Rope, sel: &Selection, motion: Motion, extend: bool, tab_width: usize) -> Selection {
    sel.transform(|r| move_range(rope, r, motion, extend, tab_width))
}

fn move_range(rope: &Rope, r: Range, motion: Motion, extend: bool, tab_width: usize) -> Range {
    let len = rope.len_chars();
    let head = r.head;
    let vertical = |lines: isize| -> Range {
        let line = rope.char_to_line(head) as isize;
        let goal = r.goal_col.unwrap_or_else(|| visual_col(rope, head, tab_width));
        let target = line + lines;
        let new_head = if target < 0 {
            0
        } else if target as usize >= rope.len_lines() {
            len
        } else {
            pos_at_visual_col(rope, target as usize, goal, tab_width)
        };
        let mut out = r.put_head(new_head, extend);
        out.goal_col = Some(goal);
        out
    };
    match motion {
        Motion::Left if !extend && !r.is_empty() => Range::point(r.start()),
        Motion::Right if !extend && !r.is_empty() => Range::point(r.end()),
        Motion::Left => r.put_head(head.saturating_sub(1), extend),
        Motion::Right => r.put_head((head + 1).min(len), extend),
        Motion::Up => vertical(-1),
        Motion::Down => vertical(1),
        Motion::PageUp(n) => vertical(-(n as isize)),
        Motion::PageDown(n) => vertical(n as isize),
        Motion::WordLeft => r.put_head(word_left(rope, head), extend),
        Motion::WordRight => r.put_head(word_right(rope, head), extend),
        Motion::LineStart => {
            let line = rope.char_to_line(head);
            let start = rope.line_to_char(line);
            let first = start + line_indent(rope, line).chars().count();
            r.put_head(if head == first { start } else { first }, extend)
        }
        Motion::LineEnd => {
            let line = rope.char_to_line(head);
            r.put_head(rope.line_to_char(line) + line_len(rope, line), extend)
        }
        Motion::DocStart => r.put_head(0, extend),
        Motion::DocEnd => r.put_head(len, extend),
    }
}

/// Adds a caret on the line above (`up`) or below every caret at the same visual column.
pub fn add_caret_vertical(rope: &Rope, sel: &Selection, up: bool, tab_width: usize) -> Selection {
    let mut ranges: Vec<Range> = sel.ranges().to_vec();
    let edge = if up { sel.ranges()[0] } else { *sel.ranges().last().expect("non-empty") };
    let line = rope.char_to_line(edge.head);
    let target = if up { line.checked_sub(1) } else { (line + 1 < rope.len_lines()).then_some(line + 1) };
    let Some(target) = target else { return sel.clone() };
    let goal = edge.goal_col.unwrap_or_else(|| visual_col(rope, edge.head, tab_width));
    let mut caret = Range::point(pos_at_visual_col(rope, target, goal, tab_width));
    caret.goal_col = Some(goal);
    ranges.push(caret);
    let n = ranges.len() - 1;
    Selection::new(ranges, n)
}

/// Alt+J: selects the word under the caret, or adds the next occurrence of the primary
/// selection's text as a new selection (wrapping around the document).
pub fn select_next_occurrence(rope: &Rope, sel: &Selection) -> Selection {
    let primary = sel.primary();
    if primary.is_empty() {
        let (s, e) = word_at(rope, primary.head);
        if s == e {
            return sel.clone();
        }
        return sel.transform(|r| if r == primary { Range::new(s, e) } else { r });
    }
    let needle: String = rope.slice(primary.start()..primary.end()).to_string();
    let text = rope.to_string();
    let last_end = sel.ranges().iter().map(|r| r.end()).max().unwrap_or(0);
    let from_byte = rope.char_to_byte(last_end);
    let found = text[from_byte..].find(&needle).map(|i| i + from_byte).or_else(|| text.find(&needle));
    let Some(b) = found else { return sel.clone() };
    let start = rope.byte_to_char(b);
    let range = Range::new(start, start + needle.chars().count());
    if sel.ranges().iter().any(|r| r.start() == range.start()) {
        return sel.clone();
    }
    sel.push(range)
}

/// Selects every occurrence of the primary selection's text (or the word under the caret).
pub fn select_all_occurrences(rope: &Rope, sel: &Selection) -> Selection {
    let mut primary = sel.primary();
    if primary.is_empty() {
        let (s, e) = word_at(rope, primary.head);
        if s == e {
            return sel.clone();
        }
        primary = Range::new(s, e);
    }
    let needle: String = rope.slice(primary.start()..primary.end()).to_string();
    let text = rope.to_string();
    let ranges: Vec<Range> = text
        .match_indices(&needle)
        .map(|(b, _)| {
            let s = rope.byte_to_char(b);
            Range::new(s, s + needle.chars().count())
        })
        .collect();
    let primary_index = ranges.iter().position(|r| r.start() == primary.start()).unwrap_or(0);
    Selection::new(ranges, primary_index)
}

// ----------------------------------------------------------------- editing

/// Replaces every range with `text` (typing or pasting). When `per_range` has one entry per
/// range (pasting as many lines as there are carets), each range gets its own text.
pub fn replace_selections(sel: &Selection, text: &str, per_range: Option<&[String]>) -> Edited {
    let changes: Vec<Change> = sel
        .ranges()
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let t = per_range.and_then(|p| p.get(i)).map(String::as_str).unwrap_or(text);
            Change::new(r.start(), r.end(), t)
        })
        .collect();
    let cs = ChangeSet::new(changes);
    let selection = sel.map(&cs).transform(|r| Range::point(r.end()));
    Edited { changes: cs, selection }
}

const PAIRS: [(char, char); 5] = [('(', ')'), ('[', ']'), ('{', '}'), ('"', '"'), ('\'', '\'')];

/// Types one character, with IntelliJ-style pairing: an opening bracket or quote gets its
/// closing one when the caret is before blank/closing text, typing a closing character over
/// the same one just moves past it, and a selection is wrapped in brackets/quotes.
pub fn type_char(rope: &Rope, sel: &Selection, ch: char, auto_pair: bool) -> Edited {
    if auto_pair {
        // Type-over: every caret sits before the same closing character.
        let closes = PAIRS.iter().any(|&(_, c)| c == ch);
        if closes && sel.ranges().iter().all(|r| r.is_empty() && char_at(rope, r.head) == Some(ch)) {
            let is_quote = ch == '"' || ch == '\'';
            if !is_quote || sel.ranges().iter().all(|r| char_before(rope, r.head).is_some_and(|c| c != '\\')) {
                return Edited { changes: ChangeSet::default(), selection: sel.transform(|r| Range::point(r.head + 1)) };
            }
        }
        if let Some(&(open, close)) = PAIRS.iter().find(|&&(o, _)| o == ch) {
            // Wrap selections.
            if sel.ranges().iter().all(|r| !r.is_empty()) {
                let mut changes = Vec::new();
                for r in sel.ranges() {
                    changes.push(Change::insert(r.start(), open.to_string()));
                    changes.push(Change::insert(r.end(), close.to_string()));
                }
                let cs = ChangeSet::new(changes);
                let selection = sel.transform(|r| {
                    let shift = 1 + 2 * sel.ranges().iter().filter(|o| o.start() < r.start()).count();
                    Range::new(r.anchor + shift, r.head + shift)
                });
                return Edited { changes: cs, selection };
            }
            let pairable = sel.ranges().iter().all(|r| {
                let next = char_at(rope, r.end());
                let next_ok = next.is_none_or(|c| c.is_whitespace() || ")]};,".contains(c));
                let quote_ok = open != close || char_before(rope, r.start()).is_none_or(|c| !(c.is_alphanumeric() || c == '_' || c == open));
                next_ok && quote_ok
            });
            if pairable {
                let mut s = String::with_capacity(2);
                s.push(open);
                s.push(close);
                let cs = ChangeSet::new(sel.ranges().iter().map(|r| Change::new(r.start(), r.end(), s.clone())).collect());
                let selection = sel.map(&cs).transform(|r| Range::point(r.end() - 1));
                return Edited { changes: cs, selection };
            }
        }
    }
    replace_selections(sel, ch.encode_utf8(&mut [0; 4]), None)
}

/// Enter: breaks the line keeping the indentation, adds one level after `{`, and opens a
/// block when the caret sits between `{` and `}`.
pub fn newline(rope: &Rope, sel: &Selection, indent: &Indent) -> Edited {
    let mut changes = Vec::new();
    let mut carets = Vec::new();
    let mut delta: isize = 0;
    for r in sel.ranges() {
        let line = rope.char_to_line(r.start());
        let line_start = rope.line_to_char(line);
        let before = rope.slice(line_start..r.start()).to_string();
        let trimmed = before.trim_end_matches([' ', '\t']);
        // With the caret inside the leading blanks, the new line keeps the caret's indent.
        let base = if trimmed.is_empty() { before.clone() } else { line_indent(rope, line) };
        // Blanks on both sides of the caret go: no trailing spaces, the new line starts at the indent.
        let start = line_start + trimmed.chars().count();
        let mut end = r.end();
        while char_at(rope, end).is_some_and(|c| c == ' ' || c == '\t') {
            end += 1;
        }
        let prev = trimmed.chars().last();
        let (text, caret_offset) = if prev == Some('{') && char_at(rope, end) == Some('}') {
            let inner = format!("\n{base}{}", indent.unit);
            let n = inner.chars().count();
            (format!("{inner}\n{base}"), n)
        } else {
            let t = if prev == Some('{') { format!("\n{base}{}", indent.unit) } else { format!("\n{base}") };
            let n = t.chars().count();
            (t, n)
        };
        carets.push(Range::point((start as isize + delta) as usize + caret_offset));
        delta += text.chars().count() as isize - (end - start) as isize;
        changes.push(Change::new(start, end, text));
    }
    Edited { changes: ChangeSet::new(changes), selection: Selection::new(carets, sel.primary_index()) }
}

/// Backspace: deletes selections, a pair of brackets around the caret, a whole indent level
/// in leading whitespace, or one char.
pub fn backspace(rope: &Rope, sel: &Selection, indent: &Indent) -> Edited {
    let changes: Vec<Change> = sel
        .ranges()
        .iter()
        .filter_map(|r| {
            if !r.is_empty() {
                return Some(Change::delete(r.start(), r.end()));
            }
            let pos = r.head;
            if pos == 0 {
                return None;
            }
            let prev = rope.char(pos - 1);
            if let Some(next) = char_at(rope, pos)
                && PAIRS.iter().any(|&(o, c)| o == prev && c == next)
            {
                return Some(Change::delete(pos - 1, pos + 1));
            }
            let line = rope.char_to_line(pos);
            let line_start = rope.line_to_char(line);
            let before = rope.slice(line_start..pos).to_string();
            if !before.is_empty() && before.chars().all(|c| c == ' ') && indent.unit != "\t" {
                let width = indent.unit.chars().count().max(1);
                let n = before.chars().count();
                let remove = match n % width {
                    0 => width,
                    m => m,
                };
                return Some(Change::delete(pos - remove, pos));
            }
            Some(Change::delete(pos - 1, pos))
        })
        .collect();
    let cs = ChangeSet::new(changes);
    let selection = sel.map(&cs).transform(|r| Range::point(r.head));
    Edited { changes: cs, selection }
}

/// Delete: deletes selections or the char after each caret.
pub fn delete_forward(rope: &Rope, sel: &Selection) -> Edited {
    let len = rope.len_chars();
    let changes = sel
        .ranges()
        .iter()
        .filter_map(|r| {
            if !r.is_empty() {
                Some(Change::delete(r.start(), r.end()))
            } else {
                (r.head < len).then(|| Change::delete(r.head, r.head + 1))
            }
        })
        .collect();
    let cs = ChangeSet::new(changes);
    let selection = sel.map(&cs).transform(|r| Range::point(r.head));
    Edited { changes: cs, selection }
}

/// Deletes from every caret to the word boundary on its left (`left`) or right.
pub fn delete_word(rope: &Rope, sel: &Selection, left: bool) -> Edited {
    let changes = sel
        .ranges()
        .iter()
        .map(|r| {
            if !r.is_empty() {
                Change::delete(r.start(), r.end())
            } else if left {
                Change::delete(word_left(rope, r.head), r.head)
            } else {
                Change::delete(r.head, word_right(rope, r.head))
            }
        })
        .collect();
    let cs = ChangeSet::new(changes);
    let selection = sel.map(&cs).transform(|r| Range::point(r.head));
    Edited { changes: cs, selection }
}

/// Lines covered by each range (a selection ending at column 0 does not take that line),
/// merged and sorted.
pub fn selected_line_blocks(rope: &Rope, sel: &Selection) -> Vec<(usize, usize)> {
    let mut blocks: Vec<(usize, usize)> = Vec::new();
    for r in sel.ranges() {
        let first = rope.char_to_line(r.start());
        let mut last = rope.char_to_line(r.end());
        if !r.is_empty() && last > first && rope.line_to_char(last) == r.end() {
            last -= 1;
        }
        match blocks.last_mut() {
            Some(b) if first <= b.1 + 1 => b.1 = b.1.max(last),
            _ => blocks.push((first, last)),
        }
    }
    blocks
}

/// Char range of lines `first..=last` including the final line break (or the one before
/// when the block is the last line of the document).
fn block_range(rope: &Rope, first: usize, last: usize) -> (usize, usize) {
    let start = rope.line_to_char(first);
    let end = if last + 1 < rope.len_lines() { rope.line_to_char(last + 1) } else { rope.len_chars() };
    (start, end)
}

/// Ctrl+D: duplicates selections, or the line under each caret.
pub fn duplicate(rope: &Rope, sel: &Selection) -> Edited {
    if sel.ranges().iter().all(|r| !r.is_empty()) {
        let changes = sel.ranges().iter().map(|r| Change::insert(r.end(), rope.slice(r.start()..r.end()).to_string())).collect();
        let cs = ChangeSet::new(changes);
        let selection = sel.transform(|r| {
            let shift: usize = sel.ranges().iter().filter(|o| o.start() < r.start()).map(|o| o.len()).sum::<usize>() + r.len();
            Range::new(r.anchor + shift, r.head + shift)
        });
        return Edited { changes: cs, selection };
    }
    let blocks = selected_line_blocks(rope, sel);
    let mut changes = Vec::new();
    for &(first, last) in &blocks {
        let (start, end) = block_range(rope, first, last);
        let mut text = rope.slice(start..end).to_string();
        if !text.ends_with('\n') {
            text.insert(0, '\n');
            changes.push(Change::insert(end, text));
        } else {
            changes.push(Change::insert(end, text));
        }
    }
    let cs = ChangeSet::new(changes);
    // Carets move down onto the copy, keeping their column.
    let selection = sel.transform(|r| {
        let mut shift = 0;
        for &(first, last) in &blocks {
            let (start, end) = block_range(rope, first, last);
            if start <= r.start() && r.start() <= end && (r.start() < end || end == rope.len_chars()) {
                shift += end - start + usize::from(!rope.slice(start..end).to_string().ends_with('\n'));
                break;
            }
            if end <= r.start() {
                shift += end - start;
            }
        }
        Range::new(r.anchor + shift, r.head + shift)
    });
    Edited { changes: cs, selection }
}

/// Deletes the lines touched by every range (Ctrl+Y in the Rider keymap).
pub fn delete_lines(rope: &Rope, sel: &Selection) -> Edited {
    let blocks = selected_line_blocks(rope, sel);
    let changes: Vec<Change> = blocks
        .iter()
        .map(|&(first, last)| {
            let (mut start, end) = block_range(rope, first, last);
            // The last line of the document takes the line break before it.
            if end == rope.len_chars() && start > 0 && !rope.slice(start..end).to_string().ends_with('\n') {
                start -= 1;
            }
            Change::delete(start, end)
        })
        .collect();
    let cs = ChangeSet::new(changes);
    let after = cs.len_after(rope.len_chars());
    let selection = sel.transform(|r| {
        let line = rope.char_to_line(r.head);
        let _col = r.head - rope.line_to_char(line);
        let p = cs.map_pos(rope.line_to_char(line), crate::change::Assoc::Before).min(after);
        Range::point(p)
    });
    Edited { changes: cs, selection }
}

/// Alt+Shift+Up/Down: moves the selected lines one line up or down.
pub fn move_lines(rope: &Rope, sel: &Selection, up: bool) -> Edited {
    let blocks = selected_line_blocks(rope, sel);
    let last_line = rope.len_lines() - 1;
    if (up && blocks[0].0 == 0) || (!up && blocks.last().expect("non-empty").1 >= last_line) {
        return Edited { changes: ChangeSet::default(), selection: sel.clone() };
    }
    let line_str = |l: usize| -> String {
        let start = rope.line_to_char(l);
        rope.slice(start..start + line_len(rope, l)).to_string()
    };
    let mut changes = Vec::new();
    let mut shifts: Vec<(usize, usize, isize)> = Vec::new();
    for &(first, last) in &blocks {
        let other = if up { first - 1 } else { last + 1 };
        let (range_first, range_last) = (first.min(other), last.max(other));
        let start = rope.line_to_char(range_first);
        let end = rope.line_to_char(range_last) + line_len(rope, range_last);
        let moved: Vec<String> = (first..=last).map(line_str).collect();
        let other_text = line_str(other);
        let mut lines = moved;
        if up { lines.push(other_text.clone()) } else { lines.insert(0, other_text.clone()) }
        changes.push(Change::new(start, end, lines.join("\n")));
        let step = other_text.chars().count() as isize + 1;
        let block_start = rope.line_to_char(first);
        let block_end = rope.line_to_char(last) + line_len(rope, last);
        shifts.push((block_start, block_end, if up { -step } else { step }));
    }
    let cs = ChangeSet::new(changes);
    let selection = sel.transform(|r| {
        let shift = shifts.iter().find(|(s, e, _)| *s <= r.start() && r.start() <= *e).map_or(0, |x| x.2);
        Range::new((r.anchor as isize + shift) as usize, (r.head as isize + shift) as usize)
    });
    Edited { changes: cs, selection }
}

/// Tab with a multi-line selection / Shift+Tab: indents or outdents the selected lines.
pub fn indent_lines(rope: &Rope, sel: &Selection, indent: &Indent, outdent: bool) -> Edited {
    let mut changes = Vec::new();
    for (first, last) in selected_line_blocks(rope, sel) {
        for line in first..=last {
            let start = rope.line_to_char(line);
            if outdent {
                let lead = line_indent(rope, line);
                let n = if lead.starts_with('\t') {
                    1
                } else {
                    lead.chars().take_while(|c| *c == ' ').count().min(indent.unit.chars().count().max(1))
                };
                if n > 0 {
                    changes.push(Change::delete(start, start + n));
                }
            } else if line_len(rope, line) > 0 {
                changes.push(Change::insert(start, indent.unit.clone()));
            }
        }
    }
    let cs = ChangeSet::new(changes);
    let selection = sel.transform(|r| {
        let (a, h) = (cs.map_pos(r.anchor, crate::change::Assoc::After), cs.map_pos(r.head, crate::change::Assoc::After));
        Range::new(a, h)
    });
    Edited { changes: cs, selection }
}

/// Tab without a multi-line selection: inserts spaces up to the next indent stop.
pub fn insert_tab(rope: &Rope, sel: &Selection, indent: &Indent) -> Edited {
    if indent.unit == "\t" {
        return replace_selections(sel, "\t", None);
    }
    let width = indent.unit.chars().count().max(1);
    let texts: Vec<String> = sel
        .ranges()
        .iter()
        .map(|r| {
            let col = visual_col(rope, r.start(), indent.tab_width);
            " ".repeat(width - col % width)
        })
        .collect();
    replace_selections(sel, "", Some(&texts))
}

/// Ctrl+/: toggles `prefix` line comments on the selected lines. Lines are commented at the
/// smallest indentation of the block (as Rider does when "comment at first column" is off).
pub fn toggle_line_comment(rope: &Rope, sel: &Selection, prefix: &str) -> Edited {
    let mut changes = Vec::new();
    for (first, last) in selected_line_blocks(rope, sel) {
        let lines: Vec<usize> = (first..=last).filter(|&l| !rope.line(l).to_string().trim().is_empty()).collect();
        let lines = if lines.is_empty() { vec![first] } else { lines };
        let all_commented = lines.iter().all(|&l| rope.line(l).to_string().trim_start().starts_with(prefix));
        let min_indent = lines.iter().map(|&l| line_indent(rope, l).chars().count()).min().unwrap_or(0);
        for &l in &lines {
            let start = rope.line_to_char(l);
            if all_commented {
                let lead = line_indent(rope, l).chars().count();
                let at = start + lead;
                let mut n = prefix.chars().count();
                if char_at(rope, at + n) == Some(' ') {
                    n += 1;
                }
                changes.push(Change::delete(at, at + n));
            } else {
                changes.push(Change::insert(start + min_indent, format!("{prefix} ")));
            }
        }
    }
    let cs = ChangeSet::new(changes);
    let selection = sel.transform(|r| {
        Range::new(cs.map_pos(r.anchor, crate::change::Assoc::After), cs.map_pos(r.head, crate::change::Assoc::After))
    });
    // A single caret moves to the next line, as in IntelliJ.
    let selection = if sel.len() == 1 && sel.primary().is_empty() {
        let after = {
            let mut r = rope.clone();
            cs.apply(&mut r);
            r
        };
        let line = after.char_to_line(selection.primary().head);
        if line + 1 < after.len_lines() {
            let col = selection.primary().head - after.line_to_char(line);
            Selection::point(after.line_to_char(line + 1) + col.min(line_len(&after, line + 1)))
        } else {
            selection
        }
    } else {
        selection
    };
    Edited { changes: cs, selection }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(text: &str, sel: Selection, f: impl Fn(&Rope, &Selection) -> Edited) -> (String, Selection) {
        let mut rope = Rope::from_str(text);
        let e = f(&rope, &sel);
        e.changes.apply(&mut rope);
        (rope.to_string(), e.selection)
    }

    #[test]
    fn brackets_pair_and_type_over() {
        let (t, s) = run("x", Selection::point(1), |r, s| type_char(r, s, '(', true));
        assert_eq!((t.as_str(), s.primary().head), ("x()", 2));
        let (t, s) = run("x()", Selection::point(2), |r, s| type_char(r, s, ')', true));
        assert_eq!((t.as_str(), s.primary().head), ("x()", 3));
        // No pairing before a word.
        let (t, _) = run("abc", Selection::point(0), |r, s| type_char(r, s, '(', true));
        assert_eq!(t, "(abc");
    }

    #[test]
    fn enter_between_braces_opens_a_block() {
        let (t, s) = run("    if (x) {}", Selection::point(12), |r, s| newline(r, s, &Indent::default()));
        assert_eq!(t, "    if (x) {\n        \n    }");
        assert_eq!(s.primary().head, 21);
        let (t, s) = run("    foo();", Selection::point(10), |r, s| newline(r, s, &Indent::default()));
        assert_eq!(t, "    foo();\n    ");
        assert_eq!(s.primary().head, 15);
    }

    #[test]
    fn backspace_removes_an_indent_level_and_pairs() {
        let (t, _) = run("        x", Selection::point(8), |r, s| backspace(r, s, &Indent::default()));
        assert_eq!(t, "    x");
        let (t, _) = run("f()", Selection::point(2), |r, s| backspace(r, s, &Indent::default()));
        assert_eq!(t, "f");
    }

    #[test]
    fn smart_home_toggles_between_indent_and_column_zero() {
        let rope = Rope::from_str("    abc");
        let s = move_selection(&rope, &Selection::point(6), Motion::LineStart, false, 4);
        assert_eq!(s.primary().head, 4);
        let s = move_selection(&rope, &s, Motion::LineStart, false, 4);
        assert_eq!(s.primary().head, 0);
    }

    #[test]
    fn vertical_moves_keep_the_goal_column() {
        let rope = Rope::from_str("abcdef\nab\nabcdef");
        let s = move_selection(&rope, &Selection::point(5), Motion::Down, false, 4);
        assert_eq!(s.primary().head, 9);
        let s = move_selection(&rope, &s, Motion::Down, false, 4);
        assert_eq!(s.primary().head, 15);
    }

    #[test]
    fn duplicate_and_delete_lines() {
        let (t, s) = run("a\nbc\nd", Selection::point(3), duplicate);
        assert_eq!(t, "a\nbc\nbc\nd");
        assert_eq!(s.primary().head, 6);
        let (t, _) = run("a\nbc\nd", Selection::point(6), duplicate);
        assert_eq!(t, "a\nbc\nd\nd");
        let (t, _) = run("a\nbc\nd", Selection::point(3), delete_lines);
        assert_eq!(t, "a\nd");
        let (t, _) = run("a\nbc\nd", Selection::point(5), delete_lines);
        assert_eq!(t, "a\nbc");
    }

    #[test]
    fn move_lines_swaps_with_neighbours() {
        let (t, s) = run("a\nb\nc", Selection::point(2), |r, s| move_lines(r, s, true));
        assert_eq!(t, "b\na\nc");
        assert_eq!(s.primary().head, 0);
        let (t, s) = run("a\nb\nc", Selection::point(2), |r, s| move_lines(r, s, false));
        assert_eq!(t, "a\nc\nb");
        assert_eq!(s.primary().head, 4);
    }

    #[test]
    fn comments_toggle_at_the_block_indent() {
        let sel = Selection::single(Range::new(0, 14));
        let (t, _) = run("  a\n    b\n  c\n", sel.clone(), |r, s| toggle_line_comment(r, s, "//"));
        assert_eq!(t, "  // a\n  //   b\n  // c\n");
        let (t, _) = run(&t, Selection::single(Range::new(0, 23)), |r, s| toggle_line_comment(r, s, "//"));
        assert_eq!(t, "  a\n    b\n  c\n");
    }

    #[test]
    fn next_occurrence_adds_carets() {
        let rope = Rope::from_str("foo bar foo foo");
        let s = select_next_occurrence(&rope, &Selection::point(1));
        assert_eq!(s.primary(), Range::new(0, 3));
        let s = select_next_occurrence(&rope, &s);
        assert_eq!(s.len(), 2);
        assert_eq!(s.primary(), Range::new(8, 11));
        assert_eq!(select_all_occurrences(&rope, &Selection::point(0)).len(), 3);
    }
}
