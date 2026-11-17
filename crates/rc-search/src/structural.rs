//! C# structural search: a pattern is C# code with `$name$` placeholders, matched against the
//! syntax tree token by token, so whitespace, line breaks and comments never matter.
//!
//! - A placeholder stands for one node (an expression, a type, an identifier…). Written alone
//!   as an argument (`Foo($args$)`) or as a statement (`{ $body$; }`) it stands for the whole
//!   argument or statement, and its [`Count`] lets it match a run of them (none, one, many).
//! - A placeholder used twice must match the same code both times.
//! - Constraints narrow what a placeholder accepts: a [`NodeKind`] and a regular expression on
//!   the matched text.

use std::collections::BTreeMap;

use rc_syntax::tree_sitter::{Node, Tree};
use rc_syntax::{Lang, parse_now};
use rc_text::Rope;
use regex::Regex;

const PREFIX: &str = "__dotcraft_placeholder_";
const STATEMENT_PREFIX: &str = "__dotcraft_statement_";

/// What kind of syntax a placeholder accepts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NodeKind {
    #[default]
    Any,
    Identifier,
    Expression,
    Statement,
    Type,
}

impl NodeKind {
    pub const ALL: [NodeKind; 5] = [NodeKind::Any, NodeKind::Identifier, NodeKind::Expression, NodeKind::Statement, NodeKind::Type];
}

/// How many arguments or statements a placeholder written alone in a list matches.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Count {
    #[default]
    One,
    ZeroOrOne,
    OneOrMore,
    Any,
}

impl Count {
    pub const ALL: [Count; 4] = [Count::One, Count::ZeroOrOne, Count::OneOrMore, Count::Any];

    fn range(self) -> (usize, usize) {
        match self {
            Count::One => (1, 1),
            Count::ZeroOrOne => (0, 1),
            Count::OneOrMore => (1, usize::MAX),
            Count::Any => (0, usize::MAX),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Constraint {
    pub kind: NodeKind,
    pub text_regex: Option<String>,
    pub count: Count,
}

/// Why a pattern or a replacement template was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PatternError {
    Empty,
    /// A `$` that is neither a `$name$` placeholder nor an interpolated string.
    StrayDollar,
    NotCSharp,
    OnlyPlaceholders,
    Regex { name: String, error: String },
    /// The replacement uses a placeholder the pattern does not have.
    UnknownPlaceholder(String),
}

impl std::fmt::Display for PatternError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PatternError::Empty => write!(f, "The pattern is empty"),
            PatternError::StrayDollar => write!(f, "Write placeholders as $name$"),
            PatternError::NotCSharp => write!(f, "The pattern is not valid C# code"),
            PatternError::OnlyPlaceholders => write!(f, "The pattern needs code around its placeholders"),
            PatternError::Regex { name, error } => write!(f, "Invalid regex for ${name}$: {error}"),
            PatternError::UnknownPlaceholder(name) => write!(f, "The replacement uses ${name}$, which the pattern does not have"),
        }
    }
}

/// A match: the byte range in the searched text and the text each placeholder matched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StructuralMatch {
    pub start: usize,
    pub end: usize,
    pub captures: BTreeMap<String, String>,
}

struct Compiled {
    kind: NodeKind,
    regex: Option<Regex>,
    count: (usize, usize),
}

/// The pattern parsed in one context (expression, statement, member, file).
struct Alternative {
    wrapped: String,
    tree: Tree,
    mode: RootMode,
}

pub struct Pattern {
    alternatives: Vec<Alternative>,
    names: Vec<String>,
    constraints: BTreeMap<String, Compiled>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RootMode {
    Expression,
    Statement,
    Member,
    TopLevel,
}

type Captures = BTreeMap<String, String>;

fn placeholder_regex() -> Regex {
    Regex::new(r"\$([A-Za-z_][A-Za-z0-9_]*)\$").unwrap()
}

/// The placeholder names of `query`, in order of first use.
pub fn placeholder_names(query: &str) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for caps in placeholder_regex().captures_iter(query) {
        if !names.iter().any(|name| name == &caps[1]) {
            names.push(caps[1].to_string());
        }
    }
    names
}

impl Pattern {
    pub fn compile(query: &str, constraints: &BTreeMap<String, Constraint>) -> Result<Self, PatternError> {
        let query = query.trim();
        if query.is_empty() {
            return Err(PatternError::Empty);
        }
        let names = placeholder_names(query);
        let plain = placeholder_regex().replace_all(query, |caps: &regex::Captures<'_>| format!("{PREFIX}{}__", &caps[1])).into_owned();
        // A `$` left over is a typo, unless it starts an interpolated string.
        let stray = plain.match_indices('$').any(|(i, _)| {
            let rest = &plain[i + 1..];
            !(rest.starts_with('"') || rest.starts_with("@\"") || rest.starts_with('$'))
        });
        if stray {
            return Err(PatternError::StrayDollar);
        }
        // `$s$;` and `{ $s$ }` stand for statements; a bare identifier is no C# statement, so
        // they parse as calls with their own prefix. Initializers (`new[] { $x$ }`) look the same,
        // hence the fallback to the plain form.
        let statements = statement_form(query);
        let mut alternatives = if statements != plain { alternatives(&statements) } else { Vec::new() };
        if !alternatives.iter().all(|alt| statement_calls_are_statements(alt.tree.root_node(), &alt.wrapped)) {
            alternatives.clear();
        }
        if alternatives.is_empty() {
            alternatives = self::alternatives(&plain);
        }
        if alternatives.is_empty() {
            return Err(PatternError::NotCSharp);
        }
        let mut compiled = BTreeMap::new();
        for name in &names {
            let constraint = constraints.get(name).cloned().unwrap_or_default();
            let regex = constraint
                .text_regex
                .as_deref()
                .filter(|text| !text.is_empty())
                .map(|text| Regex::new(&format!("^(?:{text})$")))
                .transpose()
                .map_err(|error| PatternError::Regex { name: name.clone(), error: error.to_string() })?;
            compiled.insert(name.clone(), Compiled { kind: constraint.kind, regex, count: constraint.count.range() });
        }
        let pattern = Self { alternatives, names, constraints: compiled };
        if pattern.alternatives.iter().all(|alt| alt.root().is_some_and(|root| pattern.placeholder(alt, root).is_some())) {
            return Err(PatternError::OnlyPlaceholders);
        }
        Ok(pattern)
    }

    pub fn names(&self) -> &[String] {
        &self.names
    }

    /// The outermost matches in `source`, in text order, at most `limit`.
    pub fn find(&self, source: &str, limit: usize) -> Vec<StructuralMatch> {
        let Some(tree) = parse_now(Lang::CSharp, &Rope::from_str(source), None) else {
            return Vec::new();
        };
        let mut matches = Vec::new();
        self.walk(tree.root_node(), source, limit, &mut matches);
        matches
    }

    /// The replacement template with each `$name$` replaced by its capture.
    pub fn replace(&self, template: &str, captures: &BTreeMap<String, String>) -> Result<String, PatternError> {
        let mut missing = None;
        let replaced = placeholder_regex().replace_all(template, |caps: &regex::Captures<'_>| {
            captures.get(&caps[1]).cloned().unwrap_or_else(|| {
                missing = Some(caps[1].to_string());
                String::new()
            })
        });
        if let Some(name) = missing {
            return Err(PatternError::UnknownPlaceholder(name));
        }
        Ok(replaced.into_owned())
    }

    fn walk(&self, node: Node<'_>, source: &str, limit: usize, out: &mut Vec<StructuralMatch>) {
        if out.len() >= limit || node.kind() == "comment" {
            return;
        }
        for alt in &self.alternatives {
            let Some(root) = alt.root() else { continue };
            let mut captures = Captures::new();
            if self.matches(alt, root, node, source, &mut captures) {
                out.push(StructuralMatch { start: node.start_byte(), end: node.end_byte(), captures });
                return;
            }
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.walk(child, source, limit, out);
            if out.len() >= limit {
                return;
            }
        }
    }

    /// The placeholder `node` stands for: the placeholder token itself, an argument or attribute
    /// made of nothing but the placeholder, or a statement placeholder.
    fn placeholder<'a>(&'a self, alt: &Alternative, node: Node<'_>) -> Option<&'a str> {
        let children = significant_children(node);
        if children.is_empty() {
            return node.is_named().then(|| self.placeholder_token(alt, node, PREFIX)).flatten();
        }
        match (node.kind(), &children[..]) {
            ("argument" | "attribute", [only]) => self.placeholder_token(alt, *only, PREFIX),
            ("expression_statement", [call, semicolon]) if call.kind() == "invocation_expression" && semicolon.kind() == ";" => {
                let function = call.child_by_field_name("function")?;
                let arguments = call.child_by_field_name("arguments")?;
                (arguments.named_child_count() == 0).then(|| self.placeholder_token(alt, function, STATEMENT_PREFIX)).flatten()
            }
            _ => None,
        }
    }

    fn placeholder_token<'a>(&'a self, alt: &Alternative, node: Node<'_>, prefix: &str) -> Option<&'a str> {
        let text = &alt.wrapped[node.byte_range()];
        let name = text.strip_prefix(prefix)?.strip_suffix("__")?;
        self.names.iter().find(|known| *known == name).map(String::as_str)
    }

    /// Whether `target` may stand where the placeholder node `pattern` is.
    fn fits(&self, pattern: Node<'_>, name: &str, target: Node<'_>, source: &str) -> bool {
        let Some(constraint) = self.constraints.get(name) else { return false };
        let token = pattern.child_count() == 0;
        let shape = match pattern.kind() {
            _ if token => target.is_named() && target.kind() != "comment",
            "expression_statement" => is_statement(target.kind()),
            kind => target.kind() == kind,
        };
        if !shape {
            return false;
        }
        // An argument or attribute is judged by what it holds (`ref x` → `x`).
        let inner = if token { None } else { only_named_child(target) };
        let kind_ok = kind_accepts(constraint.kind, target.kind()) || inner.is_some_and(|inner| kind_accepts(constraint.kind, inner.kind()));
        kind_ok && constraint.regex.as_ref().is_none_or(|regex| regex.is_match(&source[target.byte_range()]))
    }

    fn bind(captures: &mut Captures, name: &str, text: &str) -> bool {
        match captures.get(name) {
            Some(previous) => compact(previous) == compact(text),
            None => {
                captures.insert(name.to_string(), text.to_string());
                true
            }
        }
    }

    fn matches(&self, alt: &Alternative, pattern: Node<'_>, target: Node<'_>, source: &str, captures: &mut Captures) -> bool {
        if let Some(name) = self.placeholder(alt, pattern) {
            return self.fits(pattern, name, target, source) && Self::bind(captures, name, &source[target.byte_range()]);
        }
        if pattern.kind() != target.kind() {
            return false;
        }
        let pchildren = significant_children(pattern);
        let tchildren = significant_children(target);
        if pchildren.is_empty() || tchildren.is_empty() {
            return pchildren.is_empty() && tchildren.is_empty() && alt.wrapped[pattern.byte_range()] == source[target.byte_range()];
        }
        self.match_list(alt, &pchildren, &tchildren, source, captures)
    }

    /// Matches a list of sibling nodes, letting counted placeholders take runs of siblings.
    fn match_list(&self, alt: &Alternative, pattern: &[Node<'_>], target: &[Node<'_>], source: &str, captures: &mut Captures) -> bool {
        let Some((&head, rest)) = pattern.split_first() else {
            return target.is_empty();
        };
        if let Some((name, (min, max))) = self.counted(alt, head) {
            for count in min..=max {
                let Some(taken) = take_run(target, count) else { break };
                let items: Vec<Node<'_>> = target[..taken].iter().copied().filter(|node| node.kind() != ",").collect();
                if !items.iter().all(|item| self.fits(head, name, *item, source)) {
                    continue;
                }
                let text = match (items.first(), items.last()) {
                    (Some(first), Some(last)) => &source[first.start_byte()..last.end_byte()],
                    _ => "",
                };
                let saved = captures.clone();
                if Self::bind(captures, name, text) {
                    if self.match_list(alt, rest, &target[taken..], source, captures) {
                        return true;
                    }
                    // `$args$, x` with no args: the comma goes too.
                    if count == 0
                        && rest.first().is_some_and(|next| next.kind() == ",")
                        && self.match_list(alt, &rest[1..], &target[taken..], source, captures)
                    {
                        return true;
                    }
                }
                *captures = saved;
            }
            return false;
        }
        // `x, $args$` with no args: the comma goes too.
        if head.kind() == ","
            && let Some(&next) = rest.first()
            && let Some((name, (0, _))) = self.counted(alt, next)
        {
            let saved = captures.clone();
            if Self::bind(captures, name, "") && self.match_list(alt, &rest[1..], target, source, captures) {
                return true;
            }
            *captures = saved;
        }
        let Some((&first, others)) = target.split_first() else {
            return false;
        };
        let saved = captures.clone();
        if self.matches(alt, head, first, source, captures) && self.match_list(alt, rest, others, source, captures) {
            return true;
        }
        *captures = saved;
        false
    }

    /// A placeholder whose count is not exactly one, with its count range.
    fn counted<'a>(&'a self, alt: &Alternative, node: Node<'_>) -> Option<(&'a str, (usize, usize))> {
        let name = self.placeholder(alt, node)?;
        let count = self.constraints.get(name)?.count;
        (count != (1, 1)).then_some((name, count))
    }
}

impl Alternative {
    fn root(&self) -> Option<Node<'_>> {
        root_for(&self.tree, self.mode)
    }
}

/// The contexts `code` parses in without errors: an expression or statement inside a method,
/// a member inside a class, a top-level declaration. Code that is both a local function or
/// variable and a member (`void $m$() { }`, `int $x$ = 0;`) is tried as both.
fn alternatives(code: &str) -> Vec<Alternative> {
    let parse = |wrapped: String, mode: RootMode| -> Option<Alternative> {
        let tree = parse_now(Lang::CSharp, &Rope::from_str(&wrapped), None)?;
        if tree.root_node().has_error() {
            return None;
        }
        let alt = Alternative { wrapped, tree, mode };
        alt.root()?;
        Some(alt)
    };
    let mut found = Vec::new();
    // Any expression, not just those C# allows as statements (`a + b`, `typeof(T)`, lambdas).
    if !code.ends_with(';')
        && let Some(alt) = parse(format!("class __DotCraftPattern {{ void __match() {{ var __dotcraft_value = {code}; }} }}"), RootMode::Expression)
    {
        found.push(alt);
        return found;
    }
    let terminator = if code.ends_with(';') || code.ends_with('}') { "" } else { ";" };
    let statement = parse(format!("class __DotCraftPattern {{ void __match() {{ {code}{terminator} }} }}"), RootMode::Statement);
    let mut try_member = true;
    if let Some(alt) = statement {
        let kind = alt.root().map(|root| root.kind().to_string()).unwrap_or_default();
        try_member = matches!(kind.as_str(), "local_function_statement" | "local_declaration_statement");
        found.push(alt);
    }
    if try_member && let Some(alt) = parse(format!("class __DotCraftPattern {{ {code} }}"), RootMode::Member) {
        let nested_type = alt.root().is_some_and(|root| root.kind().ends_with("_declaration") && !matches!(root.kind(), "method_declaration" | "field_declaration" | "property_declaration" | "constructor_declaration" | "event_field_declaration" | "event_declaration" | "indexer_declaration" | "operator_declaration" | "destructor_declaration"));
        found.push(alt);
        // A type declaration matches the same nodes as a member or at the top level.
        if nested_type {
            return found;
        }
    }
    if found.is_empty()
        && let Some(alt) = parse(code.to_string(), RootMode::TopLevel)
    {
        found.push(alt);
    }
    found
}

fn root_for(tree: &Tree, mode: RootMode) -> Option<Node<'_>> {
    let root = tree.root_node();
    if mode == RootMode::TopLevel {
        let children = significant_children(root);
        return (children.len() == 1).then(|| children[0]);
    }
    let class = root.named_child(0)?;
    let body = class.child_by_field_name("body")?;
    let members = significant_children(body).into_iter().filter(Node::is_named).collect::<Vec<_>>();
    let [member] = members[..] else { return None };
    if mode == RootMode::Member {
        return Some(member);
    }
    let block = member.child_by_field_name("body")?;
    let statements = significant_children(block).into_iter().filter(Node::is_named).collect::<Vec<_>>();
    let [statement] = statements[..] else { return None };
    if mode == RootMode::Statement {
        return Some(statement);
    }
    // `var __dotcraft_value = <expression>;`
    let declaration = statement.named_child(0)?;
    let declarators = significant_children(declaration).into_iter().filter(|node| node.kind() == "variable_declarator").collect::<Vec<_>>();
    let [declarator] = declarators[..] else { return None };
    let value = significant_children(declarator).into_iter().filter(Node::is_named).nth(1)?;
    (value.end_byte() + 1 == statement.end_byte()).then_some(value)
}

/// Whether every statement placeholder call in `node` ended up as a statement of its own
/// (`a ? b : $c$;` puts one inside an expression: that query means the plain form).
fn statement_calls_are_statements(node: Node<'_>, wrapped: &str) -> bool {
    if node.kind() == "identifier" && wrapped[node.byte_range()].starts_with(STATEMENT_PREFIX) {
        let call = node.parent().filter(|call| call.kind() == "invocation_expression");
        let statement = call.and_then(|call| call.parent()).filter(|statement| statement.kind() == "expression_statement");
        return statement.is_some_and(|statement| statement.named_child_count() == 1);
    }
    let mut cursor = node.walk();
    node.named_children(&mut cursor).all(|child| statement_calls_are_statements(child, wrapped))
}

/// `query` with its placeholders replaced, those in statement position (after `{`, `}`, `;`,
/// `)`, `:`, `else` or at the start, and before `;`, `}` or the end) as calls.
fn statement_form(query: &str) -> String {
    let mut out = String::new();
    let mut last = 0;
    for caps in placeholder_regex().captures_iter(query) {
        let whole = caps.get(0).unwrap();
        let before = query[..whole.start()].trim_end();
        let after = query[whole.end()..].trim_start();
        let opens = before.is_empty() || before.ends_with(['{', '}', ';', ')', ':']) || before.ends_with("else");
        let closes = after.is_empty() || after.starts_with([';', '}']);
        out.push_str(&query[last..whole.start()]);
        if opens && closes {
            out.push_str(&format!("{STATEMENT_PREFIX}{}__();", &caps[1]));
            // The statement's own `;` is part of the call now.
            last = whole.end() + if after.starts_with(';') { query[whole.end()..].find(';').unwrap() + 1 } else { 0 };
        } else {
            out.push_str(&format!("{PREFIX}{}__", &caps[1]));
            last = whole.end();
        }
    }
    out.push_str(&query[last..]);
    out
}

/// Children that take part in matching: every token and node except comments.
fn significant_children(node: Node<'_>) -> Vec<Node<'_>> {
    let mut cursor = node.walk();
    node.children(&mut cursor).filter(|child| child.kind() != "comment").collect()
}

fn only_named_child(node: Node<'_>) -> Option<Node<'_>> {
    let named: Vec<Node<'_>> = significant_children(node).into_iter().filter(Node::is_named).collect();
    match named[..] {
        [only] => Some(only),
        _ => None,
    }
}

/// How many leading siblings `count` list items take (named nodes, with the commas between).
fn take_run(target: &[Node<'_>], count: usize) -> Option<usize> {
    let mut taken = 0;
    for item in 0..count {
        if item > 0 && target.get(taken).is_some_and(|node| node.kind() == ",") {
            taken += 1;
        }
        let node = target.get(taken)?;
        if !node.is_named() {
            return None;
        }
        taken += 1;
    }
    Some(taken)
}

fn is_statement(kind: &str) -> bool {
    kind.ends_with("_statement") || kind == "block"
}

fn compact(input: &str) -> String {
    input.chars().filter(|ch| !ch.is_whitespace()).collect()
}

fn kind_accepts(expected: NodeKind, actual: &str) -> bool {
    match expected {
        NodeKind::Any => true,
        NodeKind::Identifier => actual == "identifier",
        NodeKind::Expression => {
            actual == "identifier" || actual.ends_with("_expression") || actual.ends_with("_literal") || matches!(actual, "this" | "base" | "generic_name" | "qualified_name")
        }
        NodeKind::Statement => is_statement(actual),
        NodeKind::Type => {
            matches!(actual, "identifier" | "generic_name" | "qualified_name" | "alias_qualified_name") || actual.ends_with("_type")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compile(query: &str) -> Pattern {
        Pattern::compile(query, &BTreeMap::new()).unwrap()
    }

    fn with(name: &str, constraint: Constraint) -> BTreeMap<String, Constraint> {
        BTreeMap::from([(name.to_string(), constraint)])
    }

    fn texts(pattern: &Pattern, source: &str) -> Vec<String> {
        pattern.find(source, 100).into_iter().map(|found| source[found.start..found.end].to_string()).collect()
    }

    #[test]
    fn matches_call_shape_and_replaces_captured_expression() {
        let pattern = compile("Console.WriteLine($value$)");
        let source = "class C { void M() { Console.WriteLine(a + b); Console.ReadLine(); } }";
        let found = pattern.find(source, 10);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].captures["value"], "a + b");
        assert_eq!(pattern.replace("Log($value$)", &found[0].captures).unwrap(), "Log(a + b)");
    }

    #[test]
    fn constraints_and_repeated_placeholders_filter_matches() {
        let constraints = with("value", Constraint { kind: NodeKind::Identifier, text_regex: Some("good".into()), ..Default::default() });
        let pattern = Pattern::compile("Equal($value$, $value$)", &constraints).unwrap();
        let source = "class C { void M() { Equal(good, good); Equal(good, bad); Equal(1, 1); Equal(goodness, goodness); } }";
        assert_eq!(texts(&pattern, source), ["Equal(good, good)"]);
    }

    #[test]
    fn statement_pattern_distinguishes_syntax_and_rejects_bad_constraints() {
        let pattern = compile("return $value$;");
        let source = "class C { int A() { return 1 + 2; } void B() { Foo(1); } }";
        let hits = pattern.find(source, 10);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].captures["value"], "1 + 2");
        let bad = with("value", Constraint { text_regex: Some("[".into()), ..Default::default() });
        assert!(Pattern::compile("return $value$;", &bad).is_err());
    }

    #[test]
    fn matches_member_and_type_declarations() {
        let source = "class Number { public int Double(int value) { return value + value; } }";
        let method = compile("public int $name$(int $arg$) { return $value$; }");
        let hits = method.find(source, 10);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].captures["name"], "Double");
        assert_eq!(hits[0].captures["value"], "value + value");
        let ty = compile("class $name$ { public int Double(int value) { return value + value; } }");
        assert_eq!(ty.find(source, 10)[0].captures["name"], "Number");
    }

    #[test]
    fn whitespace_line_breaks_and_comments_do_not_matter() {
        let pattern = compile("if ($a$ == null) { return; }");
        let source = "class C { void M(string s) {\n    if (s /* the name */ ==\n        null)\n    {\n        // nothing to do\n        return;\n    }\n} }";
        let hits = pattern.find(source, 10);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].captures["a"], "s");
        // A pattern with a comment still matches code without one.
        assert_eq!(compile("Foo(/* x */ $a$)").find("class C { void M() { Foo(1); } }", 10).len(), 1);
    }

    #[test]
    fn literal_tokens_must_be_identical() {
        let pattern = compile("Foo(\"a b\")");
        assert_eq!(texts(&pattern, "class C { void M() { Foo(\"a b\"); Foo(\"ab\"); Foo(\"a  b\"); } }"), ["Foo(\"a b\")"]);
    }

    #[test]
    fn matches_generics_lambdas_and_nested_expressions() {
        let source = "class C { void M() { var d = new Dictionary<string, List<int>>(); var q = items.Where(x => x.Age > 18 && x.Active).Select(x => x.Name); } }";
        let generic = compile("new Dictionary<$K$, $V$>()");
        let hits = generic.find(source, 10);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].captures["K"], "string");
        assert_eq!(hits[0].captures["V"], "List<int>");
        let lambda = compile("$list$.Where($x$ => $cond$)");
        let hits = lambda.find(source, 10);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].captures["list"], "items");
        assert_eq!(hits[0].captures["cond"], "x.Age > 18 && x.Active");
        // The receiver is itself a call: the match nests inside the outer Select.
        let select = compile("$q$.Select($f$)");
        assert_eq!(select.find(source, 10)[0].captures["q"], "items.Where(x => x.Age > 18 && x.Active)");
    }

    #[test]
    fn counted_argument_placeholders_take_runs_of_arguments() {
        let any = with("args", Constraint { count: Count::Any, ..Default::default() });
        let pattern = Pattern::compile("Log($args$)", &any).unwrap();
        let source = "class C { void M() { Log(); Log(a); Log(a, b + 1, \"c\"); } }";
        let captures: Vec<String> = pattern.find(source, 10).into_iter().map(|hit| hit.captures["args"].clone()).collect();
        assert_eq!(captures, ["", "a", "a, b + 1, \"c\""]);
        // Exactly one by default.
        assert_eq!(compile("Log($args$)").find(source, 10).len(), 1);
        let rest = with("rest", Constraint { count: Count::Any, ..Default::default() });
        let first = Pattern::compile("Log($first$, $rest$)", &rest).unwrap();
        let hits = first.find(source, 10);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].captures["first"], "a");
        assert_eq!(hits[0].captures["rest"], "");
        assert_eq!(first.replace("Write($rest$)", &hits[1].captures).unwrap(), "Write(b + 1, \"c\")");
        let one_or_more = with("args", Constraint { count: Count::OneOrMore, ..Default::default() });
        assert_eq!(Pattern::compile("Log($args$)", &one_or_more).unwrap().find(source, 10).len(), 2);
    }

    #[test]
    fn statement_placeholders_match_any_statements() {
        let body = with("body", Constraint { count: Count::Any, ..Default::default() });
        let pattern = Pattern::compile("try { $body$; } catch { }", &body).unwrap();
        let source = "class C { void M() { try { var a = 1; Foo(a); } catch { } try { } catch { } try { Foo(); } catch (Exception e) { } } }";
        let hits = pattern.find(source, 10);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].captures["body"], "var a = 1; Foo(a);");
        assert_eq!(hits[1].captures["body"], "");
        let statement = with("s", Constraint { kind: NodeKind::Statement, ..Default::default() });
        let single = Pattern::compile("if ($c$) $s$;", &statement).unwrap();
        assert_eq!(texts(&single, "class C { void M() { if (a) return; if (b) { x(); } } }"), ["if (a) return;", "if (b) { x(); }"]);
    }

    #[test]
    fn attributes_and_methods_without_modifiers_match_class_members() {
        let source = "class C {\n  [Obsolete(\"old\")]\n  public void Run() { Step(); Step(); }\n  void Helper() { }\n}";
        let body = with("body", Constraint { count: Count::Any, ..Default::default() });
        let attributed = Pattern::compile("[Obsolete($why$)] public void $m$() { $body$; }", &body).unwrap();
        let hits = attributed.find(source, 10);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].captures["why"], "\"old\"");
        assert_eq!(hits[0].captures["m"], "Run");
        // Parses as a local function too, but must find the class's method.
        let plain = compile("void $m$() { }");
        assert_eq!(plain.find(source, 10)[0].captures["m"], "Helper");
    }

    #[test]
    fn kind_constraints_check_the_matched_syntax() {
        let ty = with("T", Constraint { kind: NodeKind::Type, ..Default::default() });
        let pattern = Pattern::compile("typeof($T$)", &ty).unwrap();
        assert_eq!(pattern.find("class C { void M() { var a = typeof(List<int>); var b = typeof(int); } }", 10).len(), 2);
        let ident = with("x", Constraint { kind: NodeKind::Identifier, ..Default::default() });
        let pattern = Pattern::compile("Use($x$)", &ident).unwrap();
        assert_eq!(texts(&pattern, "class C { void M() { Use(a); Use(a.b); Use(1); } }"), ["Use(a)"]);
        let expression = with("x", Constraint { kind: NodeKind::Expression, ..Default::default() });
        let pattern = Pattern::compile("Use($x$)", &expression).unwrap();
        assert_eq!(pattern.find("class C { void M() { Use(a); Use(a.b); Use(1); Use(() => 1); } }", 10).len(), 4);
    }

    #[test]
    fn rejects_patterns_that_cannot_match_usefully() {
        assert!(Pattern::compile("", &BTreeMap::new()).is_err());
        assert!(Pattern::compile("$x$", &BTreeMap::new()).is_err());
        assert!(Pattern::compile("Foo($x)", &BTreeMap::new()).is_err());
        assert!(Pattern::compile("Foo(", &BTreeMap::new()).is_err());
        // Interpolated strings keep their `$`.
        assert!(Pattern::compile("Log($\"{$x$}\")", &BTreeMap::new()).is_ok());
    }

    #[test]
    fn matches_are_outermost_and_in_text_order() {
        let pattern = compile("Wrap($x$)");
        let source = "class C { void M() { Wrap(Wrap(1)); Wrap(2); } }";
        assert_eq!(texts(&pattern, source), ["Wrap(Wrap(1))", "Wrap(2)"]);
        assert_eq!(pattern.find(source, 1).len(), 1);
    }

    #[test]
    fn statement_position_is_told_apart_from_expressions() {
        let any = with("body", Constraint { count: Count::Any, ..Default::default() });
        // Without the `;` inside braces.
        let pattern = Pattern::compile("lock ($o$) { $body$ }", &any).unwrap();
        let hits = pattern.find("class C { void M() { lock (gate) { a(); b(); } } }", 10);
        assert_eq!(hits[0].captures["body"], "a(); b();");
        // After `:` but inside an expression.
        let ternary = compile("x = $a$ ? $b$ : $c$;");
        assert_eq!(ternary.find("class C { void M() { x = ok ? 1 : Fallback(); } }", 10)[0].captures["c"], "Fallback()");
        // Braces of an initializer, not a block.
        let array = compile("new[] { $x$ }");
        assert_eq!(array.find("class C { void M() { var a = new[] { 1 }; } }", 10)[0].captures["x"], "1");
        // Expressions C# does not allow as statements.
        assert_eq!(texts(&compile("$a$ + 1"), "class C { int M() { return b * 2 + 1; } }"), ["b * 2 + 1"]);
    }

    #[test]
    fn top_level_declarations_match() {
        let pattern = compile("using $ns$;");
        let source = "using System;\nusing System.Linq;\nnamespace N { class C { } }";
        let names: Vec<String> = pattern.find(source, 10).into_iter().map(|hit| hit.captures["ns"].clone()).collect();
        assert_eq!(names, ["System", "System.Linq"]);
    }

    #[test]
    fn unknown_replacement_placeholders_are_errors() {
        let pattern = compile("Foo($a$)");
        let hit = &pattern.find("class C { void M() { Foo(1); } }", 1)[0];
        assert!(pattern.replace("Bar($b$)", &hit.captures).is_err());
    }
}
