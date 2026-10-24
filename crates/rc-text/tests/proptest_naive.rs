//! Property tests: the rope buffer must behave exactly like a naive `String`.

use proptest::prelude::*;
use rc_text::commands::{self, Indent, Motion};
use rc_text::{Buffer, Change, ChangeSet, EditKind, Range, Rope, Selection, Utf16Pos};

fn text_strategy() -> impl Strategy<Value = String> {
    proptest::string::string_regex("[a-cñ😀 {}()\n\t]{0,40}").unwrap()
}

/// Non-overlapping changes, as (start, len, text) relative fractions of the document.
fn changes_strategy() -> impl Strategy<Value = Vec<(f64, f64, String)>> {
    proptest::collection::vec((0.0..1.0f64, 0.0..0.3f64, proptest::string::string_regex("[xñ😀\n]{0,4}").unwrap()), 1..4)
}

fn build(len: usize, raw: &[(f64, f64, String)]) -> ChangeSet {
    let mut cuts: Vec<(usize, usize, String)> = raw
        .iter()
        .map(|(s, l, t)| {
            let start = (s * len as f64) as usize;
            let end = (start + (l * len as f64) as usize).min(len);
            (start, end, t.clone())
        })
        .collect();
    cuts.sort_by_key(|c| c.0);
    let mut out = Vec::new();
    let mut last_end = 0;
    for (s, e, t) in cuts {
        if s < last_end {
            continue;
        }
        out.push(Change::new(s, e, t));
        last_end = e.max(s);
        if e == s {
            last_end = s + 1;
        }
    }
    ChangeSet::new(out)
}

fn naive_apply(s: &str, cs: &ChangeSet) -> String {
    let mut chars: Vec<char> = s.chars().collect();
    for c in cs.changes().iter().rev() {
        chars.splice(c.start..c.end, c.text.chars());
    }
    chars.into_iter().collect()
}

proptest! {
    #[test]
    fn edits_match_a_string_and_undo_restores_everything(text in text_strategy(), steps in proptest::collection::vec(changes_strategy(), 1..6)) {
        let mut buf = Buffer::new(&text);
        let mut naive = buf.rope().to_string();
        let original = naive.clone();
        let mut history = vec![naive.clone()];
        for raw in &steps {
            let cs = build(buf.len_chars(), raw);
            if cs.is_empty() { continue; }
            let before = buf.rope().clone();
            let edits = buf.apply(&cs, &Selection::point(0), &Selection::point(0), EditKind::Other);
            naive = naive_apply(&naive, &cs);
            prop_assert_eq!(buf.rope().to_string(), naive.clone());
            history.push(naive.clone());
            // Edits replay on a copy of the old document, in order.
            let mut replay = before;
            for e in &edits {
                replay.remove(e.start_char..e.old_end_char);
                replay.insert(e.start_char, &e.text);
                prop_assert_eq!(replay.char_to_byte(e.new_end_char), e.new_end_byte);
            }
            prop_assert_eq!(replay.to_string(), naive.clone());
        }
        // Each `Other` step is its own undo step.
        for expected in history.iter().rev().skip(1) {
            buf.undo().unwrap();
            prop_assert_eq!(&buf.rope().to_string(), expected);
        }
        prop_assert_eq!(buf.rope().to_string(), original);
        prop_assert!(!buf.is_modified());
        while buf.redo().is_some() {}
        prop_assert_eq!(buf.rope().to_string(), naive);
    }

    #[test]
    fn utf16_round_trips_on_char_boundaries(text in text_strategy(), frac in 0.0..1.0f64) {
        let buf = Buffer::new(&text);
        let pos = (frac * buf.len_chars() as f64) as usize;
        let u = buf.char_to_utf16(pos);
        prop_assert_eq!(buf.utf16_to_char(u), pos);
        // Against a naive computation.
        let s = buf.rope().to_string();
        let prefix: String = s.chars().take(pos).collect();
        let line = prefix.matches('\n').count() as u32;
        let col = prefix.rsplit('\n').next().unwrap().encode_utf16().count() as u32;
        prop_assert_eq!(u, Utf16Pos { line, character: col });
    }

    #[test]
    fn mapped_positions_stay_in_bounds(text in text_strategy(), raw in changes_strategy(), frac in 0.0..1.0f64) {
        let rope = Rope::from_str(&text);
        let cs = build(rope.len_chars(), &raw);
        let pos = (frac * rope.len_chars() as f64) as usize;
        let after = cs.len_after(rope.len_chars());
        for assoc in [rc_text::Assoc::Before, rc_text::Assoc::After] {
            prop_assert!(cs.map_pos(pos, assoc) <= after);
        }
    }

    #[test]
    fn commands_never_panic_and_keep_selections_valid(text in text_strategy(), a in 0.0..1.0f64, b in 0.0..1.0f64, op in 0usize..14) {
        let rope = Rope::from_str(&text);
        let len = rope.len_chars();
        let (a, b) = ((a * len as f64) as usize, (b * len as f64) as usize);
        let sel = Selection::new([Range::new(a, b), Range::point(len)], 0);
        let ind = Indent::default();
        let edited = match op {
            0 => commands::type_char(&rope, &sel, '(', true),
            1 => commands::type_char(&rope, &sel, '}', true),
            2 => commands::newline(&rope, &sel, &ind),
            3 => commands::backspace(&rope, &sel, &ind),
            4 => commands::delete_forward(&rope, &sel),
            5 => commands::delete_word(&rope, &sel, true),
            6 => commands::duplicate(&rope, &sel),
            7 => commands::delete_lines(&rope, &sel),
            8 => commands::move_lines(&rope, &sel, true),
            9 => commands::move_lines(&rope, &sel, false),
            10 => commands::indent_lines(&rope, &sel, &ind, false),
            11 => commands::indent_lines(&rope, &sel, &ind, true),
            12 => commands::toggle_line_comment(&rope, &sel, "//"),
            _ => commands::insert_tab(&rope, &sel, &ind),
        };
        let mut after = rope.clone();
        edited.changes.apply(&mut after);
        for r in edited.selection.ranges() {
            prop_assert!(r.anchor <= after.len_chars() && r.head <= after.len_chars());
        }
        for m in [Motion::Up, Motion::Down, Motion::WordLeft, Motion::WordRight, Motion::LineStart, Motion::LineEnd] {
            let s = commands::move_selection(&after, &edited.selection, m, true, 4);
            for r in s.ranges() {
                prop_assert!(r.head <= after.len_chars());
            }
        }
    }

    #[test]
    fn move_lines_up_then_down_is_identity(text in text_strategy(), frac in 0.0..1.0f64) {
        let mut rope = Rope::from_str(&text);
        let pos = (frac * rope.len_chars() as f64) as usize;
        let sel = Selection::point(pos);
        let up = commands::move_lines(&rope, &sel, true);
        if up.changes.is_empty() { return Ok(()); }
        let original = rope.to_string();
        up.changes.apply(&mut rope);
        let down = commands::move_lines(&rope, &up.selection, false);
        down.changes.apply(&mut rope);
        prop_assert_eq!(rope.to_string(), original);
        prop_assert_eq!(down.selection.primary().head, pos);
    }
}
