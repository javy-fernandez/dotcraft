//! A tolerant JSON parser for configuration files: comments, trailing commas and text being
//! typed (unclosed strings, objects and arrays, missing values) still give a tree, with the byte
//! range of every key and value. [`spot_at`] says what the caret is on: a key or a value, and
//! the configuration path that leads there.

use std::ops::Range;

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Object(Vec<Member>),
    Array(Vec<Node>),
    String(String),
    Number(String),
    Bool(bool),
    Null,
    /// A bare word that is not a JSON literal.
    Invalid(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub range: Range<usize>,
    pub kind: Kind,
    /// The closing quote, brace or bracket is there.
    pub closed: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Member {
    pub key: String,
    /// The key with its quotes (or the bare word typed in its place).
    pub key_range: Range<usize>,
    pub quoted: bool,
    pub key_closed: bool,
    /// Byte offset of the `:`.
    pub colon: Option<usize>,
    pub value: Option<Node>,
}

/// A step of a configuration path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Seg {
    Key(String),
    Index(usize),
}

/// The path as configuration writes it (`Logging:LogLevel:Default`).
pub fn path_string(path: &[Seg]) -> String {
    path.iter()
        .map(|s| match s {
            Seg::Key(k) => k.clone(),
            Seg::Index(i) => i.to_string(),
        })
        .collect::<Vec<_>>()
        .join(":")
}

/// What is at a byte offset.
#[derive(Clone, Debug, PartialEq)]
pub enum Spot {
    /// A property name (being typed, or an empty place where one can go).
    Key {
        /// Path of the object that holds it.
        path: Vec<Seg>,
        /// Bytes an accepted name replaces: the quoted key, the bare word, or the caret.
        range: Range<usize>,
        /// The name before the caret.
        typed: String,
        /// The key has a `:` already (only the name is replaced).
        has_colon: bool,
        /// Names of the object's other members.
        siblings: Vec<String>,
    },
    /// A value (or the empty place after a `:` or in an array).
    Value {
        /// Path of the value itself.
        path: Vec<Seg>,
        /// Bytes an accepted value replaces.
        range: Range<usize>,
        typed: String,
        /// Inside a string's quotes.
        quoted: bool,
    },
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Colon,
    Comma,
    Str(String, bool),
    Word(String),
}

#[derive(Clone, Debug)]
struct Token {
    tok: Tok,
    range: Range<usize>,
}

fn tokenize(text: &str) -> Vec<Token> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        match c {
            b' ' | b'\t' | b'\r' | b'\n' => i += 1,
            b'/' if b.get(i + 1) == Some(&b'/') => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                i = text[i + 2..].find("*/").map_or(b.len(), |e| i + 2 + e + 2);
            }
            b'{' | b'}' | b'[' | b']' | b':' | b',' => {
                let tok = match c {
                    b'{' => Tok::LBrace,
                    b'}' => Tok::RBrace,
                    b'[' => Tok::LBracket,
                    b']' => Tok::RBracket,
                    b':' => Tok::Colon,
                    _ => Tok::Comma,
                };
                out.push(Token { tok, range: i..i + 1 });
                i += 1;
            }
            b'"' => {
                let start = i;
                i += 1;
                let mut s = String::new();
                let mut closed = false;
                while i < b.len() {
                    match b[i] {
                        b'"' => {
                            closed = true;
                            i += 1;
                            break;
                        }
                        // Unterminated: the string ends with its line.
                        b'\n' => break,
                        b'\\' if i + 1 < b.len() => {
                            let e = b[i + 1];
                            match e {
                                b'n' => s.push('\n'),
                                b't' => s.push('\t'),
                                b'r' => s.push('\r'),
                                b'b' => s.push('\u{8}'),
                                b'f' => s.push('\u{c}'),
                                b'u' if i + 6 <= b.len() => {
                                    if let Some(ch) = u32::from_str_radix(&text[i + 2..i + 6], 16).ok().and_then(char::from_u32) {
                                        s.push(ch);
                                    }
                                    i += 4;
                                }
                                _ => s.push(e as char),
                            }
                            i += 2;
                        }
                        _ => {
                            let ch = text[i..].chars().next().unwrap_or(' ');
                            s.push(ch);
                            i += ch.len_utf8();
                        }
                    }
                }
                let end = i.min(b.len());
                // Keep the range on a char boundary even for a cut `\u` escape.
                let end = (end..=b.len()).find(|&e| text.is_char_boundary(e)).unwrap_or(b.len());
                out.push(Token { tok: Tok::Str(s, closed), range: start..end });
                i = end;
            }
            _ => {
                let start = i;
                while i < b.len() && !matches!(b[i], b' ' | b'\t' | b'\r' | b'\n' | b'{' | b'}' | b'[' | b']' | b':' | b',' | b'"') && !(b[i] == b'/' && matches!(b.get(i + 1), Some(b'/' | b'*'))) {
                    i += 1;
                }
                if i == start {
                    i += 1;
                    continue;
                }
                let end = (i..=b.len()).find(|&e| text.is_char_boundary(e)).unwrap_or(b.len());
                out.push(Token { tok: Tok::Word(text[start..end].to_string()), range: start..end });
                i = end;
            }
        }
    }
    out
}

struct Parser {
    toks: Vec<Token>,
    at: usize,
    len: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.toks.get(self.at)
    }

    fn peek_at(&self, n: usize) -> Option<&Token> {
        self.toks.get(self.at + n)
    }

    fn starts_value(t: &Tok) -> bool {
        matches!(t, Tok::LBrace | Tok::LBracket | Tok::Str(..) | Tok::Word(_))
    }

    fn value(&mut self, depth: usize) -> Option<Node> {
        let t = self.peek()?.clone();
        self.at += 1;
        let node = match t.tok {
            Tok::LBrace if depth < 128 => self.object(t.range.start, depth),
            Tok::LBracket if depth < 128 => self.array(t.range.start, depth),
            Tok::Str(s, closed) => Node { range: t.range, kind: Kind::String(s), closed },
            Tok::Word(w) => {
                let kind = match w.as_str() {
                    "true" => Kind::Bool(true),
                    "false" => Kind::Bool(false),
                    "null" => Kind::Null,
                    _ if w.parse::<f64>().is_ok() => Kind::Number(w),
                    _ => Kind::Invalid(w),
                };
                Node { range: t.range, kind, closed: true }
            }
            _ => return None,
        };
        Some(node)
    }

    fn object(&mut self, start: usize, depth: usize) -> Node {
        let mut members = Vec::new();
        loop {
            let Some(t) = self.peek().cloned() else {
                return Node { range: start..self.len, kind: Kind::Object(members), closed: false };
            };
            match t.tok {
                Tok::RBrace => {
                    self.at += 1;
                    return Node { range: start..t.range.end, kind: Kind::Object(members), closed: true };
                }
                Tok::Str(..) | Tok::Word(_) => {
                    self.at += 1;
                    let (key, quoted, key_closed) = match t.tok {
                        Tok::Str(s, closed) => (s, true, closed),
                        Tok::Word(w) => (w, false, true),
                        _ => unreachable!(),
                    };
                    let mut m = Member { key, key_range: t.range, quoted, key_closed, colon: None, value: None };
                    if let Some(Token { tok: Tok::Colon, range }) = self.peek() {
                        m.colon = Some(range.start);
                        self.at += 1;
                        // `"a": "b": 1`: "b" starts the next member (a's value is still missing).
                        let next_is_key = matches!(self.peek().map(|t| &t.tok), Some(Tok::Str(..) | Tok::Word(_))) && matches!(self.peek_at(1).map(|t| &t.tok), Some(Tok::Colon));
                        if self.peek().is_some_and(|t| Self::starts_value(&t.tok)) && !next_is_key {
                            m.value = self.value(depth + 1);
                        }
                    }
                    members.push(m);
                }
                Tok::LBrace | Tok::LBracket => {
                    // A value without a key: parse it to stay in step.
                    let _ = self.value(depth + 1);
                }
                Tok::RBracket if depth > 0 => {
                    // A `]` closing an outer array: this object ends unclosed.
                    return Node { range: start..t.range.start, kind: Kind::Object(members), closed: false };
                }
                _ => self.at += 1,
            }
        }
    }

    fn array(&mut self, start: usize, depth: usize) -> Node {
        let mut items = Vec::new();
        loop {
            let Some(t) = self.peek().cloned() else {
                return Node { range: start..self.len, kind: Kind::Array(items), closed: false };
            };
            match t.tok {
                Tok::RBracket => {
                    self.at += 1;
                    return Node { range: start..t.range.end, kind: Kind::Array(items), closed: true };
                }
                Tok::RBrace => return Node { range: start..t.range.start, kind: Kind::Array(items), closed: false },
                ref tok if Self::starts_value(tok) => {
                    if let Some(v) = self.value(depth + 1) {
                        items.push(v);
                    }
                }
                _ => self.at += 1,
            }
        }
    }
}

/// The document's top value (`None` when it holds no value at all).
pub fn parse(text: &str) -> Option<Node> {
    let mut p = Parser { toks: tokenize(text), at: 0, len: text.len() };
    p.value(0)
}

/// The text of `range` before `b` (the typed part).
fn before(text: &str, range: &Range<usize>, b: usize, skip: usize) -> String {
    let s = (range.start + skip).min(b);
    text.get(s..b).unwrap_or("").to_string()
}

/// `b` is between a string's quotes.
fn in_string(range: &Range<usize>, closed: bool, b: usize) -> bool {
    range.start < b && (b < range.end || !closed && b <= range.end)
}

/// What is at byte `b` of `text` (parsed as `root`).
pub fn spot_at(root: &Node, text: &str, b: usize) -> Option<Spot> {
    let mut path = Vec::new();
    spot_in(root, text, b, &mut path)
}

fn spot_in(node: &Node, text: &str, b: usize, path: &mut Vec<Seg>) -> Option<Spot> {
    let inside = node.range.start < b && (b < node.range.end || !node.closed && b <= node.range.end);
    if !inside {
        return None;
    }
    match &node.kind {
        Kind::Object(members) => {
            for (i, m) in members.iter().enumerate() {
                let on_key = if m.quoted { in_string(&m.key_range, m.key_closed, b) } else { m.key_range.start <= b && b <= m.key_range.end };
                if on_key {
                    let siblings = members.iter().enumerate().filter(|(j, _)| *j != i).map(|(_, o)| o.key.clone()).collect();
                    return Some(Spot::Key { path: path.clone(), range: m.key_range.clone(), typed: before(text, &m.key_range, b, usize::from(m.quoted)), has_colon: m.colon.is_some(), siblings });
                }
                if let Some(v) = &m.value {
                    let on_value = match &v.kind {
                        Kind::Object(_) | Kind::Array(_) => v.range.start < b && (b < v.range.end || !v.closed && b <= v.range.end),
                        Kind::String(_) => in_string(&v.range, v.closed, b),
                        _ => v.range.start <= b && b <= v.range.end,
                    };
                    if on_value {
                        path.push(Seg::Key(m.key.clone()));
                        let out = match &v.kind {
                            Kind::Object(_) | Kind::Array(_) => spot_in(v, text, b, path),
                            Kind::String(_) => Some(Spot::Value { path: path.clone(), range: v.range.clone(), typed: before(text, &v.range, b, 1), quoted: true }),
                            _ => Some(Spot::Value { path: path.clone(), range: v.range.clone(), typed: before(text, &v.range, b, 0), quoted: false }),
                        };
                        path.pop();
                        return out;
                    }
                }
                // Right after `"key":` with nothing typed yet.
                if let Some(c) = m.colon
                    && b > c
                    && m.value.as_ref().is_none_or(|v| b <= v.range.start)
                    && text.get(c + 1..b).is_some_and(|s| s.trim().is_empty())
                {
                    let mut p = path.clone();
                    p.push(Seg::Key(m.key.clone()));
                    return Some(Spot::Value { path: p, range: b..b, typed: String::new(), quoted: false });
                }
            }
            // Between members: a new key.
            Some(Spot::Key { path: path.clone(), range: b..b, typed: String::new(), has_colon: false, siblings: members.iter().map(|m| m.key.clone()).collect() })
        }
        Kind::Array(items) => {
            for (i, v) in items.iter().enumerate() {
                let on = match &v.kind {
                    Kind::Object(_) | Kind::Array(_) => v.range.start < b && (b < v.range.end || !v.closed && b <= v.range.end),
                    Kind::String(_) => in_string(&v.range, v.closed, b),
                    _ => v.range.start <= b && b <= v.range.end,
                };
                if on {
                    path.push(Seg::Index(i));
                    let out = match &v.kind {
                        Kind::Object(_) | Kind::Array(_) => spot_in(v, text, b, path),
                        Kind::String(_) => Some(Spot::Value { path: path.clone(), range: v.range.clone(), typed: before(text, &v.range, b, 1), quoted: true }),
                        _ => Some(Spot::Value { path: path.clone(), range: v.range.clone(), typed: before(text, &v.range, b, 0), quoted: false }),
                    };
                    path.pop();
                    return out;
                }
            }
            let index = items.iter().filter(|v| v.range.end <= b).count();
            let mut p = path.clone();
            p.push(Seg::Index(index));
            Some(Spot::Value { path: p, range: b..b, typed: String::new(), quoted: false })
        }
        _ => None,
    }
}

/// The member whose key or value covers byte `b`, with its path (hover, navigation).
pub fn member_at(root: &Node, b: usize) -> Option<(Vec<Seg>, &Member)> {
    let mut path = Vec::new();
    let mut node = root;
    loop {
        match &node.kind {
            Kind::Object(members) => {
                let m = members.iter().find(|m| m.key_range.start <= b && b <= m.value.as_ref().map_or(m.key_range.end, |v| v.range.end))?;
                path.push(Seg::Key(m.key.clone()));
                match &m.value {
                    Some(v) if matches!(v.kind, Kind::Object(_) | Kind::Array(_)) && b > v.range.start && b > m.key_range.end => node = v,
                    _ => return Some((path, m)),
                }
            }
            Kind::Array(items) => {
                let (i, v) = items.iter().enumerate().find(|(_, v)| v.range.start <= b && b <= v.range.end)?;
                path.push(Seg::Index(i));
                node = v;
            }
            _ => return None,
        }
    }
}

/// Every member of the tree with its path (the key last).
pub fn walk<'a>(root: &'a Node, f: &mut dyn FnMut(&[Seg], &'a Member)) {
    fn go<'a>(node: &'a Node, path: &mut Vec<Seg>, f: &mut dyn FnMut(&[Seg], &'a Member)) {
        match &node.kind {
            Kind::Object(members) => {
                for m in members {
                    path.push(Seg::Key(m.key.clone()));
                    f(path, m);
                    if let Some(v) = &m.value {
                        go(v, path, f);
                    }
                    path.pop();
                }
            }
            Kind::Array(items) => {
                for (i, v) in items.iter().enumerate() {
                    path.push(Seg::Index(i));
                    go(v, path, f);
                    path.pop();
                }
            }
            _ => {}
        }
    }
    go(root, &mut Vec::new(), f);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(s: &str) -> Seg {
        Seg::Key(s.into())
    }

    /// `|` marks the caret.
    fn spot(src: &str) -> Spot {
        let b = src.find('|').unwrap();
        let text = src.replacen('|', "", 1);
        spot_at(&parse(&text).unwrap(), &text, b).unwrap()
    }

    #[test]
    fn parses_comments_trailing_commas_and_ranges() {
        let text = "{\n  // comment\n  \"Logging\": { \"LogLevel\": { \"Default\": \"Warning\", } },\n  /* x */ \"Port\": 5000,\n}";
        let root = parse(text).unwrap();
        let Kind::Object(m) = &root.kind else { panic!() };
        assert_eq!(m.iter().map(|m| m.key.as_str()).collect::<Vec<_>>(), ["Logging", "Port"]);
        assert_eq!(&text[m[1].key_range.clone()], "\"Port\"");
        assert!(matches!(&m[1].value.as_ref().unwrap().kind, Kind::Number(n) if n == "5000"));
        let mut paths = Vec::new();
        walk(&root, &mut |p, _| paths.push(path_string(p)));
        assert_eq!(paths, ["Logging", "Logging:LogLevel", "Logging:LogLevel:Default", "Port"]);
    }

    #[test]
    fn keys_being_typed() {
        assert_eq!(spot("{ \"Logging\": { \"Lo|\" } }"), Spot::Key { path: vec![key("Logging")], range: 15..19, typed: "Lo".into(), has_colon: false, siblings: vec![] });
        // Unclosed quote at the end of the line, inside an unclosed object.
        let s = spot("{\n  \"Kestrel\": {\n    \"End|\n");
        assert!(matches!(&s, Spot::Key { path, typed, .. } if *path == vec![key("Kestrel")] && typed == "End"));
        // An empty place between members.
        let s = spot("{ \"A\": 1,\n  |\n  \"B\": 2 }");
        assert!(matches!(&s, Spot::Key { path, range, siblings, .. } if path.is_empty() && range.is_empty() && *siblings == ["A", "B"]));
        // A key that has its colon already.
        assert!(matches!(spot("{ \"Allo|\": \"*\" }"), Spot::Key { has_colon: true, .. }));
        // A bare word in a key's place.
        assert!(matches!(spot("{ Kes| }"), Spot::Key { typed, .. } if typed == "Kes"));
    }

    #[test]
    fn values_and_array_items() {
        let s = spot("{ \"Logging\": { \"LogLevel\": { \"Default\": \"Inf|\" } } }");
        assert_eq!(s, Spot::Value { path: vec![key("Logging"), key("LogLevel"), key("Default")], range: 40..45, typed: "Inf".into(), quoted: true });
        // After the colon, nothing typed: the next member is not taken as the value.
        let s = spot("{ \"AllowedHosts\": |\n  \"Logging\": {} }");
        assert!(matches!(&s, Spot::Value { path, range, .. } if *path == vec![key("AllowedHosts")] && range.is_empty()));
        let s = spot("{ \"Flag\": tr| }");
        assert!(matches!(&s, Spot::Value { typed, quoted: false, .. } if typed == "tr"));
        let s = spot("{ \"Hosts\": [ \"a\", \"|\" ] }");
        assert!(matches!(&s, Spot::Value { path, .. } if *path == vec![key("Hosts"), Seg::Index(1)]));
        let s = spot("{ \"Items\": [ { \"Na|\" } ] }");
        assert!(matches!(&s, Spot::Key { path, .. } if *path == vec![key("Items"), Seg::Index(0)]));
    }

    #[test]
    fn member_under_a_byte() {
        let text = "{ \"Smtp\": { \"Host\": \"x\", \"Port\": 25 } }";
        let root = parse(text).unwrap();
        let (p, m) = member_at(&root, text.find("Port").unwrap() + 1).unwrap();
        assert_eq!((path_string(&p).as_str(), m.key.as_str()), ("Smtp:Port", "Port"));
        let (p, _) = member_at(&root, text.find("Smtp").unwrap()).unwrap();
        assert_eq!(path_string(&p), "Smtp");
        assert!(member_at(&root, text.find("25").unwrap()).is_some_and(|(p, _)| path_string(&p) == "Smtp:Port"));
    }
}
