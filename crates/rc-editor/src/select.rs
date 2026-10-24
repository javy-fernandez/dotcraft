//! Extend / shrink selection (⌥↑ / ⌥↓, Ctrl+W): grows through the word, the syntax nodes
//! around it and their contents, and shrinks back through the same steps.

use crate::state::EditorState;
use rc_text::{Range, Selection, commands};

pub(crate) fn extend(state: &mut EditorState) -> Selection {
    let rope = state.buffer.rope().clone();
    let sel = state.selection.clone();
    let r = sel.primary();
    let next = next_range(state, &rope, r);
    if let Some(n) = next {
        state.select_stack.push(r);
        Selection::single(n)
    } else {
        sel
    }
}

pub(crate) fn shrink(state: &mut EditorState) -> Selection {
    let r = state.selection.primary();
    while let Some(prev) = state.select_stack.pop() {
        if prev.start() >= r.start() && prev.end() <= r.end() && prev != r {
            return Selection::single(prev);
        }
    }
    state.selection.clone()
}

fn next_range(state: &EditorState, rope: &rc_text::Rope, r: Range) -> Option<Range> {
    if r.is_empty() {
        let (s, e) = commands::word_at(rope, r.head);
        if s < e {
            return Some(Range::new(s, e));
        }
    }
    let tree = state.syntax.tree()?;
    let (sb, eb) = (rope.char_to_byte(r.start()), rope.char_to_byte(r.end()));
    let mut node = tree.root_node().descendant_for_byte_range(sb, eb)?;
    loop {
        let (ns, ne) = (node.start_byte(), node.end_byte());
        if (ns < sb || ne > eb) && ns <= sb && ne >= eb {
            // Inside brackets, first select the contents (without the brackets).
            let first = node.child(0);
            let last = node.child(node.child_count().saturating_sub(1));
            if let (Some(f), Some(l)) = (first, last)
                && node.child_count() >= 3
                && matches!(f.kind(), "{" | "(" | "[")
            {
                let inner = (f.end_byte(), l.start_byte());
                let trimmed = trim_ws(rope, inner.0, inner.1);
                if trimmed.0 <= sb && trimmed.1 >= eb && (trimmed.0 < sb || trimmed.1 > eb) {
                    return Some(Range::new(rope.byte_to_char(trimmed.0), rope.byte_to_char(trimmed.1)));
                }
            }
            return Some(Range::new(rope.byte_to_char(ns), rope.byte_to_char(ne)));
        }
        node = node.parent()?;
    }
}

fn trim_ws(rope: &rc_text::Rope, mut s: usize, mut e: usize) -> (usize, usize) {
    while s < e && rope.byte(s).is_ascii_whitespace() {
        s += 1;
    }
    while e > s && rope.byte(e - 1).is_ascii_whitespace() {
        e -= 1;
    }
    (s, e)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extend_grows_word_then_contents_then_node_and_shrinks_back() {
        let mut s = EditorState::new(1, Some("a.cs".into()), "class C { void M() { Foo(bar, baz); } }");
        s.parse_blocking();
        let pos = s.buffer.rope().to_string().find("bar").unwrap();
        s.selection = Selection::point(pos + 1);
        let texts: Vec<String> = (0..4)
            .map(|_| {
                s.selection = extend(&mut s);
                s.selected_text()
            })
            .collect();
        assert_eq!(texts[0], "bar");
        assert_eq!(texts[1], "bar, baz");
        assert!(texts[2].contains("(bar, baz)"), "{texts:?}");
        s.selection = shrink(&mut s);
        s.selection = shrink(&mut s);
        assert_eq!(s.selected_text(), "bar, baz");
    }
}
