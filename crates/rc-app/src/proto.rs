//! Protocol Buffers (`.proto`) declarations and type references, read by a small
//! statement-aware parser: the Structure window, Go to Symbol, Go to Declaration and Find
//! Usages across the workspace's `.proto` files, and the gRPC Client's method list.
//!
//! proto2, proto3 and editions share the grammar used here: `message`, `enum`, `service`,
//! `rpc`, `oneof`, `extend`, `group`, `map<,>` fields, options and reserved ranges. Broken
//! text is fine: every step consumes at least one token, so the parser always ends.

use rc_lsp::lsp_types::SymbolKind;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// A declaration: package, message, enum, service, rpc, oneof, field or enum value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Symbol {
    pub name: String,
    /// Fully qualified name (`shop.v1.Order.Line`).
    pub full: String,
    pub kind: SymbolKind,
    pub line: usize,
    pub col: usize,
    /// Nesting level, for the Structure tree.
    pub depth: usize,
    /// For an rpc, the gRPC method as `package.Service/Method`.
    pub method: Option<String>,
}

impl Symbol {
    /// Messages, groups and enums: what a field or an rpc can refer to.
    pub fn is_type(&self) -> bool {
        matches!(self.kind, SymbolKind::STRUCT | SymbolKind::ENUM)
    }
}

/// A type name used by a field, an rpc or an `extend`, as written (`Line`, `common.Money`,
/// `.shop.v1.Order`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reference {
    pub name: String,
    /// Full name of the scope it is written in (package or message), for resolution.
    pub scope: String,
    pub line: usize,
    pub col: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProtoFile {
    pub package: String,
    pub imports: Vec<String>,
    pub symbols: Vec<Symbol>,
    pub references: Vec<Reference>,
}

impl ProtoFile {
    /// Fully qualified gRPC methods (`package.Service/Method`).
    pub fn methods(&self) -> Vec<String> {
        self.symbols.iter().filter_map(|s| s.method.clone()).collect()
    }

    /// The reference or declaration name under (`line`, `col`).
    pub fn at(&self, line: usize, col: usize) -> Option<Hit<'_>> {
        let within = |l: usize, c: usize, len: usize| l == line && c <= col && col <= c + len;
        if let Some(r) = self.references.iter().find(|r| within(r.line, r.col, r.name.chars().count())) {
            return Some(Hit::Reference(r));
        }
        self.symbols.iter().find(|s| s.kind != SymbolKind::PACKAGE && within(s.line, s.col, s.name.chars().count())).map(Hit::Declaration)
    }
}

pub enum Hit<'a> {
    Reference(&'a Reference),
    Declaration(&'a Symbol),
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    /// An identifier, possibly dotted (`google.protobuf.Timestamp`, `.shop.Order`).
    Ident(String),
    Str(String),
    Num,
    Punct(char),
}

#[derive(Clone, Debug)]
struct Token {
    tok: Tok,
    line: usize,
    col: usize,
}

fn tokenize(source: &str) -> Vec<Token> {
    let chars: Vec<char> = source.chars().collect();
    let mut out = Vec::new();
    let (mut i, mut line, mut col) = (0, 0, 0);
    let ident_start = |c: char| c.is_ascii_alphabetic() || c == '_';
    let ident_char = |c: char| c.is_ascii_alphanumeric() || c == '_';
    // Advances over `n` chars, keeping the position.
    let step = |i: &mut usize, line: &mut usize, col: &mut usize, n: usize| {
        for _ in 0..n {
            if chars.get(*i) == Some(&'\n') {
                *line += 1;
                *col = 0;
            } else {
                *col += 1;
            }
            *i += 1;
        }
    };
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        let (l0, c0) = (line, col);
        if c.is_whitespace() {
            step(&mut i, &mut line, &mut col, 1);
        } else if c == '/' && next == Some('/') {
            while i < chars.len() && chars[i] != '\n' {
                step(&mut i, &mut line, &mut col, 1);
            }
        } else if c == '/' && next == Some('*') {
            step(&mut i, &mut line, &mut col, 2);
            while i < chars.len() && !(chars[i] == '*' && chars.get(i + 1) == Some(&'/')) {
                step(&mut i, &mut line, &mut col, 1);
            }
            let close = 2.min(chars.len() - i);
            step(&mut i, &mut line, &mut col, close);
        } else if c == '"' || c == '\'' {
            // Strings end at their quote or, unterminated, at the end of the line.
            step(&mut i, &mut line, &mut col, 1);
            let mut text = String::new();
            while i < chars.len() && chars[i] != c && chars[i] != '\n' {
                if chars[i] == '\\' && i + 1 < chars.len() && chars[i + 1] != '\n' {
                    text.push(chars[i + 1]);
                    step(&mut i, &mut line, &mut col, 2);
                } else {
                    text.push(chars[i]);
                    step(&mut i, &mut line, &mut col, 1);
                }
            }
            if chars.get(i) == Some(&c) {
                step(&mut i, &mut line, &mut col, 1);
            }
            out.push(Token { tok: Tok::Str(text), line: l0, col: c0 });
        } else if ident_start(c) || (c == '.' && next.is_some_and(ident_start)) {
            let mut text = String::new();
            loop {
                if chars.get(i) == Some(&'.') && chars.get(i + 1).copied().is_some_and(ident_start) {
                    text.push('.');
                    step(&mut i, &mut line, &mut col, 1);
                }
                while i < chars.len() && ident_char(chars[i]) {
                    text.push(chars[i]);
                    step(&mut i, &mut line, &mut col, 1);
                }
                if !(chars.get(i) == Some(&'.') && chars.get(i + 1).copied().is_some_and(ident_start)) {
                    break;
                }
            }
            out.push(Token { tok: Tok::Ident(text), line: l0, col: c0 });
        } else if c.is_ascii_digit() {
            while i < chars.len() && (ident_char(chars[i]) || chars[i] == '.') {
                step(&mut i, &mut line, &mut col, 1);
            }
            out.push(Token { tok: Tok::Num, line: l0, col: c0 });
        } else {
            step(&mut i, &mut line, &mut col, 1);
            out.push(Token { tok: Tok::Punct(c), line: l0, col: c0 });
        }
    }
    out
}

const SCALARS: &[&str] = &[
    "double", "float", "int32", "int64", "uint32", "uint64", "sint32", "sint64", "fixed32", "fixed64", "sfixed32", "sfixed64", "bool",
    "string", "bytes",
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ScopeKind {
    Message,
    Enum,
    Service,
    Oneof,
    Extend,
    /// An rpc's option block, or a block the parser does not know.
    Other,
}

struct Scope {
    kind: ScopeKind,
    /// Prefix of the declarations inside (`pkg.Message`).
    full: String,
    /// Where type names written inside are looked up from (a oneof's is its message's).
    lookup: String,
}

fn qualify(scope: &str, name: &str) -> String {
    if scope.is_empty() { name.to_string() } else { format!("{scope}.{name}") }
}

struct Parser {
    toks: Vec<Token>,
}

impl Parser {
    fn ident(&self, i: usize) -> Option<&str> {
        match self.toks.get(i).map(|t| &t.tok) {
            Some(Tok::Ident(s)) => Some(s),
            _ => None,
        }
    }

    fn punct(&self, i: usize, c: char) -> bool {
        matches!(self.toks.get(i).map(|t| &t.tok), Some(Tok::Punct(p)) if *p == c)
    }

    /// Index after the statement starting at `i`: after its `;` or its balanced `{…}` block,
    /// or at an unbalanced `}` (which closes the enclosing scope). Always past `i`.
    fn skip_statement(&self, i: usize) -> usize {
        let mut depth = 0usize;
        let mut j = i;
        while j < self.toks.len() {
            match self.toks[j].tok {
                Tok::Punct(';') if depth == 0 => return j + 1,
                Tok::Punct('(' | '[' | '{' | '<') => depth += 1,
                Tok::Punct(c @ (')' | ']' | '}' | '>')) => {
                    if depth == 0 {
                        return j.max(i + 1);
                    }
                    depth -= 1;
                    if depth == 0 && c == '}' {
                        // A block ends the statement; a trailing `;` goes with it.
                        return if self.punct(j + 1, ';') { j + 2 } else { j + 1 };
                    }
                }
                _ => {}
            }
            j += 1;
        }
        j.max(i + 1)
    }

    /// The `{` of a declaration header starting at `i`, if it comes before any `;` or `}`.
    fn block_open(&self, i: usize) -> Option<usize> {
        let mut depth = 0usize;
        for j in i..self.toks.len() {
            match self.toks[j].tok {
                Tok::Punct('{') if depth == 0 => return Some(j),
                Tok::Punct('(' | '[') => depth += 1,
                Tok::Punct(')' | ']') => depth = depth.saturating_sub(1),
                Tok::Punct(';' | '}') if depth == 0 => return None,
                _ => {}
            }
        }
        None
    }
}

pub fn parse(source: &str) -> ProtoFile {
    let p = Parser { toks: tokenize(source) };
    let mut out = ProtoFile::default();
    let mut stack: Vec<Scope> = Vec::new();
    let mut i = 0;
    while i < p.toks.len() {
        let depth = stack.len();
        let (full, lookup, kind) = match stack.last() {
            Some(s) => (s.full.clone(), s.lookup.clone(), Some(s.kind)),
            None => (out.package.clone(), out.package.clone(), None),
        };
        let tok = &p.toks[i];
        let symbol = |name: &str, full: String, kind: SymbolKind, t: &Token| Symbol {
            name: name.to_string(),
            full,
            kind,
            line: t.line,
            col: t.col,
            depth,
            method: None,
        };
        match &tok.tok {
            Tok::Punct('}') => {
                stack.pop();
                i += 1;
                continue;
            }
            Tok::Punct(';') => {
                i += 1;
                continue;
            }
            Tok::Punct('{') => {
                stack.push(Scope { kind: ScopeKind::Other, full, lookup });
                i += 1;
                continue;
            }
            Tok::Ident(_) => {}
            // Stray strings, numbers or punctuation: dropped one at a time.
            _ => {
                i += 1;
                continue;
            }
        }
        let word = p.ident(i).unwrap_or_default().to_string();
        // `message X {`, `enum X {`, `service X {`, `oneof x {`, `extend X {`.
        let declares = |k: &str| word == k && p.ident(i + 1).is_some() && p.punct(i + 2, '{');
        if declares("message") || declares("enum") || declares("service") || (declares("oneof") && kind != Some(ScopeKind::Enum)) {
            let name = p.ident(i + 1).unwrap_or_default().to_string();
            let qualified = qualify(&full, &name);
            let (sk, scope) = match word.as_str() {
                "message" => (SymbolKind::STRUCT, ScopeKind::Message),
                "enum" => (SymbolKind::ENUM, ScopeKind::Enum),
                "service" => (SymbolKind::INTERFACE, ScopeKind::Service),
                _ => (SymbolKind::FIELD, ScopeKind::Oneof),
            };
            out.symbols.push(symbol(&name, qualified.clone(), sk, &p.toks[i + 1]));
            // A oneof's fields belong to the message; nested types to the type.
            let (inner, lookup) = if scope == ScopeKind::Oneof { (full.clone(), lookup) } else { (qualified.clone(), qualified) };
            stack.push(Scope { kind: scope, full: inner, lookup });
            i += 3;
            continue;
        }
        if declares("extend") {
            let t = &p.toks[i + 1];
            out.references.push(Reference { name: p.ident(i + 1).unwrap_or_default().to_string(), scope: lookup.clone(), line: t.line, col: t.col });
            stack.push(Scope { kind: ScopeKind::Extend, full: full.clone(), lookup });
            i += 3;
            continue;
        }
        match word.as_str() {
            "package" if depth == 0 => {
                if let Some(name) = p.ident(i + 1) {
                    out.package = name.trim_start_matches('.').to_string();
                    out.symbols.push(symbol(&out.package, out.package.clone(), SymbolKind::PACKAGE, &p.toks[i + 1]));
                }
                i = p.skip_statement(i);
                continue;
            }
            "import" if depth == 0 => {
                let mut j = i + 1;
                while p.ident(j).is_some_and(|w| w == "public" || w == "weak") {
                    j += 1;
                }
                if let Some(Tok::Str(path)) = p.toks.get(j).map(|t| &t.tok) {
                    out.imports.push(path.clone());
                }
                i = p.skip_statement(i);
                continue;
            }
            "syntax" | "edition" | "option" | "reserved" | "extensions" => {
                i = p.skip_statement(i);
                continue;
            }
            _ => {}
        }
        match kind {
            Some(ScopeKind::Service) if word == "rpc" && p.ident(i + 1).is_some() => {
                let name = p.ident(i + 1).unwrap_or_default().to_string();
                let mut s = symbol(&name, qualify(&full, &name), SymbolKind::METHOD, &p.toks[i + 1]);
                let service = full.rsplit('.').next().unwrap_or_default();
                let package = full.rsplit_once('.').map_or("", |(pkg, _)| pkg);
                s.method = Some(format!("{}/{name}", qualify(package, service)));
                out.symbols.push(s);
                // `(stream Request) returns (stream Reply)`: the two types are references.
                let mut j = i + 2;
                while j < p.toks.len() && !p.punct(j, ';') && !p.punct(j, '{') && !p.punct(j, '}') {
                    if p.punct(j, '(') {
                        let mut k = j + 1;
                        if p.ident(k) == Some("stream") && p.ident(k + 1).is_some() {
                            k += 1;
                        }
                        if let Some(ty) = p.ident(k) {
                            let t = &p.toks[k];
                            out.references.push(Reference { name: ty.to_string(), scope: lookup.clone(), line: t.line, col: t.col });
                        }
                    }
                    j += 1;
                }
                if p.punct(j, '{') {
                    stack.push(Scope { kind: ScopeKind::Other, full: full.clone(), lookup: lookup.clone() });
                    i = j + 1;
                } else {
                    i = if p.punct(j, ';') { j + 1 } else { j.max(i + 1) };
                }
            }
            Some(ScopeKind::Enum) => {
                // `NAME = 1 [deprecated = true];`
                if p.punct(i + 1, '=') {
                    out.symbols.push(symbol(&word, qualify(&lookup, &word), SymbolKind::ENUM_MEMBER, tok));
                }
                i = p.skip_statement(i);
            }
            Some(ScopeKind::Message | ScopeKind::Oneof | ScopeKind::Extend) => {
                let mut j = i;
                if matches!(word.as_str(), "repeated" | "optional" | "required") {
                    j += 1;
                }
                if p.ident(j) == Some("group") && p.ident(j + 1).is_some() && let Some(open) = p.block_open(j) {
                    // proto2 `optional group Result = 1 { … }`: a nested message and its field.
                    let name = p.ident(j + 1).unwrap_or_default().to_string();
                    let qualified = qualify(&full, &name);
                    out.symbols.push(symbol(&name, qualified.clone(), SymbolKind::STRUCT, &p.toks[j + 1]));
                    stack.push(Scope { kind: ScopeKind::Message, full: qualified.clone(), lookup: qualified });
                    i = open + 1;
                    continue;
                }
                // `map<string, Price> prices = 3;`
                let (ty, name) = if p.ident(j) == Some("map") && p.punct(j + 1, '<') {
                    let close = (j + 2..p.toks.len().min(j + 12)).find(|&k| p.punct(k, '>'));
                    match close {
                        Some(k) if p.punct(k - 2, ',') => (Some(k - 1), k + 1),
                        _ => (None, j + 1),
                    }
                } else {
                    (Some(j), j + 1)
                };
                if let Some(field) = p.ident(name)
                    && p.punct(name + 1, '=')
                {
                    if let Some(ty) = ty
                        && let Some(type_name) = p.ident(ty)
                        && !SCALARS.contains(&type_name)
                    {
                        let t = &p.toks[ty];
                        out.references.push(Reference { name: type_name.to_string(), scope: lookup.clone(), line: t.line, col: t.col });
                    }
                    out.symbols.push(symbol(field, qualify(&lookup, field), SymbolKind::FIELD, &p.toks[name]));
                }
                i = p.skip_statement(i);
            }
            _ => i = p.skip_statement(i),
        }
    }
    out
}

/// The type a reference written in `scope` names, protobuf style: from the innermost scope
/// outwards (`Line` in `shop.Order` tries `shop.Order.Line`, `shop.Line`, `Line`).
pub fn resolve(name: &str, scope: &str, exists: impl Fn(&str) -> bool) -> Option<String> {
    if let Some(absolute) = name.strip_prefix('.') {
        return exists(absolute).then(|| absolute.to_string());
    }
    let mut scope = scope;
    loop {
        let candidate = qualify(scope, name);
        if exists(&candidate) {
            return Some(candidate);
        }
        if scope.is_empty() {
            return None;
        }
        scope = scope.rsplit_once('.').map_or("", |(outer, _)| outer);
    }
}

/// A place in a `.proto` file (0-based line and column).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Location {
    pub path: PathBuf,
    pub line: usize,
    pub col: usize,
}

/// The parsed `.proto` files of the workspace, for navigation between them.
#[derive(Default)]
pub struct Index {
    pub files: Vec<(PathBuf, ProtoFile)>,
    types: HashMap<String, Location>,
}

impl Index {
    pub fn new(files: Vec<(PathBuf, ProtoFile)>) -> Index {
        let mut types = HashMap::new();
        for (path, file) in &files {
            for s in file.symbols.iter().filter(|s| s.is_type()) {
                types.entry(s.full.clone()).or_insert_with(|| Location { path: path.clone(), line: s.line, col: s.col });
            }
        }
        Index { files, types }
    }

    fn file(&self, path: &Path) -> Option<&ProtoFile> {
        self.files.iter().find(|(p, _)| p == path).map(|(_, f)| f)
    }

    /// Full name of the type under (`line`, `col`) of `path`: a reference resolved to its
    /// declaration, or the declaration itself.
    pub fn type_at(&self, path: &Path, line: usize, col: usize) -> Option<String> {
        match self.file(path)?.at(line, col)? {
            Hit::Reference(r) => resolve(&r.name, &r.scope, |n| self.types.contains_key(n)),
            Hit::Declaration(s) => s.is_type().then(|| s.full.clone()),
        }
    }

    /// Whether (`line`, `col`) is on the declaration's own name.
    pub fn is_declaration(&self, path: &Path, line: usize, col: usize) -> bool {
        matches!(self.file(path).and_then(|f| f.at(line, col)), Some(Hit::Declaration(_)))
    }

    pub fn declaration(&self, full: &str) -> Option<&Location> {
        self.types.get(full)
    }

    /// Every reference to the type `full`, in file order.
    pub fn usages(&self, full: &str) -> Vec<Location> {
        let mut out = Vec::new();
        for (path, file) in &self.files {
            for r in &file.references {
                if resolve(&r.name, &r.scope, |n| self.types.contains_key(n)).as_deref() == Some(full) {
                    out.push(Location { path: path.clone(), line: r.line, col: r.col });
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(file: &ProtoFile) -> Vec<(&str, usize)> {
        file.symbols.iter().map(|s| (s.full.as_str(), s.depth)).collect()
    }

    #[test]
    fn lists_declarations_and_ignores_comments_and_strings() {
        let file = parse(
            "// message Fake {}\nservice Greeter {\n  rpc SayHello (Hello) returns (Reply);\n}\nmessage Hello {\n  string value = 1;\n}\noption note = \"service Hidden {}\";\n/* message\n Hidden {} */\n",
        );
        assert_eq!(names(&file), [("Greeter", 0), ("Greeter.SayHello", 1), ("Hello", 0), ("Hello.value", 1)]);
        assert_eq!((file.symbols[1].line, file.symbols[1].col), (2, 6));
    }

    #[test]
    fn qualifies_service_methods() {
        let source = "syntax = \"proto3\";\n/* package wrong; */\npackage shop.v1;\nservice Catalog {\n  rpc Get (Request) returns (Reply);\n  rpc List (Request) returns (stream Reply) {\n    option deprecated = true;\n  }\n  rpc Watch (stream Request) returns (stream Reply);\n}\n";
        assert_eq!(parse(source).methods(), ["shop.v1.Catalog/Get", "shop.v1.Catalog/List", "shop.v1.Catalog/Watch"]);
        assert_eq!(parse("service S { rpc A (R) returns (R); }").methods(), ["S/A"]);
    }

    #[test]
    fn fields_named_like_keywords_are_fields() {
        let file = parse("message Note {\n  string message = 1;\n  service service = 2;\n  repeated enum.Kind kinds = 3;\n}\n");
        assert_eq!(names(&file), [("Note", 0), ("Note.message", 1), ("Note.service", 1), ("Note.kinds", 1)]);
        assert_eq!(file.references.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["service", "enum.Kind"]);
    }

    #[test]
    fn nested_types_oneofs_maps_enums_and_options() {
        let source = r#"syntax = "proto3";
package shop;
import "google/protobuf/timestamp.proto";
import public "common/money.proto";
option csharp_namespace = "Shop.Grpc";
message Order {
  option (custom) = { name: "x" nested { a: 1 } };
  message Line { common.Money price = 1 [(validate.rules) = { gt: 0 }]; }
  enum State { option allow_alias = true; NEW = 0; PAID = 1 [deprecated = true]; }
  repeated Line lines = 1;
  map<string, Line> by_sku = 2;
  oneof payment {
    string card = 3;
    .shop.Wallet wallet = 4;
  }
  google.protobuf.Timestamp created = 5;
  reserved 6 to 8, 10;
  reserved "old";
  State state = 9;
}
message Wallet {}
"#;
        let file = parse(source);
        assert_eq!(file.package, "shop");
        assert_eq!(file.imports, ["google/protobuf/timestamp.proto", "common/money.proto"]);
        assert_eq!(
            names(&file),
            [
                ("shop", 0),
                ("shop.Order", 0),
                ("shop.Order.Line", 1),
                ("shop.Order.Line.price", 2),
                ("shop.Order.State", 1),
                ("shop.Order.State.NEW", 2),
                ("shop.Order.State.PAID", 2),
                ("shop.Order.lines", 1),
                ("shop.Order.by_sku", 1),
                ("shop.Order.payment", 1),
                ("shop.Order.card", 2),
                ("shop.Order.wallet", 2),
                ("shop.Order.created", 1),
                ("shop.Order.state", 1),
                ("shop.Wallet", 0),
            ]
        );
        let refs: Vec<_> = file.references.iter().map(|r| (r.name.as_str(), r.scope.as_str())).collect();
        assert_eq!(
            refs,
            [
                ("common.Money", "shop.Order.Line"),
                ("Line", "shop.Order"),
                ("Line", "shop.Order"),
                (".shop.Wallet", "shop.Order"),
                ("google.protobuf.Timestamp", "shop.Order"),
                ("State", "shop.Order"),
            ]
        );
    }

    #[test]
    fn proto2_groups_extensions_and_extend() {
        let source = "syntax = \"proto2\";\npackage legacy;\nmessage Search {\n  required string query = 1;\n  optional group Result = 2 {\n    optional string url = 3;\n  }\n  extensions 100 to max;\n}\nextend Search {\n  optional int32 rank = 100;\n}\n";
        let file = parse(source);
        assert_eq!(
            names(&file),
            [("legacy", 0), ("legacy.Search", 0), ("legacy.Search.query", 1), ("legacy.Search.Result", 1), ("legacy.Search.Result.url", 2), ("legacy.rank", 1)]
        );
        assert_eq!(file.references[0].name, "Search");
    }

    #[test]
    fn broken_text_ends() {
        for source in ["message", "message A {", "message A { string", "}}}} message B {}", "rpc (", "service S { rpc", "\"unterminated\nmessage C {}", "/* open", "map<", "enum E { A = ", "{{{{", "message M { map<string, > x = 1; }"] {
            let _ = parse(source);
        }
        assert_eq!(names(&parse("}}}} message B {}")), [("B", 0)]);
        assert_eq!(names(&parse("\"unterminated\nmessage C {}")), [("C", 0)]);
    }

    #[test]
    fn resolves_innermost_scope_first() {
        let types = ["shop.Line", "shop.Order.Line", "common.Money"];
        let exists = |n: &str| types.contains(&n);
        assert_eq!(resolve("Line", "shop.Order", exists).as_deref(), Some("shop.Order.Line"));
        assert_eq!(resolve("Line", "shop.Cart", exists).as_deref(), Some("shop.Line"));
        assert_eq!(resolve("common.Money", "shop.Order", exists).as_deref(), Some("common.Money"));
        assert_eq!(resolve(".shop.Line", "shop.Order", exists).as_deref(), Some("shop.Line"));
        assert_eq!(resolve("Missing", "shop", exists), None);
    }

    #[test]
    fn navigates_between_files() {
        let common = PathBuf::from("/w/common.proto");
        let orders = PathBuf::from("/w/orders.proto");
        let index = Index::new(vec![
            (common.clone(), parse("package common;\nmessage Money { int64 units = 1; }\n")),
            (orders.clone(), parse("package shop;\nimport \"common.proto\";\nmessage Order {\n  common.Money total = 1;\n  common.Money tax = 2;\n}\n")),
        ]);
        // On `Money` of `common.Money total`.
        assert_eq!(index.type_at(&orders, 3, 10).as_deref(), Some("common.Money"));
        assert_eq!(index.declaration("common.Money"), Some(&Location { path: common.clone(), line: 1, col: 8 }));
        assert!(index.is_declaration(&common, 1, 9));
        assert_eq!(index.type_at(&common, 1, 9).as_deref(), Some("common.Money"));
        assert_eq!(index.usages("common.Money").iter().map(|l| l.line).collect::<Vec<_>>(), [3, 4]);
    }
}
