//! Discovery of ASP.NET Core HTTP endpoints from C# source, for the "Endpoints" tool window.
//!
//! Works on a small C# lexer (comments, preprocessor lines, regular/verbatim/interpolated/raw
//! strings and char literals are handled) plus a declaration walker, so it needs no compiler and
//! tolerates broken code. Covers:
//!
//! * Attribute-routed controllers: `[Route]` on the class (inherited from base classes found in
//!   the same scan), `[HttpGet("{id}")]`/`[HttpPost]`/..., `[Route]` and `[AcceptVerbs]` on
//!   actions, `[controller]`/`[action]`/`[area]` tokens, `~/` and `/` resets, `[ActionName]`,
//!   `Async` suffix trimming, `[NonAction]`/`[NonController]`.
//! * Conventional controllers (no route attributes): reported as `/{area}/{controller}/{action}`.
//! * Minimal APIs: `MapGet/Post/Put/Delete/Patch/Methods/Map` with `MapGroup` prefixes followed
//!   through chains and variables (`var g = app.MapGroup("/api"); g.MapGet("/a", ...)`).
//!
//! Razor Pages and cross-method group propagation (`app.MapGroup("/x").MapMyEndpoints()`) are not
//! covered.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EndpointKind {
    Controller,
    MinimalApi,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Endpoint {
    /// `GET`, `POST`, ... or `ANY`.
    pub method: String,
    /// Full route, e.g. `/api/orders/{id}`.
    pub route: String,
    pub file: PathBuf,
    /// 0-based line of the action name or of the `MapXxx` call.
    pub line: usize,
    /// `OrdersController.Get`, or the minimal API handler (`lambda` or a method group name).
    pub handler: String,
    pub kind: EndpointKind,
}

/// Scans every `.cs` file under `project_dir` (skipping `bin`, `obj`, hidden folders,
/// `node_modules` and nested projects). Results are sorted by file and line.
pub fn scan_project(project_dir: &Path) -> Vec<Endpoint> {
    let mut files = Vec::new();
    collect_cs_files(project_dir, true, &mut files);
    files.sort();
    let mut classes = Vec::new();
    let mut out = Vec::new();
    for f in &files {
        let Ok(text) = std::fs::read_to_string(f) else { continue };
        let toks = lex(&text);
        classes.extend(parse_classes(&toks, f));
        out.extend(scan_minimal(&toks, f));
    }
    out.extend(resolve_controllers(&classes));
    sort_endpoints(&mut out);
    out
}

/// Scans a single C# source. Controller base classes are only resolved within this text.
pub fn scan_source(path: &Path, text: &str) -> Vec<Endpoint> {
    let toks = lex(text);
    let classes = parse_classes(&toks, path);
    let mut out = scan_minimal(&toks, path);
    out.extend(resolve_controllers(&classes));
    sort_endpoints(&mut out);
    out
}

fn sort_endpoints(v: &mut Vec<Endpoint>) {
    v.sort_by(|a, b| {
        (&a.file, a.line, &a.route, &a.method).cmp(&(&b.file, b.line, &b.route, &b.method))
    });
    v.dedup();
}

fn collect_cs_files(dir: &Path, root: bool, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let entries: Vec<_> = rd.flatten().map(|e| e.path()).collect();
    if !root
        && entries
            .iter()
            .any(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("csproj")))
    {
        return; // nested project: it gets its own scan
    }
    for p in entries {
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or_default();
        if p.is_dir() {
            let lower = name.to_ascii_lowercase();
            if name.starts_with('.') || matches!(lower.as_str(), "bin" | "obj" | "node_modules") {
                continue;
            }
            collect_cs_files(&p, false, out);
        } else if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("cs")) {
            out.push(p);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Lexer

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Ident(String),
    Str(String),
    Punct(&'static str),
    Other,
}

#[derive(Debug, Clone)]
struct Token {
    tok: Tok,
    line: usize,
}

const PUNCT3: &[&str] = &["??="];
const PUNCT2: &[&str] = &[
    "=>", "==", "!=", "<=", ">=", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "??", "?.",
    "::", "&&", "||", "++", "--", "->",
];
const PUNCT1: &[&str] = &[
    "{", "}", "(", ")", "[", "]", "<", ">", ";", ",", ".", ":", "=", "?", "!", "~", "+", "-",
    "*", "/", "%", "&", "|", "^", "@", "#", "$",
];

fn lex(src: &str) -> Vec<Token> {
    let c: Vec<char> = src.chars().collect();
    let n = c.len();
    let mut out = Vec::new();
    let (mut i, mut line, mut line_start) = (0usize, 0usize, true);
    let at = |k: usize| c.get(k).copied().unwrap_or('\0');
    while i < n {
        let ch = c[i];
        if ch == '\n' {
            line += 1;
            i += 1;
            line_start = true;
            continue;
        }
        if ch.is_whitespace() {
            i += 1;
            continue;
        }
        if ch == '#' && line_start {
            while i < n && c[i] != '\n' {
                i += 1;
            }
            continue;
        }
        line_start = false;
        if ch == '/' && at(i + 1) == '/' {
            while i < n && c[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if ch == '/' && at(i + 1) == '*' {
            i += 2;
            while i < n && !(c[i] == '*' && at(i + 1) == '/') {
                if c[i] == '\n' {
                    line += 1;
                }
                i += 1;
            }
            i = (i + 2).min(n);
            continue;
        }
        if let Some((value, end)) = lex_string(&c, i) {
            out.push(Token { tok: Tok::Str(value), line });
            line += c[i..end].iter().filter(|&&x| x == '\n').count();
            i = end;
            continue;
        }
        if ch == '\'' {
            i += 1;
            while i < n && c[i] != '\'' && c[i] != '\n' {
                if c[i] == '\\' {
                    i += 1;
                }
                i += 1;
            }
            i = (i + 1).min(n);
            out.push(Token { tok: Tok::Other, line });
            continue;
        }
        let ident_start = |x: char| x.is_alphabetic() || x == '_';
        if ident_start(ch) || (ch == '@' && ident_start(at(i + 1))) {
            let start = if ch == '@' { i + 1 } else { i };
            i = start;
            while i < n && (c[i].is_alphanumeric() || c[i] == '_') {
                i += 1;
            }
            out.push(Token { tok: Tok::Ident(c[start..i].iter().collect()), line });
            continue;
        }
        if ch.is_ascii_digit() {
            while i < n
                && (c[i].is_alphanumeric() || c[i] == '_' || (c[i] == '.' && at(i + 1).is_ascii_digit()))
            {
                i += 1;
            }
            out.push(Token { tok: Tok::Other, line });
            continue;
        }
        let rest = |len: usize| c[i..(i + len).min(n)].iter().collect::<String>();
        let p = PUNCT3
            .iter()
            .find(|p| rest(3) == **p)
            .or_else(|| PUNCT2.iter().find(|p| rest(2) == **p))
            .or_else(|| PUNCT1.iter().find(|p| rest(1) == **p));
        match p {
            Some(p) => {
                i += p.chars().count();
                out.push(Token { tok: Tok::Punct(p), line });
            }
            None => {
                i += 1;
                out.push(Token { tok: Tok::Other, line });
            }
        }
    }
    out
}

/// Lexes a string literal starting at `i` (with optional `$`/`@` prefixes). Returns its value
/// (escapes kept mostly raw, interpolation holes kept as text) and the index after it.
fn lex_string(c: &[char], i: usize) -> Option<(String, usize)> {
    let n = c.len();
    let at = |k: usize| c.get(k).copied().unwrap_or('\0');
    let mut j = i;
    let mut dollars = 0;
    let mut verbatim = false;
    while at(j) == '$' {
        dollars += 1;
        j += 1;
    }
    if at(j) == '@' {
        verbatim = true;
        j += 1;
        while at(j) == '$' {
            dollars += 1;
            j += 1;
        }
    }
    if at(j) != '"' {
        return None;
    }
    let mut q = 0;
    while at(j + q) == '"' {
        q += 1;
    }
    if q >= 3 {
        // Raw string literal: ends at the same number of quotes.
        let start = j + q;
        let mut k = start;
        while k < n {
            if c[k] == '"' {
                let mut run = 0;
                while at(k + run) == '"' {
                    run += 1;
                }
                if run >= q {
                    let value: String = c[start..k].iter().collect();
                    return Some((value.trim().to_string(), k + run));
                }
                k += run;
            } else {
                k += 1;
            }
        }
        return Some((c[start..n].iter().collect(), n));
    }
    if q == 2 && !verbatim {
        return Some((String::new(), j + 2));
    }
    let mut k = j + 1;
    let mut value = String::new();
    while k < n {
        let ch = c[k];
        if verbatim && ch == '"' && at(k + 1) == '"' {
            value.push('"');
            k += 2;
            continue;
        }
        if ch == '"' {
            return Some((value, k + 1));
        }
        if !verbatim && ch == '\\' {
            value.push(at(k + 1));
            k += 2;
            continue;
        }
        if !verbatim && ch == '\n' {
            return Some((value, k));
        }
        if dollars > 0 && ch == '{' {
            if at(k + 1) == '{' {
                value.push('{');
                k += 2;
                continue;
            }
            let mut depth = 0;
            while k < n {
                let x = c[k];
                if let Some((_, end)) = lex_string(c, k).filter(|_| x == '"' || x == '$' || x == '@') {
                    value.extend(&c[k..end]);
                    k = end;
                    continue;
                }
                if x == '{' {
                    depth += 1;
                } else if x == '}' {
                    depth -= 1;
                }
                value.push(x);
                k += 1;
                if depth == 0 {
                    break;
                }
            }
            continue;
        }
        if dollars > 0 && ch == '}' && at(k + 1) == '}' {
            value.push('}');
            k += 2;
            continue;
        }
        value.push(ch);
        k += 1;
    }
    Some((value, n))
}

// ---------------------------------------------------------------------------------------------
// Token helpers

fn punct(t: &[Token], i: usize) -> Option<&'static str> {
    match t.get(i).map(|x| &x.tok) {
        Some(Tok::Punct(p)) => Some(p),
        _ => None,
    }
}

fn is(t: &[Token], i: usize, p: &str) -> bool {
    punct(t, i) == Some(p)
}

fn ident(t: &[Token], i: usize) -> Option<&str> {
    match t.get(i).map(|x| &x.tok) {
        Some(Tok::Ident(s)) => Some(s.as_str()),
        _ => None,
    }
}

fn is_open(p: Option<&str>) -> bool {
    matches!(p, Some("(" | "[" | "{"))
}

fn is_close(p: Option<&str>) -> bool {
    matches!(p, Some(")" | "]" | "}"))
}

/// `t[i]` is an opening bracket; returns the index after its matching close.
fn skip_balanced(t: &[Token], mut i: usize) -> usize {
    let mut depth = 0usize;
    while i < t.len() {
        let p = punct(t, i);
        if is_open(p) {
            depth += 1;
        } else if is_close(p) {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return i + 1;
            }
        }
        i += 1;
    }
    t.len()
}

/// Index of the `;` (or unbalanced closer) ending the statement that contains `i`.
fn stmt_end(t: &[Token], mut i: usize) -> usize {
    let mut depth = 0usize;
    while i < t.len() {
        let p = punct(t, i);
        if is_open(p) {
            depth += 1;
        } else if is_close(p) {
            if depth == 0 {
                return i;
            }
            depth -= 1;
        } else if depth == 0 && p == Some(";") {
            return i;
        }
        i += 1;
    }
    t.len()
}

/// Splits `t[from..to]` at depth-0 commas.
fn split_args(t: &[Token], from: usize, to: usize) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut start = from;
    for i in from..to {
        let p = punct(t, i);
        if is_open(p) {
            depth += 1;
        } else if is_close(p) {
            depth = depth.saturating_sub(1);
        } else if depth == 0 && p == Some(",") {
            out.push((start, i));
            start = i + 1;
        }
    }
    if start < to {
        out.push((start, to));
    }
    out
}

fn strings_in(t: &[Token], from: usize, to: usize) -> Vec<String> {
    t[from..to]
        .iter()
        .filter_map(|x| match &x.tok {
            Tok::Str(s) => Some(s.clone()),
            _ => None,
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Declarations

#[derive(Debug, Clone)]
struct AttrArg {
    name: Option<String>,
    strings: Vec<String>,
}

#[derive(Debug, Clone)]
struct Attr {
    name: String,
    args: Vec<AttrArg>,
}

impl Attr {
    /// Concatenated string literals of the first positional argument.
    fn positional(&self) -> Option<String> {
        self.args
            .iter()
            .find(|a| a.name.is_none())
            .filter(|a| !a.strings.is_empty())
            .map(|a| a.strings.concat())
    }

    fn named(&self, name: &str) -> Option<String> {
        self.args
            .iter()
            .find(|a| a.name.as_deref().is_some_and(|n| n.eq_ignore_ascii_case(name)))
            .filter(|a| !a.strings.is_empty())
            .map(|a| a.strings.concat())
    }
}

#[derive(Debug, Clone)]
struct MethodInfo {
    name: String,
    line: usize,
    attrs: Vec<Attr>,
    is_public: bool,
    is_static: bool,
}

#[derive(Debug, Clone)]
struct ClassInfo {
    name: String,
    ns: String,
    file: PathBuf,
    attrs: Vec<Attr>,
    bases: Vec<String>,
    is_public: bool,
    is_abstract: bool,
    is_static: bool,
    is_generic: bool,
    methods: Vec<MethodInfo>,
}

fn parse_classes(t: &[Token], file: &Path) -> Vec<ClassInfo> {
    let mut classes = Vec::new();
    parse_block(t, 0, "", None, file, &mut classes);
    classes
}

/// `t[i]` is `[`. Parses one attribute section; returns its attributes and the index after `]`.
fn parse_attr_section(t: &[Token], i: usize) -> (Vec<Attr>, usize) {
    let end = skip_balanced(t, i);
    let close = end.saturating_sub(1).max(i + 1);
    let mut k = i + 1;
    // Attribute target (`[return: X]`, `[assembly: X]`): not about the declaration.
    if ident(t, k).is_some() && is(t, k + 1, ":") {
        if matches!(ident(t, k), Some("assembly" | "module" | "return" | "param")) {
            return (Vec::new(), end);
        }
        k += 2;
    }
    let mut attrs = Vec::new();
    for (from, to) in split_args(t, k, close) {
        let mut j = from;
        let mut name = None;
        while j < to {
            if let Some(id) = ident(t, j) {
                name = Some(id.to_string());
                j += 1;
            } else if is(t, j, ".") || is(t, j, "::") {
                j += 1;
            } else {
                break;
            }
        }
        let Some(mut name) = name else { continue };
        if name.len() > "Attribute".len() && name.ends_with("Attribute") {
            name.truncate(name.len() - "Attribute".len());
        }
        if is(t, j, "<") {
            while j < to && !is(t, j, "(") {
                j += 1;
            }
        }
        let mut args = Vec::new();
        if is(t, j, "(") {
            let close_paren = skip_balanced(t, j) - 1;
            for (a, b) in split_args(t, j + 1, close_paren) {
                let named = ident(t, a).filter(|_| is(t, a + 1, "=") || is(t, a + 1, ":"));
                let from = if named.is_some() { a + 2 } else { a };
                args.push(AttrArg { name: named.map(str::to_string), strings: strings_in(t, from, b) });
            }
        }
        attrs.push(Attr { name, args });
    }
    (attrs, end)
}

const TYPE_KEYWORDS: &[&str] = &["class", "struct", "interface", "enum", "record"];

/// Parses declarations until the `}` closing the current scope; returns the index after it.
fn parse_block(
    t: &[Token],
    mut i: usize,
    ns: &str,
    class: Option<usize>,
    file: &Path,
    classes: &mut Vec<ClassInfo>,
) -> usize {
    let mut ns = ns.to_string();
    loop {
        if i >= t.len() {
            return t.len();
        }
        if is(t, i, "}") {
            return i + 1;
        }
        if is(t, i, ";") {
            i += 1;
            continue;
        }
        let mut attrs = Vec::new();
        while is(t, i, "[") {
            let (a, next) = parse_attr_section(t, i);
            attrs.extend(a);
            i = next;
        }
        // Header: everything up to a depth-0 `{`, `;`, `=>` or `}`.
        let start = i;
        let mut depth = 0usize;
        let mut first_paren = None;
        let mut assign = false;
        let mut term = None;
        while i < t.len() {
            let p = punct(t, i);
            if depth == 0 {
                if matches!(p, Some("{" | ";" | "=>" | "}")) {
                    term = p;
                    break;
                }
                if p == Some("(") && first_paren.is_none() {
                    first_paren = Some(i - start);
                }
                if p == Some("=") {
                    assign = true;
                }
            }
            if is_open(p) {
                depth += 1;
            } else if is_close(p) {
                depth = depth.saturating_sub(1);
            }
            i += 1;
        }
        let header = &t[start..i];
        if header.is_empty() {
            if term == Some("}") {
                return i + 1;
            }
            if term.is_none() {
                return t.len();
            }
        }
        let limit = first_paren.unwrap_or(header.len());
        let kw = (0..limit).find(|&k| ident(header, k).is_some_and(|s| TYPE_KEYWORDS.contains(&s)));

        if ident(header, 0) == Some("namespace") {
            let name: String = header[1..]
                .iter()
                .map(|x| match &x.tok {
                    Tok::Ident(s) => s.as_str(),
                    Tok::Punct(".") => ".",
                    _ => "",
                })
                .collect();
            let full = if ns.is_empty() { name } else { format!("{ns}.{name}") };
            if term == Some("{") {
                i = parse_block(t, i + 1, &full, None, file, classes);
            } else {
                ns = full;
                i += 1;
            }
            continue;
        }

        if let Some(kw) = kw.filter(|_| !assign || term == Some("{")) {
            let keyword = ident(header, kw).unwrap_or_default();
            let mut n = kw + 1;
            if keyword == "record" && matches!(ident(header, n), Some("class" | "struct")) {
                n += 1;
            }
            let name = ident(header, n).unwrap_or_default().to_string();
            let mods: Vec<&str> = (0..kw).filter_map(|k| ident(header, k)).collect();
            let is_generic = is(header, n + 1, "<");
            let mut bases = Vec::new();
            // Base list: after a `:` outside of `<...>`/`(...)`, up to `where`.
            let mut k = n + 1;
            let mut d = 0i32;
            while k < header.len() {
                match punct(header, k) {
                    Some("<" | "(") => d += 1,
                    Some(">" | ")") => d -= 1,
                    Some(":") if d == 0 => break,
                    _ => {}
                }
                k += 1;
            }
            if k < header.len() {
                let end = (k + 1..header.len())
                    .find(|&x| ident(header, x) == Some("where"))
                    .unwrap_or(header.len());
                let mut d = 0i32;
                let mut last = None;
                for x in k + 1..end {
                    match punct(header, x) {
                        Some("<" | "(") => d += 1,
                        Some(">" | ")") => d -= 1,
                        Some(",") if d == 0 => {
                            bases.extend(last.take());
                        }
                        _ => {
                            if d == 0
                                && let Some(id) = ident(header, x)
                            {
                                last = Some(id.to_string());
                            }
                        }
                    }
                }
                bases.extend(last);
            }
            let info = ClassInfo {
                name,
                ns: ns.clone(),
                file: file.to_path_buf(),
                attrs,
                bases,
                is_public: mods.contains(&"public"),
                is_abstract: mods.contains(&"abstract"),
                is_static: mods.contains(&"static"),
                is_generic,
                methods: Vec::new(),
            };
            match term {
                Some("{") if keyword != "interface" && keyword != "enum" => {
                    classes.push(info);
                    let idx = classes.len() - 1;
                    i = parse_block(t, i + 1, &ns, Some(idx), file, classes);
                }
                Some("{") => i = skip_balanced(t, i),
                Some(";") => {
                    classes.push(info);
                    i += 1;
                }
                _ => i += 1,
            }
            continue;
        }

        if let (Some(ci), Some(fp), false) = (class, first_paren, assign) {
            let mut name_idx = fp.checked_sub(1);
            if let Some(ni) = name_idx.filter(|&ni| is(header, ni, ">")) {
                let mut d = 0i32;
                let mut k = ni as isize;
                while k >= 0 {
                    match punct(header, k as usize) {
                        Some(">") => d += 1,
                        Some("<") => {
                            d -= 1;
                            if d == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    k -= 1;
                }
                name_idx = (k as usize).checked_sub(1);
            }
            let excluded = header.iter().any(|x| {
                matches!(&x.tok, Tok::Ident(s) if matches!(s.as_str(), "operator" | "delegate" | "event"))
            });
            if let Some(ni) = name_idx
                && let Some(name) = ident(header, ni)
                && !excluded
                && name != classes[ci].name
                && !(ni > 0 && (is(header, ni - 1, "~") || is(header, ni - 1, ".")))
            {
                let mods: Vec<&str> = (0..ni).filter_map(|k| ident(header, k)).collect();
                let info = MethodInfo {
                    name: name.to_string(),
                    line: header[ni].line,
                    attrs,
                    is_public: mods.contains(&"public"),
                    is_static: mods.contains(&"static"),
                };
                classes[ci].methods.push(info);
            }
        }

        i = match term {
            Some("{") if assign => stmt_end(t, i) + 1,
            Some("{") => skip_balanced(t, i),
            Some("=>") => stmt_end(t, i) + 1,
            Some(";") => i + 1,
            Some("}") => return i + 1,
            _ => t.len(),
        };
    }
}

// ---------------------------------------------------------------------------------------------
// Controllers

struct Group<'a> {
    name: &'a str,
    ns: &'a str,
    parts: Vec<&'a ClassInfo>,
}

impl Group<'_> {
    fn attrs(&self) -> impl Iterator<Item = &Attr> {
        self.parts.iter().flat_map(|p| p.attrs.iter())
    }
    fn bases(&self) -> impl Iterator<Item = &str> {
        self.parts.iter().flat_map(|p| p.bases.iter().map(String::as_str))
    }
    fn has_attr(&self, name: &str) -> bool {
        self.attrs().any(|a| a.name == name)
    }
}

const HTTP_ATTRS: &[(&str, &str)] = &[
    ("HttpGet", "GET"),
    ("HttpPost", "POST"),
    ("HttpPut", "PUT"),
    ("HttpDelete", "DELETE"),
    ("HttpPatch", "PATCH"),
    ("HttpHead", "HEAD"),
    ("HttpOptions", "OPTIONS"),
];

fn resolve_controllers(classes: &[ClassInfo]) -> Vec<Endpoint> {
    let mut groups: Vec<Group> = Vec::new();
    let mut index: HashMap<(&str, &str), usize> = HashMap::new();
    for c in classes {
        let key = (c.ns.as_str(), c.name.as_str());
        match index.get(&key) {
            Some(&g) => groups[g].parts.push(c),
            None => {
                index.insert(key, groups.len());
                groups.push(Group { name: &c.name, ns: &c.ns, parts: vec![c] });
            }
        }
    }
    let find_base = |name: &str, ns: &str| -> Option<usize> {
        index.get(&(ns, name)).copied().or_else(|| {
            groups.iter().position(|g| g.name == name)
        })
    };
    let ancestors = |g: usize| -> Vec<usize> {
        let mut out = Vec::new();
        let mut seen = HashSet::from([g]);
        let mut cur = g;
        for _ in 0..16 {
            let next = groups[cur].bases().find_map(|b| find_base(b, groups[cur].ns));
            match next {
                Some(nx) if seen.insert(nx) => {
                    out.push(nx);
                    cur = nx;
                }
                _ => break,
            }
        }
        out
    };

    let mut out = Vec::new();
    for (gi, g) in groups.iter().enumerate() {
        let anc = ancestors(gi);
        let chain: Vec<&Group> = std::iter::once(g).chain(anc.iter().map(|&a| &groups[a])).collect();
        let public = g.parts.iter().any(|p| p.is_public);
        let unusable = g.parts.iter().any(|p| p.is_abstract || p.is_static || p.is_generic);
        if !public || unusable || chain.iter().any(|c| c.has_attr("NonController")) {
            continue;
        }
        let is_controller = g.name.ends_with("Controller")
            || chain.iter().any(|c| c.has_attr("Controller") || c.has_attr("ApiController"))
            || chain
                .iter()
                .flat_map(|c| c.bases())
                .any(|b| b.ends_with("Controller") || b.ends_with("ControllerBase"));
        if !is_controller {
            continue;
        }
        let class_routes: Vec<String> = chain
            .iter()
            .map(|c| c.attrs().filter(|a| a.name == "Route").map(|a| a.positional().or_else(|| a.named("Template")).unwrap_or_default()).collect::<Vec<_>>())
            .find(|v| !v.is_empty())
            .unwrap_or_default();
        let area = chain
            .iter()
            .flat_map(|c| c.attrs())
            .find(|a| a.name == "Area")
            .and_then(|a| a.positional().or_else(|| a.named("AreaName")));
        let controller = g.name.strip_suffix("Controller").filter(|s| !s.is_empty()).unwrap_or(g.name);

        // Own methods first, then inherited ones that are not redeclared.
        let mut names = HashSet::new();
        let mut methods: Vec<(&MethodInfo, &Path)> = Vec::new();
        for c in &chain {
            let mut level = Vec::new();
            for p in &c.parts {
                for m in &p.methods {
                    if !names.contains(m.name.as_str()) {
                        level.push((m, p.file.as_path()));
                    }
                }
            }
            names.extend(level.iter().map(|(m, _)| m.name.as_str()));
            methods.extend(level);
        }

        for (m, file) in methods {
            if !m.is_public || m.is_static || m.attrs.iter().any(|a| a.name == "NonAction") {
                continue;
            }
            let action_name = m
                .attrs
                .iter()
                .find(|a| a.name == "ActionName")
                .and_then(Attr::positional)
                .unwrap_or_else(|| {
                    m.name
                        .strip_suffix("Async")
                        .filter(|s| !s.is_empty())
                        .unwrap_or(&m.name)
                        .to_string()
                });
            // (template, verbs) per routing attribute.
            let mut selectors: Vec<(Option<String>, Vec<String>)> = Vec::new();
            for a in &m.attrs {
                if a.name == "Route" {
                    selectors.push((a.positional().or_else(|| a.named("Template")), Vec::new()));
                } else if let Some((_, verb)) = HTTP_ATTRS.iter().find(|(n, _)| *n == a.name) {
                    let tpl = a.positional().or_else(|| a.named("Template"));
                    selectors.push((tpl, vec![verb.to_string()]));
                } else if a.name == "AcceptVerbs" {
                    let verbs = a
                        .args
                        .iter()
                        .filter(|x| x.name.is_none())
                        .flat_map(|x| x.strings.iter().map(|s| s.to_ascii_uppercase()))
                        .collect();
                    selectors.push((a.named("Route"), verbs));
                }
            }
            let loose_verbs: Vec<String> = selectors
                .iter()
                .filter(|(t, _)| t.is_none())
                .flat_map(|(_, v)| v.iter().cloned())
                .collect();
            let templated: Vec<(String, Vec<String>)> = selectors
                .iter()
                .filter_map(|(t, v)| {
                    t.clone().map(|t| (t, if v.is_empty() { loose_verbs.clone() } else { v.clone() }))
                })
                .collect();

            let mut routes: Vec<(String, Vec<String>)> = Vec::new();
            if !templated.is_empty() {
                for (tpl, verbs) in &templated {
                    if is_absolute(tpl) || class_routes.is_empty() {
                        routes.push((join_route("", tpl), verbs.clone()));
                    } else {
                        for cr in &class_routes {
                            routes.push((join_route(cr, tpl), verbs.clone()));
                        }
                    }
                }
            } else if !class_routes.is_empty() {
                for cr in &class_routes {
                    routes.push((join_route(cr, ""), loose_verbs.clone()));
                }
            } else {
                // Conventional routing: {area}/{controller}/{action}.
                let mut r = String::new();
                if let Some(area) = &area {
                    r.push_str(area);
                    r.push('/');
                }
                r.push_str(controller);
                r.push('/');
                r.push_str(&action_name);
                routes.push((join_route("", &r), loose_verbs.clone()));
            }

            for (route, verbs) in routes {
                let route = replace_tokens(&route, controller, &action_name, area.as_deref().unwrap_or(""));
                let route = join_route("", &route);
                let verbs = if verbs.is_empty() { vec!["ANY".to_string()] } else { verbs };
                for v in verbs {
                    out.push(Endpoint {
                        method: v,
                        route: route.clone(),
                        file: file.to_path_buf(),
                        line: m.line,
                        handler: format!("{}.{}", g.name, m.name),
                        kind: EndpointKind::Controller,
                    });
                }
            }
        }
    }
    out
}

fn is_absolute(tpl: &str) -> bool {
    tpl.starts_with('/') || tpl.starts_with("~/")
}

/// Joins two route templates with single slashes; result starts with `/`, no trailing slash.
fn join_route(prefix: &str, tpl: &str) -> String {
    let strip = |s: &str| s.strip_prefix('~').unwrap_or(s).to_string();
    let joined = format!("{}/{}", strip(prefix), strip(tpl));
    let segs: Vec<&str> = joined.split('/').filter(|s| !s.is_empty()).collect();
    format!("/{}", segs.join("/"))
}

fn replace_tokens(route: &str, controller: &str, action: &str, area: &str) -> String {
    let mut out = String::with_capacity(route.len());
    let mut rest = route;
    while let Some(pos) = rest.find('[') {
        out.push_str(&rest[..pos]);
        let after = &rest[pos..];
        let Some(end) = after.find(']') else {
            out.push_str(after);
            return out;
        };
        let token = &after[1..end];
        let value = if token.eq_ignore_ascii_case("controller") {
            Some(controller)
        } else if token.eq_ignore_ascii_case("action") {
            Some(action)
        } else if token.eq_ignore_ascii_case("area") {
            Some(area)
        } else {
            None
        };
        match value {
            Some(v) => out.push_str(v),
            None => out.push_str(&after[..=end]),
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

// ---------------------------------------------------------------------------------------------
// Minimal APIs

const MAP_METHODS: &[(&str, &str)] = &[
    ("MapGet", "GET"),
    ("MapPost", "POST"),
    ("MapPut", "PUT"),
    ("MapDelete", "DELETE"),
    ("MapPatch", "PATCH"),
    ("MapMethods", ""),
    ("Map", "ANY"),
];

/// Route prefix of the expression ending at `t[idx]`, when it is (or derives from) a route group.
fn group_prefix(t: &[Token], idx: usize, groups: &HashMap<String, String>, budget: usize) -> Option<String> {
    if budget == 0 {
        return None;
    }
    match &t.get(idx)?.tok {
        Tok::Ident(name) => groups.get(name).cloned(),
        Tok::Punct(")") => {
            // Find the matching `(`.
            let mut depth = 0usize;
            let mut k = idx;
            loop {
                let p = punct(t, k);
                if is_close(p) {
                    depth += 1;
                } else if is_open(p) {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                k = k.checked_sub(1)?;
            }
            let m = k.checked_sub(1)?;
            let name = ident(t, m)?;
            let recv = if m >= 2 && is(t, m - 1, ".") {
                group_prefix(t, m - 2, groups, budget - 1)
            } else {
                None
            };
            if name == "MapGroup" {
                let args = split_args(t, k + 1, idx);
                let pattern = args.first().map(|&(a, b)| strings_in(t, a, b).concat())?;
                Some(join_route(recv.as_deref().unwrap_or(""), &pattern))
            } else {
                recv
            }
        }
        _ => None,
    }
}

fn scan_minimal(t: &[Token], file: &Path) -> Vec<Endpoint> {
    let mut groups: HashMap<String, String> = HashMap::new();
    let mut out = Vec::new();
    for i in 0..t.len() {
        if is(t, i, "=") && i > 0 {
            if let Some(var) = ident(t, i - 1) {
                let end = stmt_end(t, i + 1);
                match (end > i + 1).then(|| group_prefix(t, end - 1, &groups, 64)).flatten() {
                    Some(p) => {
                        groups.insert(var.to_string(), p);
                    }
                    None => {
                        groups.remove(var);
                    }
                }
            }
            continue;
        }
        let Some(name) = ident(t, i) else { continue };
        let Some(&(_, verb)) = MAP_METHODS.iter().find(|(n, _)| *n == name) else { continue };
        if i < 2 || !is(t, i - 1, ".") || !is(t, i + 1, "(") {
            continue;
        }
        let close = skip_balanced(t, i + 1) - 1;
        let args = split_args(t, i + 2, close);
        let Some(&(a0, b0)) = args.first() else { continue };
        if !(a0..b0).all(|k| matches!(t[k].tok, Tok::Str(_)) || is(t, k, "+")) || a0 == b0 {
            continue; // pattern is not a literal
        }
        let pattern = strings_in(t, a0, b0).concat();
        let (verbs, handler_arg) = if name == "MapMethods" {
            let verbs: Vec<String> = args
                .get(1)
                .map(|&(a, b)| strings_in(t, a, b).iter().map(|s| s.to_ascii_uppercase()).collect())
                .unwrap_or_default();
            (verbs, args.get(2))
        } else {
            (vec![verb.to_string()], args.get(1))
        };
        let verbs = if verbs.is_empty() { vec!["ANY".to_string()] } else { verbs };
        let handler = match handler_arg {
            Some(&(a, b)) => {
                let lambda = (a..b).any(|k| is(t, k, "=>")) || ident(t, a) == Some("delegate");
                let simple = (a..b).all(|k| ident(t, k).is_some() || is(t, k, "."));
                if lambda {
                    "lambda".to_string()
                } else if simple && a < b {
                    t[a..b]
                        .iter()
                        .map(|x| match &x.tok {
                            Tok::Ident(s) => s.as_str(),
                            _ => ".",
                        })
                        .collect()
                } else {
                    "handler".to_string()
                }
            }
            None => "handler".to_string(),
        };
        let prefix = group_prefix(t, i - 2, &groups, 64).unwrap_or_default();
        let route = join_route(&prefix, &pattern);
        for v in verbs {
            out.push(Endpoint {
                method: v,
                route: route.clone(),
                file: file.to_path_buf(),
                line: t[i].line,
                handler: handler.clone(),
                kind: EndpointKind::MinimalApi,
            });
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn scan(src: &str) -> Vec<(String, String, String, usize)> {
        scan_source(Path::new("X.cs"), src)
            .into_iter()
            .map(|e| (e.method, e.route, e.handler, e.line))
            .collect()
    }

    fn routes(src: &str) -> Vec<(String, String)> {
        scan(src).into_iter().map(|(m, r, _, _)| (m, r)).collect()
    }

    fn pairs(v: &[(&str, &str)]) -> Vec<(String, String)> {
        v.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
    }

    #[test]
    fn endpoints_api_controller_basic() {
        let src = r#"
using Microsoft.AspNetCore.Mvc;
namespace Shop.Api.Controllers;

[ApiController]
[Route("api/[controller]")]
public class OrdersController : ControllerBase
{
    private readonly string[] _x = new[] { "a", "b" };
    public OrdersController(ILogger<OrdersController> log) { }

    [HttpGet]
    public IActionResult List() => Ok();

    [HttpGet("{id:int}")]
    public async Task<IActionResult> GetAsync(int id) { return Ok(); }

    [HttpPost]
    public IActionResult Create([FromBody] Order o) { if (o == null) { return BadRequest(); } return Ok(); }

    [HttpDelete("~/admin/orders/{id}")]
    public IActionResult Delete(int id) => Ok();

    [NonAction]
    public void Helper() { }

    private void Hidden() { }
    public static void Util() { }
    public int Count { get; set; } = 5;
}
"#;
        let got = scan(src);
        assert_eq!(
            got.iter().map(|(m, r, _, _)| (m.clone(), r.clone())).collect::<Vec<_>>(),
            pairs(&[
                ("GET", "/api/Orders"),
                ("GET", "/api/Orders/{id:int}"),
                ("POST", "/api/Orders"),
                ("DELETE", "/admin/orders/{id}"),
            ])
        );
        assert_eq!(got[1].2, "OrdersController.GetAsync");
        assert_eq!(got[0].3, 12);
        assert_eq!(got[1].3, 15);
    }

    #[test]
    fn endpoints_route_and_action_tokens() {
        let src = r#"
[Route("api/[controller]/[action]")]
public class ReportsController : Controller
{
    public IActionResult Daily() => View();
    [HttpPost, ActionName("run-now")]
    public IActionResult Run() => View();
    [Route("/custom/{x}")]
    [HttpGet]
    public IActionResult Custom(string x) => View();
    [AcceptVerbs("GET", "post", Route = "both")]
    public IActionResult Both() => View();
}
"#;
        assert_eq!(
            routes(src),
            pairs(&[
                ("ANY", "/api/Reports/Daily"),
                ("POST", "/api/Reports/run-now"),
                ("GET", "/custom/{x}"),
                ("GET", "/api/Reports/Both/both"),
                ("POST", "/api/Reports/Both/both"),
            ])
        );
    }

    #[test]
    fn endpoints_areas_conventional_and_non_controllers() {
        let src = r#"
namespace App.Areas.Admin
{
    [Area("Admin")]
    [Route("[area]/[controller]")]
    public class UsersController : Controller
    {
        [HttpGet("")] public IActionResult Index() => View();
    }

    public class HomeController : Controller
    {
        public IActionResult Index() => View();
        [HttpPost] public IActionResult Save() => View();
    }

    [NonController] public class FakeController { public void X() {} }
    public abstract class BaseApiController : ControllerBase { public void Y() {} }
    public class NotAController { }
    public class Service { public void Z() {} }
}
"#;
        assert_eq!(
            routes(src),
            pairs(&[
                ("GET", "/Admin/Users"),
                ("ANY", "/Home/Index"),
                ("POST", "/Home/Save"),
            ])
        );
    }

    #[test]
    fn endpoints_inherited_route_and_actions() {
        let src = r#"
[ApiController]
[Route("api/v1/[controller]")]
public abstract class ApiBase : ControllerBase
{
    [HttpGet("ping")] public string Ping() => "pong";
}
public class ItemsController : ApiBase
{
    [HttpGet("{id}")] public string Get(int id) => "";
}
"#;
        assert_eq!(
            routes(src),
            pairs(&[("GET", "/api/v1/Items/ping"), ("GET", "/api/v1/Items/{id}")])
        );
    }

    #[test]
    fn endpoints_comments_and_strings_ignored() {
        let src = r#"
// [Route("bad")] public class XController {}
/* app.MapGet("/nope", () => 1); */
var s = "app.MapGet(\"/nope2\", () => 1);";
var v = @"app.MapGet(""/nope3"", x)";
var r = """
    app.MapGet("/nope4", () => 1);
    """;
var i = $"{(true ? "a" : "b")} app.MapGet(\"/nope5\")";
#if DEBUG
app.MapGet("/debug", () => 1);
#endif
app.MapGet("/yes", () => 1);
"#;
        let got = scan(src);
        assert_eq!(got.len(), 2, "{got:?}");
        assert_eq!(got[0].1, "/debug");
        assert_eq!(got[1].1, "/yes");
        assert_eq!(got[1].3, 12);
    }

    #[test]
    fn endpoints_minimal_api_groups() {
        let src = r#"
var app = builder.Build();
app.MapGet("/", () => "hi");
app.MapPost("/todos", async (Todo t, Db db) => { await db.SaveAsync(); return Results.Ok(); });
var api = app.MapGroup("/api").WithTags("Api").RequireAuthorization();
api.MapGet("/a", GetA);
var v1 = api.MapGroup("v1/");
v1.MapPut("/items/{id}", Handlers.Put);
v1.MapGroup("/nested").MapDelete("{id}", (int id) => id);
app.MapMethods("/opts", new[] { "OPTIONS", "head" }, () => "");
app.MapGroup("/x").MapPatch("", delegate (int a) { return a; });
app.Map("/any", () => 1);
mapper.Map<Dto>(src);
app.MapGet(SomeConst, () => 1);
api = app.MapGroup("/other");
api.MapGet("/b", () => 2);
"#;
        let got = scan(src);
        let simple: Vec<_> = got.iter().map(|(m, r, h, _)| (m.as_str(), r.as_str(), h.as_str())).collect();
        assert_eq!(
            simple,
            vec![
                ("GET", "/", "lambda"),
                ("POST", "/todos", "lambda"),
                ("GET", "/api/a", "GetA"),
                ("PUT", "/api/v1/items/{id}", "Handlers.Put"),
                ("DELETE", "/api/v1/nested/{id}", "lambda"),
                ("HEAD", "/opts", "lambda"),
                ("OPTIONS", "/opts", "lambda"),
                ("PATCH", "/x", "lambda"),
                ("ANY", "/any", "lambda"),
                ("GET", "/other/b", "lambda"),
            ]
        );
    }

    #[test]
    fn endpoints_join_route() {
        assert_eq!(join_route("", ""), "/");
        assert_eq!(join_route("/api/", "/a//b/"), "/api/a/b");
        assert_eq!(join_route("~/x", "y"), "/x/y");
        assert_eq!(replace_tokens("[Area]/[controller]/[x]", "C", "A", "Z"), "Z/C/[x]");
    }

    #[test]
    fn endpoints_shop_fixture() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/Shop/src/Shop.Api");
        let got = scan_project(&dir);
        let simple: Vec<_> = got
            .iter()
            .map(|e| {
                (
                    e.method.as_str(),
                    e.route.as_str(),
                    e.handler.as_str(),
                    e.kind,
                    e.file.file_name().unwrap().to_str().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            simple,
            vec![
                ("GET", "/Orders/sample", "OrdersController.Sample", EndpointKind::Controller, "OrdersController.cs"),
                ("POST", "/Orders/quote", "OrdersController.Post", EndpointKind::Controller, "OrdersController.cs"),
                ("GET", "/WeatherForecast", "WeatherForecastController.Get", EndpointKind::Controller, "WeatherForecastController.cs"),
                ("GET", "/health", "lambda", EndpointKind::MinimalApi, "Program.cs"),
            ]
        );
        assert_eq!(got[3].line, 21);
        assert!(got.iter().all(|e| !e.file.components().any(|c| c.as_os_str() == "obj" || c.as_os_str() == "bin")));
    }
}
