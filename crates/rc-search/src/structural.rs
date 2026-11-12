//! C# syntax-tree search with named placeholders and optional constraints.

use std::collections::BTreeMap;

use rc_syntax::tree_sitter::{Node, Tree};
use rc_syntax::{Lang, parse_now};
use rc_text::Rope;
use regex::Regex;

const PREFIX: &str = "__dotcraft_placeholder_";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeKind {
    Any,
    Identifier,
    Expression,
    Statement,
    Type,
}

#[derive(Clone, Debug)]
pub struct Constraint {
    pub kind: NodeKind,
    pub text_regex: Option<String>,
}

impl Default for Constraint {
    fn default() -> Self {
        Self {
            kind: NodeKind::Any,
            text_regex: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StructuralMatch {
    pub start: usize,
    pub end: usize,
    pub captures: BTreeMap<String, String>,
}

pub struct Pattern {
    wrapped: String,
    tree: Tree,
    mode: RootMode,
    names: Vec<String>,
    constraints: BTreeMap<String, (NodeKind, Option<Regex>)>,
}

#[derive(Clone, Copy)]
enum RootMode {
    Expression,
    Statement,
    Member,
    TopLevel,
}

impl Pattern {
    pub fn compile(
        query: &str,
        constraints: &BTreeMap<String, Constraint>,
    ) -> Result<Self, String> {
        let query = query.trim();
        if query.is_empty() {
            return Err("Pattern cannot be empty".into());
        }
        let placeholders = Regex::new(r"\$([A-Za-z_][A-Za-z0-9_]*)\$").unwrap();
        let mut names = Vec::new();
        let code = placeholders
            .replace_all(query, |caps: &regex::Captures<'_>| {
                let name = caps[1].to_string();
                if !names.contains(&name) {
                    names.push(name.clone());
                }
                format!("{PREFIX}{name}__")
            })
            .into_owned();
        if code.contains('$') {
            return Err("Use $name$ for each placeholder".into());
        }
        let statement_candidate = (
            format!(
                "class __DotCraftPattern {{ void __match() {{ {}{} }} }}",
                code,
                if code.ends_with(';') || code.ends_with('}') {
                    ""
                } else {
                    ";"
                }
            ),
            RootMode::Statement,
        );
        let member_candidate = (
            format!("class __DotCraftPattern {{ {code} }}"),
            RootMode::Member,
        );
        let top_candidate = (code.clone(), RootMode::TopLevel);
        let looks_member = [
            "public ",
            "private ",
            "protected ",
            "internal ",
            "static ",
            "virtual ",
            "override ",
            "abstract ",
            "event ",
            "class ",
            "struct ",
            "interface ",
        ]
        .iter()
        .any(|prefix| code.starts_with(prefix));
        let candidates = if looks_member {
            [member_candidate, statement_candidate, top_candidate]
        } else {
            [statement_candidate, member_candidate, top_candidate]
        };
        let (wrapped, tree, mode) = candidates
            .into_iter()
            .find_map(|(wrapped, candidate_mode)| {
                let tree = parse_now(Lang::CSharp, &Rope::from_str(&wrapped), None)?;
                if tree.root_node().has_error() {
                    return None;
                }
                let mode = if matches!(candidate_mode, RootMode::Statement) {
                    let statement = root_for(&tree, RootMode::Statement)?;
                    if statement.kind() == "expression_statement" && !code.ends_with(';') {
                        RootMode::Expression
                    } else {
                        RootMode::Statement
                    }
                } else {
                    candidate_mode
                };
                root_for(&tree, mode)?;
                Some((wrapped, tree, mode))
            })
            .ok_or("Pattern is not valid C# code")?;
        let mut compiled = BTreeMap::new();
        for name in &names {
            let constraint = constraints.get(name).cloned().unwrap_or_default();
            let regex = constraint
                .text_regex
                .as_deref()
                .filter(|text| !text.is_empty())
                .map(Regex::new)
                .transpose()
                .map_err(|error| format!("Invalid regex for {name}: {error}"))?;
            compiled.insert(name.clone(), (constraint.kind, regex));
        }
        Ok(Self {
            wrapped,
            tree,
            mode,
            names,
            constraints: compiled,
        })
    }

    pub fn names(&self) -> &[String] {
        &self.names
    }

    pub fn find(&self, source: &str, limit: usize) -> Vec<StructuralMatch> {
        let Some(tree) = parse_now(Lang::CSharp, &Rope::from_str(source), None) else {
            return Vec::new();
        };
        let Some(pattern) = self.root() else {
            return Vec::new();
        };
        let mut matches = Vec::new();
        self.walk(pattern, tree.root_node(), source, limit, &mut matches);
        matches
    }

    pub fn replace(
        &self,
        template: &str,
        captures: &BTreeMap<String, String>,
    ) -> Result<String, String> {
        let placeholders = Regex::new(r"\$([A-Za-z_][A-Za-z0-9_]*)\$").unwrap();
        let mut missing = None;
        let replaced = placeholders.replace_all(template, |caps: &regex::Captures<'_>| {
            captures.get(&caps[1]).cloned().unwrap_or_else(|| {
                missing = Some(caps[1].to_string());
                String::new()
            })
        });
        if let Some(name) = missing {
            return Err(format!("Unknown replacement placeholder: {name}"));
        }
        Ok(replaced.into_owned())
    }

    fn root(&self) -> Option<Node<'_>> {
        root_for(&self.tree, self.mode)
    }

    fn walk(
        &self,
        pattern: Node<'_>,
        node: Node<'_>,
        source: &str,
        limit: usize,
        out: &mut Vec<StructuralMatch>,
    ) {
        if out.len() >= limit {
            return;
        }
        let mut captures = BTreeMap::new();
        if self.matches(pattern, node, source, &mut captures) {
            out.push(StructuralMatch {
                start: node.start_byte(),
                end: node.end_byte(),
                captures,
            });
            return;
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.walk(pattern, child, source, limit, out);
            if out.len() >= limit {
                return;
            }
        }
    }

    fn matches(
        &self,
        pattern: Node<'_>,
        target: Node<'_>,
        source: &str,
        captures: &mut BTreeMap<String, String>,
    ) -> bool {
        let pattern_text = &self.wrapped[pattern.byte_range()];
        if pattern.kind() == "identifier"
            && pattern_text.starts_with(PREFIX)
            && pattern_text.ends_with("__")
        {
            let name = &pattern_text[PREFIX.len()..pattern_text.len() - 2];
            let Some((kind, regex)) = self.constraints.get(name) else {
                return false;
            };
            let target_text = &source[target.byte_range()];
            if !kind_accepts(*kind, target.kind())
                || regex
                    .as_ref()
                    .is_some_and(|regex| !regex.is_match(target_text))
            {
                return false;
            }
            return match captures.get(name) {
                Some(previous) => previous == target_text,
                None => {
                    captures.insert(name.into(), target_text.into());
                    true
                }
            };
        }
        if pattern.kind() != target.kind() {
            return false;
        }
        let mut pattern_cursor = pattern.walk();
        let mut target_cursor = target.walk();
        let pchildren = pattern
            .named_children(&mut pattern_cursor)
            .collect::<Vec<_>>();
        let tchildren = target
            .named_children(&mut target_cursor)
            .collect::<Vec<_>>();
        if pchildren.len() != tchildren.len() {
            return false;
        }
        if pchildren.is_empty() {
            return pattern_text == &source[target.byte_range()];
        }
        let mut pstart = pattern.start_byte();
        let mut tstart = target.start_byte();
        for (p, t) in pchildren.iter().zip(tchildren.iter()) {
            if compact(&self.wrapped[pstart..p.start_byte()])
                != compact(&source[tstart..t.start_byte()])
                || !self.matches(*p, *t, source, captures)
            {
                return false;
            }
            pstart = p.end_byte();
            tstart = t.end_byte();
        }
        compact(&self.wrapped[pstart..pattern.end_byte()])
            == compact(&source[tstart..target.end_byte()])
    }
}

fn compact(input: &str) -> String {
    input.chars().filter(|ch| !ch.is_whitespace()).collect()
}

fn root_for(tree: &Tree, mode: RootMode) -> Option<Node<'_>> {
    let root = tree.root_node();
    if matches!(mode, RootMode::TopLevel) {
        return root.named_child(0);
    }
    let class = root.named_child(0)?;
    let body = class.child_by_field_name("body")?;
    let member = body.named_child(0)?;
    if matches!(mode, RootMode::Member) {
        return Some(member);
    }
    let block = member.child_by_field_name("body")?;
    let statement = block.named_child(0)?;
    if matches!(mode, RootMode::Statement) {
        Some(statement)
    } else {
        statement.named_child(0)
    }
}

fn kind_accepts(expected: NodeKind, actual: &str) -> bool {
    match expected {
        NodeKind::Any => true,
        NodeKind::Identifier => actual == "identifier",
        NodeKind::Expression => {
            actual == "identifier"
                || actual.ends_with("_expression")
                || actual.ends_with("_literal")
                || matches!(
                    actual,
                    "invocation_expression" | "string_literal" | "integer_literal"
                )
        }
        NodeKind::Statement => actual.ends_with("_statement"),
        NodeKind::Type => {
            actual == "identifier" || actual.ends_with("_type") || actual == "generic_name"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_call_shape_and_replaces_captured_expression() {
        let pattern = Pattern::compile("Console.WriteLine($value$)", &BTreeMap::new()).unwrap();
        let source = "class C { void M() { Console.WriteLine(a + b); Console.ReadLine(); } }";
        let found = pattern.find(source, 10);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].captures["value"], "a + b");
        assert_eq!(
            pattern.replace("Log($value$)", &found[0].captures).unwrap(),
            "Log(a + b)"
        );
    }

    #[test]
    fn constraints_and_repeated_placeholders_filter_matches() {
        let constraints = BTreeMap::from([(
            "value".into(),
            Constraint {
                kind: NodeKind::Identifier,
                text_regex: Some("^good$".into()),
            },
        )]);
        let pattern = Pattern::compile("Equal($value$, $value$)", &constraints).unwrap();
        let source = "class C { void M() { Equal(good, good); Equal(good, bad); Equal(1, 1); } }";
        assert_eq!(pattern.find(source, 10).len(), 1);
    }

    #[test]
    fn statement_pattern_distinguishes_syntax_and_rejects_bad_constraints() {
        let pattern = Pattern::compile("return $value$;", &BTreeMap::new()).unwrap();
        let source = "class C { int A() { return 1 + 2; } void B() { Foo(1); } }";
        let hits = pattern.find(source, 10);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].captures["value"], "1 + 2");
        let bad = BTreeMap::from([(
            "value".into(),
            Constraint {
                kind: NodeKind::Any,
                text_regex: Some("[".into()),
            },
        )]);
        assert!(Pattern::compile("return $value$;", &bad).is_err());
    }

    #[test]
    fn matches_member_and_type_declarations() {
        let source = "class Number { public int Double(int value) { return value + value; } }";
        let method = Pattern::compile(
            "public int $name$(int $arg$) { return $value$; }",
            &BTreeMap::new(),
        )
        .unwrap();
        let hits = method.find(source, 10);
        assert_eq!(
            hits.len(),
            1,
            "pattern: {}",
            method.root().unwrap().to_sexp()
        );
        assert_eq!(hits[0].captures["name"], "Double");
        assert_eq!(hits[0].captures["value"], "value + value");
        let ty = Pattern::compile(
            "class $name$ { public int Double(int value) { return value + value; } }",
            &BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(ty.find(source, 10)[0].captures["name"], "Number");
    }
}
