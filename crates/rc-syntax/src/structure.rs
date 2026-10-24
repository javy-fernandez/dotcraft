//! Structure from the tree: folding ranges and bracket matching.

use crate::lang::Lang;
use rc_text::Rope;
use tree_sitter::{Node, Tree};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FoldKind {
    Block,
    Comment,
    Imports,
    Region,
}

/// A foldable range: lines `start_line + 1 ..= end_line` hide behind `start_line`
/// (for braces, the closing line stays visible: `end_line` is the line before it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FoldRange {
    pub start_line: usize,
    pub end_line: usize,
    pub kind: FoldKind,
    /// Char column on `start_line` where the placeholder (`{...}`) starts.
    pub start_col: usize,
    /// Char column on `end_line` right after the folded text (what follows stays visible
    /// after the placeholder, e.g. the `);` of `});`).
    pub end_col: usize,
}

/// C# nodes that fold from their `{` to their `}`.
const CS_BLOCKS: &[&str] = &[
    "declaration_list",
    "block",
    "switch_body",
    "accessor_list",
    "enum_member_declaration_list",
    "initializer_expression",
    "switch_expression",
    "anonymous_object_creation_expression",
];

pub fn folding_ranges(lang: Lang, tree: &Tree, rope: &Rope) -> Vec<FoldRange> {
    let mut out = Vec::new();
    match lang {
        Lang::CSharp => {
            let mut regions: Vec<(usize, usize)> = Vec::new();
            let mut usings: Option<(usize, usize, usize)> = None;
            let mut comments: Option<(usize, usize, usize)> = None;
            walk(tree.root_node(), &mut |node| {
                let kind = node.kind();
                let (sr, er) = (node.start_position().row, node.end_position().row);
                if CS_BLOCKS.contains(&kind) && er > sr {
                    out.push(FoldRange { start_line: sr, end_line: er, kind: FoldKind::Block, start_col: char_col(rope, node), end_col: end_col(rope, node) });
                } else if kind == "using_directive" && node.parent().is_some_and(|p| p.kind() == "compilation_unit") {
                    usings = Some(match usings {
                        Some((s, _, c)) => (s, er, c),
                        None => (sr, er, "using ".len()),
                    });
                } else if kind == "comment" {
                    if er > sr {
                        out.push(FoldRange { start_line: sr, end_line: er, kind: FoldKind::Comment, start_col: char_col(rope, node), end_col: end_col(rope, node) });
                    } else {
                        // Consecutive `//` lines fold together.
                        comments = match comments {
                            Some((s, e, c)) if e + 1 == sr => Some((s, sr, c)),
                            Some((s, e, c)) => {
                                if e > s {
                                    out.push(FoldRange { start_line: s, end_line: e, kind: FoldKind::Comment, start_col: c, end_col: line_end(rope, e) });
                                }
                                Some((sr, sr, char_col(rope, node)))
                            }
                            None => Some((sr, sr, char_col(rope, node))),
                        };
                    }
                } else if kind == "preproc_region" {
                    regions.push((sr, char_col(rope, node)));
                } else if kind == "preproc_endregion"
                    && let Some((start, col)) = regions.pop()
                {
                    out.push(FoldRange { start_line: start, end_line: sr, kind: FoldKind::Region, start_col: col, end_col: line_end(rope, sr) });
                }
            });
            if let Some((s, e, c)) = comments
                && e > s
            {
                out.push(FoldRange { start_line: s, end_line: e, kind: FoldKind::Comment, start_col: c, end_col: line_end(rope, e) });
            }
            if let Some((s, e, c)) = usings
                && e > s
            {
                out.push(FoldRange { start_line: s, end_line: e, kind: FoldKind::Imports, start_col: c, end_col: line_end(rope, e) });
            }
        }
        Lang::Xml => walk(tree.root_node(), &mut |node| {
            let (sr, er) = (node.start_position().row, node.end_position().row);
            if (node.kind() == "element" || node.kind() == "Comment") && er > sr {
                out.push(FoldRange { start_line: sr, end_line: er, kind: FoldKind::Block, start_col: char_col(rope, node), end_col: end_col(rope, node) });
            }
        }),
        Lang::Json => walk(tree.root_node(), &mut |node| {
            let (sr, er) = (node.start_position().row, node.end_position().row);
            if (node.kind() == "object" || node.kind() == "array") && er > sr {
                out.push(FoldRange { start_line: sr, end_line: er, kind: FoldKind::Block, start_col: char_col(rope, node), end_col: end_col(rope, node) });
            }
        }),
        Lang::Markdown => walk(tree.root_node(), &mut |node| {
            let (sr, er) = (node.start_position().row, node.end_position().row);
            if (node.kind() == "section" || node.kind() == "fenced_code_block") && er > sr + 1 {
                let end = if node.end_position().column == 0 { er - 1 } else { er };
                if end > sr {
                    out.push(FoldRange { start_line: sr, end_line: end, kind: FoldKind::Block, start_col: line_end(rope, sr), end_col: line_end(rope, end) });
                }
            }
        }),
        Lang::Plain => {}
    }
    out.sort_by_key(|f| (f.start_line, std::cmp::Reverse(f.end_line)));
    // One fold per start line (the outermost), like IntelliJ.
    out.dedup_by_key(|f| f.start_line);
    out
}

fn line_end(rope: &Rope, line: usize) -> usize {
    rc_text::buffer::line_len(rope, line)
}

fn end_col(rope: &Rope, node: Node) -> usize {
    let p = node.end_position();
    let line_start = rope.line_to_byte(p.row.min(rope.len_lines().saturating_sub(1)));
    let byte = (line_start + p.column).min(rope.len_bytes());
    rope.byte_to_char(byte) - rope.line_to_char(rope.byte_to_line(byte))
}

fn char_col(rope: &Rope, node: Node) -> usize {
    let p = node.start_position();
    let line_start = rope.line_to_byte(p.row.min(rope.len_lines().saturating_sub(1)));
    let byte = (line_start + p.column).min(rope.len_bytes());
    rope.byte_to_char(byte) - rope.line_to_char(rope.byte_to_line(byte))
}

/// Pre-order walk of the nodes that may hold a fold: a node on a single line cannot
/// contain a multi-line one, so its children are skipped (the node itself is visited).
fn walk(root: Node, f: &mut impl FnMut(Node)) {
    let mut cursor = root.walk();
    loop {
        let node = cursor.node();
        f(node);
        if node.end_position().row > node.start_position().row && cursor.goto_first_child() {
            continue;
        }
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() {
                return;
            }
        }
    }
}

const BRACKETS: [(&str, &str); 4] = [("{", "}"), ("(", ")"), ("[", "]"), ("<", ">")];

/// The bracket matching the one at (or just before) char `pos`: returns both char offsets.
/// Uses the tree so brackets inside strings and comments are ignored.
pub fn matching_bracket(tree: &Tree, rope: &Rope, pos: usize) -> Option<(usize, usize)> {
    let try_at = |p: usize| -> Option<(usize, usize)> {
        if p >= rope.len_chars() {
            return None;
        }
        let byte = rope.char_to_byte(p);
        let node = tree.root_node().descendant_for_byte_range(byte, byte + 1)?;
        if node.is_named() || node.start_byte() != byte {
            return None;
        }
        let kind = node.kind();
        let (open, close) = BRACKETS.iter().find(|(o, c)| *o == kind || *c == kind)?;
        if *open == "<" && node.parent().is_none_or(|p| p.kind() != "type_argument_list" && p.kind() != "type_parameter_list") {
            return None;
        }
        let parent = node.parent()?;
        let mut cursor = parent.walk();
        let siblings: Vec<Node> = parent.children(&mut cursor).collect();
        let idx = siblings.iter().position(|n| n.id() == node.id())?;
        let other = if kind == *open {
            siblings[idx + 1..].iter().find(|n| n.kind() == *close)?
        } else {
            siblings[..idx].iter().rev().find(|n| n.kind() == *open)?
        };
        Some((p, rope.byte_to_char(other.start_byte())))
    };
    try_at(pos).or_else(|| pos.checked_sub(1).and_then(try_at))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse_now;

    #[test]
    fn csharp_folds_blocks_usings_regions_and_comments() {
        let src = "using A;\nusing B;\nnamespace N\n{\n    #region R\n    class C\n    {\n        // a\n        // b\n        void M()\n        {\n        }\n    }\n    #endregion\n}\n";
        let rope = Rope::from_str(src);
        let tree = parse_now(Lang::CSharp, &rope, None).unwrap();
        let folds = folding_ranges(Lang::CSharp, &tree, &rope);
        let lines: Vec<(usize, usize, FoldKind)> = folds.iter().map(|f| (f.start_line, f.end_line, f.kind)).collect();
        assert!(lines.contains(&(0, 1, FoldKind::Imports)), "{lines:?}");
        assert!(lines.contains(&(3, 14, FoldKind::Block)), "{lines:?}");
        assert!(lines.contains(&(4, 13, FoldKind::Region)), "{lines:?}");
        assert!(lines.contains(&(7, 8, FoldKind::Comment)), "{lines:?}");
        assert!(lines.contains(&(10, 11, FoldKind::Block)), "{lines:?}");
    }

    #[test]
    fn brackets_match_outside_strings() {
        let src = "class C { void M() { var s = \"{\"; } }";
        let rope = Rope::from_str(src);
        let tree = parse_now(Lang::CSharp, &rope, None).unwrap();
        assert_eq!(matching_bracket(&tree, &rope, 8), Some((8, 36)));
        assert_eq!(matching_bracket(&tree, &rope, 37), Some((36, 8)));
        assert_eq!(matching_bracket(&tree, &rope, 16), Some((16, 17)));
        assert_eq!(matching_bracket(&tree, &rope, 30), None);
    }
}
