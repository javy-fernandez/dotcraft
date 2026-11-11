//! The options classes a workspace binds to configuration sections, found by a text scan of its
//! C# files (no compiler needed, works while the solution loads):
//!
//! - `services.Configure<T>(config.GetSection("X"))` (also `GetRequiredSection`),
//! - `services.AddOptions<T>().Bind(config.GetSection("X"))` / `.BindConfiguration("X")`,
//! - `config.GetSection("X").Get<T>()`,
//! - a `const string SectionName = "X"` (any constant named `*Section*`) in the class, or an
//!   attribute such as `[ConfigurationSection("X")]` on it.
//!
//! Section arguments may be literals (`"A:B"`), `nameof(T)` or constants (`T.SectionName`).
//! Properties map to keys (`[ConfigurationKeyName("x")]` renames them); a property whose type
//! is another class of the workspace, a collection or a dictionary of one leads into it.

use super::json::Seg;
use regex::Regex;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

#[derive(Clone, Debug, PartialEq)]
pub struct PropInfo {
    /// The configuration key (the property's name unless renamed).
    pub key: String,
    pub name: String,
    /// The C# type as written.
    pub ty: String,
    /// 0-based line and column of the property's name.
    pub line: usize,
    pub col: usize,
    pub doc: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ClassInfo {
    pub name: String,
    pub file: PathBuf,
    pub line: usize,
    pub col: usize,
    pub doc: Option<String>,
    pub props: Vec<PropInfo>,
    /// `const string` fields: (name, value).
    pub consts: Vec<(String, String)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EnumInfo {
    pub name: String,
    pub members: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Binding {
    /// The section's path (`"A:B"` → `["A", "B"]`).
    pub path: Vec<String>,
    pub class: String,
    pub file: PathBuf,
    pub line: usize,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct OptionsIndex {
    pub classes: Vec<ClassInfo>,
    pub enums: Vec<EnumInfo>,
    pub bindings: Vec<Binding>,
}

/// What a C# type means for configuration.
#[derive(Clone, Debug, PartialEq)]
pub enum Shape<'a> {
    Class(&'a ClassInfo),
    Enum(&'a EnumInfo),
    /// `List<T>`, `T[]`…: items by index.
    List(String),
    /// `Dictionary<string, T>`: any key.
    Map(String),
    /// A scalar (`string`, `int`, `bool`, `TimeSpan`…), by its simple name.
    Scalar(String),
}

/// A configuration path resolved through the options classes.
#[derive(Clone, Debug, PartialEq)]
pub struct Resolved<'a> {
    pub binding: &'a Binding,
    /// The property the last key is (`None`: the path is the bound section itself, or an
    /// item of a collection).
    pub prop: Option<(&'a ClassInfo, &'a PropInfo)>,
    /// What the value at the path is.
    pub shape: Shape<'a>,
}

/// `Namespace.Type<...>?` → `Type`.
fn simple(name: &str) -> &str {
    let name = name.trim().trim_end_matches('?');
    let name = name.split('<').next().unwrap_or(name);
    name.rsplit(['.', ':']).next().unwrap_or(name).trim()
}

/// The generic arguments of `T<A, B>` split at depth 0.
fn generic_args(ty: &str) -> Option<(&str, Vec<String>)> {
    let open = ty.find('<')?;
    let close = ty.rfind('>')?;
    let mut args = Vec::new();
    let (mut depth, mut cur) = (0, String::new());
    for c in ty[open + 1..close].chars() {
        match c {
            '<' => depth += 1,
            '>' => depth -= 1,
            ',' if depth == 0 => {
                args.push(cur.trim().to_string());
                cur.clear();
                continue;
            }
            _ => {}
        }
        cur.push(c);
    }
    args.push(cur.trim().to_string());
    Some((ty[..open].trim(), args))
}

impl OptionsIndex {
    pub fn class(&self, name: &str) -> Option<&ClassInfo> {
        let n = simple(name);
        self.classes.iter().find(|c| c.name == n)
    }

    pub fn enum_named(&self, name: &str) -> Option<&EnumInfo> {
        let n = simple(name);
        self.enums.iter().find(|e| e.name == n)
    }

    pub fn shape(&self, ty: &str) -> Shape<'_> {
        let ty = ty.trim().trim_end_matches('?').trim();
        if let Some(elem) = ty.strip_suffix("[]") {
            return Shape::List(elem.trim().to_string());
        }
        if let Some((outer, args)) = generic_args(ty) {
            let outer = simple(outer);
            match (outer, args.as_slice()) {
                ("List" | "IList" | "ICollection" | "IEnumerable" | "IReadOnlyList" | "IReadOnlyCollection" | "HashSet" | "ISet" | "Collection" | "ImmutableArray" | "ImmutableList", [t]) => return Shape::List(t.clone()),
                ("Dictionary" | "IDictionary" | "IReadOnlyDictionary" | "ConcurrentDictionary" | "SortedDictionary" | "ImmutableDictionary", [_, t]) => return Shape::Map(t.clone()),
                ("Nullable", [t]) => return self.shape(t),
                _ => {}
            }
        }
        if let Some(c) = self.class(ty) {
            return Shape::Class(c);
        }
        if let Some(e) = self.enum_named(ty) {
            return Shape::Enum(e);
        }
        Shape::Scalar(simple(ty).to_string())
    }

    /// Every way `path` resolves through the bound classes (several bindings may share a section).
    pub fn resolve(&self, path: &[Seg]) -> Vec<Resolved<'_>> {
        let mut out = Vec::new();
        'bindings: for b in &self.bindings {
            if path.len() < b.path.len() || !b.path.iter().zip(path).all(|(a, s)| matches!(s, Seg::Key(k) if k.eq_ignore_ascii_case(a))) {
                continue;
            }
            let Some(class) = self.class(&b.class) else { continue };
            let mut shape = Shape::Class(class);
            let mut prop = None;
            for seg in &path[b.path.len()..] {
                let next = match (&shape, seg) {
                    (Shape::Class(c), Seg::Key(k)) => match c.props.iter().find(|p| p.key.eq_ignore_ascii_case(k)) {
                        Some(p) => {
                            prop = Some((*c, p));
                            self.shape(&p.ty)
                        }
                        None => continue 'bindings,
                    },
                    (Shape::List(t), Seg::Index(_)) => {
                        prop = None;
                        self.shape(t)
                    }
                    // Configuration writes list items as keys "0", "1"…
                    (Shape::List(t), Seg::Key(k)) if k.parse::<usize>().is_ok() => {
                        prop = None;
                        self.shape(t)
                    }
                    (Shape::Map(t), Seg::Key(_)) => {
                        prop = None;
                        self.shape(t)
                    }
                    _ => continue 'bindings,
                };
                shape = next;
            }
            out.push(Resolved { binding: b, prop, shape });
        }
        out
    }
}

// ---------------------------------------------------------------------- scanning

/// Folders never scanned.
fn skipped(name: &str) -> bool {
    name.starts_with('.') || matches!(name, "bin" | "obj" | "node_modules" | "packages" | "TestResults" | "artifacts" | "target")
}

/// The C# files under `root` (sorted, at most 20 000, none over 2 MB).
pub fn cs_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let Ok(ft) = e.file_type() else { continue };
            let name = e.file_name().to_string_lossy().into_owned();
            if ft.is_dir() {
                if !skipped(&name) {
                    stack.push(e.path());
                }
            } else if name.ends_with(".cs") && e.metadata().is_ok_and(|m| m.len() < 2 << 20) {
                out.push(e.path());
                if out.len() >= 20_000 {
                    out.sort();
                    return out;
                }
            }
        }
    }
    out.sort();
    out
}

/// Scans the C# files under `root`.
pub fn scan(root: &Path) -> OptionsIndex {
    let files: Vec<(PathBuf, String)> = cs_files(root).into_iter().filter_map(|p| Some((p.clone(), std::fs::read_to_string(&p).ok()?))).collect();
    scan_sources(&files)
}

/// `text` with comments (and string contents when `strings`) blanked, offsets unchanged.
fn mask(text: &str, strings: bool) -> String {
    let b = text.as_bytes();
    let mut out = b.to_vec();
    let blank = |out: &mut Vec<u8>, from: usize, to: usize| {
        for c in &mut out[from..to.min(b.len())] {
            if *c != b'\n' {
                *c = b' ';
            }
        }
    };
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'/' if b.get(i + 1) == Some(&b'/') => {
                let end = text[i..].find('\n').map_or(b.len(), |e| i + e);
                blank(&mut out, i, end);
                i = end;
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                let end = text[i + 2..].find("*/").map_or(b.len(), |e| i + 2 + e + 2);
                blank(&mut out, i, end);
                i = end;
            }
            b'\'' => {
                // A char literal ('"' must not open a string).
                let mut j = i + 1;
                while j < b.len() && b[j] != b'\'' && b[j] != b'\n' {
                    j += if b[j] == b'\\' { 2 } else { 1 };
                }
                if strings {
                    blank(&mut out, i + 1, j);
                }
                i = j + 1;
            }
            b'"' => {
                let verbatim = i > 0 && (b[i - 1] == b'@' || (i > 1 && b[i - 1] == b'$' && b[i - 2] == b'@'));
                let raw = text[i..].starts_with("\"\"\"");
                let start = i;
                let end = if raw {
                    let q = text[i..].bytes().take_while(|&c| c == b'"').count();
                    let close = "\"".repeat(q);
                    text[i + q..].find(&close).map_or(b.len(), |e| i + q + e + q)
                } else {
                    let mut j = i + 1;
                    loop {
                        if j >= b.len() {
                            break b.len();
                        }
                        match b[j] {
                            b'\\' if !verbatim => j += 2,
                            b'"' if verbatim && b.get(j + 1) == Some(&b'"') => j += 2,
                            b'"' => break j + 1,
                            b'\n' if !verbatim => break j,
                            _ => j += 1,
                        }
                    }
                };
                if strings {
                    blank(&mut out, start + 1, end.saturating_sub(1).max(start + 1));
                }
                i = end.max(i + 1);
            }
            _ => i += 1,
        }
    }
    String::from_utf8(out).unwrap_or_default()
}

fn re(cell: &'static OnceLock<Regex>, pattern: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pattern).expect("valid regex"))
}

/// The offset after the bracket matching the one at `open` (in masked text).
fn matching(masked: &[u8], open: usize) -> Option<usize> {
    let (o, c) = match masked[open] {
        b'{' => (b'{', b'}'),
        b'(' => (b'(', b')'),
        _ => return None,
    };
    let mut depth = 0;
    for (i, &ch) in masked.iter().enumerate().skip(open) {
        if ch == o {
            depth += 1;
        } else if ch == c {
            depth -= 1;
            if depth == 0 {
                return Some(i + 1);
            }
        }
    }
    None
}

struct Lines(Vec<usize>);

impl Lines {
    fn new(text: &str) -> Lines {
        Lines(std::iter::once(0).chain(text.match_indices('\n').map(|(i, _)| i + 1)).collect())
    }

    fn at(&self, text: &str, b: usize) -> (usize, usize) {
        let line = self.0.partition_point(|&s| s <= b).saturating_sub(1);
        (line, text[self.0[line]..b].chars().count())
    }
}

/// The `///` summary right above byte `at` (tags removed, lines joined).
fn doc_above(text: &str, at: usize) -> Option<String> {
    let line_start = text[..at].rfind('\n').map_or(0, |i| i + 1);
    let mut lines = Vec::new();
    for l in text[..line_start].lines().rev() {
        let t = l.trim();
        if let Some(d) = t.strip_prefix("///") {
            lines.push(d.trim().to_string());
        } else if t.starts_with('[') || t.is_empty() && lines.is_empty() {
            continue;
        } else {
            break;
        }
    }
    lines.reverse();
    let joined = lines.join(" ");
    static TAG: OnceLock<Regex> = OnceLock::new();
    static REF: OnceLock<Regex> = OnceLock::new();
    let joined = re(&REF, r#"<(?:see|seealso|paramref|typeparamref)\s+(?:cref|name|langword)="(?:[A-Z]:)?([^"]+)"\s*/>"#).replace_all(&joined, "`$1`");
    let summary = joined.split("<summary>").nth(1).map_or(joined.as_ref(), |s| s.split("</summary>").next().unwrap_or(s));
    let clean = re(&TAG, r"<[^>]+>").replace_all(summary, "");
    let clean = clean.split_whitespace().collect::<Vec<_>>().join(" ");
    (!clean.is_empty()).then_some(clean)
}

struct Decl {
    body: std::ops::Range<usize>,
    info: ClassInfo,
}

/// Scans C# sources: classes with their properties and constants, enums, and bindings.
pub fn scan_sources(files: &[(PathBuf, String)]) -> OptionsIndex {
    static CLASS: OnceLock<Regex> = OnceLock::new();
    static PROP: OnceLock<Regex> = OnceLock::new();
    static CONST: OnceLock<Regex> = OnceLock::new();
    static ENUM: OnceLock<Regex> = OnceLock::new();
    static KEYNAME: OnceLock<Regex> = OnceLock::new();
    let class_re = re(&CLASS, r"\b(class|record|struct)\s+(?:class\s+|struct\s+)?([A-Za-z_]\w*)");
    let prop_re = re(
        &PROP,
        r"\b((?:(?:public|internal|protected|private|required|virtual|override|sealed|new|static|readonly|abstract|unsafe|extern)\s+)+)([A-Za-z_][\w.]*(?:\s*<[^;{}()=]*>)?(?:\s*\[\s*\])?\??)\s+([A-Za-z_]\w*)\s*\{([^{}]*)\}",
    );
    let const_re = re(&CONST, r#"\bconst\s+string\s+([A-Za-z_]\w*)\s*=\s*"((?:[^"\\]|\\.)*)"\s*;"#);
    let enum_re = re(&ENUM, r"\benum\s+([A-Za-z_]\w*)(?:\s*:\s*[\w.]+)?\s*\{([^}]*)\}");
    let keyname_re = re(&KEYNAME, r#"ConfigurationKeyName(?:Attribute)?\s*\(\s*"([^"]*)"\s*\)"#);

    let mut index = OptionsIndex::default();
    for (path, text) in files {
        let masked = mask(text, true);
        let mb = masked.as_bytes();
        let lines = Lines::new(text);
        let mut decls = Vec::new();
        for c in class_re.captures_iter(&masked) {
            let kind = &c[1];
            let name = c.get(2).expect("group");
            let mut i = name.end();
            let mut positional = None;
            while i < mb.len() && !matches!(mb[i], b'{' | b';' | b'(') {
                i += 1;
            }
            if i < mb.len() && mb[i] == b'(' {
                let Some(close) = matching(mb, i) else { continue };
                if kind == "record" {
                    positional = Some(i + 1..close - 1);
                }
                i = close;
                while i < mb.len() && !matches!(mb[i], b'{' | b';') {
                    i += 1;
                }
            }
            let body = if i < mb.len() && mb[i] == b'{' { i + 1..matching(mb, i).map_or(mb.len(), |e| e - 1) } else { i..i };
            let (line, col) = lines.at(text, name.start());
            let mut info = ClassInfo { name: name.as_str().to_string(), file: path.clone(), line, col, doc: doc_above(text, c.get(0).expect("match").start()), props: Vec::new(), consts: Vec::new() };
            // Positional record parameters are properties.
            if let Some(r) = positional {
                let mut depth = 0;
                let mut start = r.start;
                for j in r.start..=r.end {
                    let ch = if j < r.end { mb[j] } else { b',' };
                    match ch {
                        b'<' | b'(' | b'[' => depth += 1,
                        b'>' | b')' | b']' => depth -= 1,
                        b',' if depth == 0 => {
                            let raw = &masked[start..j];
                            let no_attr = raw.rfind(']').map_or(raw, |k| &raw[k + 1..]);
                            let decl = no_attr.split('=').next().unwrap_or("").trim();
                            if let Some((ty, pname)) = decl.rsplit_once(char::is_whitespace) {
                                let off = start + raw.find(pname).unwrap_or(0);
                                let (l, c) = lines.at(text, off);
                                let key = keyname_re.captures(&text[start..j]).map_or_else(|| pname.to_string(), |k| k[1].to_string());
                                info.props.push(PropInfo { key, name: pname.to_string(), ty: ty.trim().to_string(), line: l, col: c, doc: None });
                            }
                            start = j + 1;
                        }
                        _ => {}
                    }
                }
            }
            decls.push(Decl { body, info });
        }
        // Members belong to the innermost declaration whose body holds them, at its top level.
        let depth_at = |from: usize, to: usize| mb[from..to].iter().fold(0i32, |d, &c| d + i32::from(c == b'{') - i32::from(c == b'}'));
        let owner = |decls: &[Decl], at: usize| decls.iter().enumerate().filter(|(_, d)| d.body.start <= at && at < d.body.end).max_by_key(|(_, d)| d.body.start).map(|(k, _)| k);
        for p in prop_re.captures_iter(&masked) {
            let mods = &p[1];
            if !mods.split_whitespace().any(|m| m == "public") || mods.split_whitespace().any(|m| m == "static") || !p[4].contains("get") {
                continue;
            }
            let at = p.get(1).expect("group").start();
            let Some(k) = owner(&decls, at) else { continue };
            if depth_at(decls[k].body.start, at) != 0 {
                continue;
            }
            let name = p.get(3).expect("group");
            let (line, col) = lines.at(text, name.start());
            // Attributes between the previous member and this one.
            let prev = masked[..at].rfind([';', '}', '{']).map_or(0, |i| i + 1);
            let key = keyname_re.captures(&text[prev..at]).map_or_else(|| name.as_str().to_string(), |c| c[1].to_string());
            let ty = p[2].split_whitespace().collect::<Vec<_>>().join(" ");
            decls[k].info.props.push(PropInfo { key, name: name.as_str().to_string(), ty, line, col, doc: doc_above(text, at) });
        }
        let comments_only = mask(text, false);
        for c in const_re.captures_iter(&comments_only) {
            let at = c.get(0).expect("match").start();
            if let Some(k) = owner(&decls, at) {
                decls[k].info.consts.push((c[1].to_string(), c[2].to_string()));
            }
        }
        for e in enum_re.captures_iter(&masked) {
            let members = e[2]
                .split(',')
                .map(|m| {
                    let m = m.rfind(']').map_or(m, |k| &m[k + 1..]);
                    m.split('=').next().unwrap_or("").trim().to_string()
                })
                .filter(|m| !m.is_empty())
                .collect();
            index.enums.push(EnumInfo { name: e[1].to_string(), members });
        }
        index.classes.extend(decls.into_iter().map(|d| d.info));
    }

    // Bindings.
    static CONFIGURE: OnceLock<Regex> = OnceLock::new();
    static ADD_OPTIONS: OnceLock<Regex> = OnceLock::new();
    static SECTION_GET: OnceLock<Regex> = OnceLock::new();
    static SECTION: OnceLock<Regex> = OnceLock::new();
    static BIND_CONFIGURATION: OnceLock<Regex> = OnceLock::new();
    static ATTRIBUTE: OnceLock<Regex> = OnceLock::new();
    const ARG: &str = r#"("(?:[^"\\]|\\.)*"|nameof\s*\(\s*[\w.]+\s*\)|[A-Za-z_][\w.]*)"#;
    let configure = re(&CONFIGURE, r"\bConfigure\s*<\s*([\w.]+)\s*>\s*\(");
    let add_options = re(&ADD_OPTIONS, r"\bAddOptions\s*<\s*([\w.]+)\s*>\s*\(");
    let section_get = re(&SECTION_GET, &format!(r"\b(?:GetSection|GetRequiredSection)\s*\(\s*{ARG}\s*\)\s*\.\s*Get(?:Value)?\s*<\s*([\w.]+)\s*>"));
    let section = re(&SECTION, &format!(r"\b(?:GetSection|GetRequiredSection)\s*\(\s*{ARG}\s*\)"));
    let bind_configuration = re(&BIND_CONFIGURATION, &format!(r"\.\s*BindConfiguration\s*\(\s*{ARG}"));
    let attribute = re(&ATTRIBUTE, r#"\[\s*([\w.]*Section\w*)\s*\(\s*"([^"]+)"\s*\)\s*\]"#);

    let resolve_arg = |index: &OptionsIndex, arg: &str| -> Option<Vec<String>> {
        let arg = arg.trim();
        let value = if let Some(s) = arg.strip_prefix('"') {
            s.strip_suffix('"')?.replace("\\\\", "\\")
        } else if let Some(n) = arg.strip_prefix("nameof") {
            simple(n.trim().trim_start_matches('(').trim_end_matches(')')).to_string()
        } else if let Some((owner, name)) = arg.rsplit_once('.') {
            index.class(owner)?.consts.iter().find(|(n, _)| n == name)?.1.clone()
        } else {
            index.classes.iter().flat_map(|c| &c.consts).find(|(n, _)| n == arg)?.1.clone()
        };
        let path: Vec<String> = value.split(':').map(str::to_string).filter(|s| !s.is_empty()).collect();
        (!path.is_empty()).then_some(path)
    };
    let mut bindings = Vec::new();
    for (path, text) in files {
        let masked = mask(text, true);
        let code = mask(text, false);
        let mb = masked.as_bytes();
        let lines = Lines::new(text);
        let add = |bindings: &mut Vec<Binding>, section: Option<Vec<String>>, class: &str, at: usize| {
            if let Some(section) = section {
                bindings.push(Binding { path: section, class: simple(class).to_string(), file: path.clone(), line: lines.at(text, at).0 });
            }
        };
        for c in configure.captures_iter(&code) {
            let open = c.get(0).expect("match").end() - 1;
            let Some(close) = matching(mb, open) else { continue };
            if let Some(s) = section.captures(&code[open..close]) {
                add(&mut bindings, resolve_arg(&index, &s[1]), &c[1], open);
            }
        }
        for c in add_options.captures_iter(&code) {
            let start = c.get(0).expect("match").end();
            let end = masked[start..].find(';').map_or(masked.len(), |e| start + e);
            let stmt = &code[start..end];
            let arg = bind_configuration.captures(stmt).or_else(|| section.captures(stmt));
            if let Some(a) = arg {
                add(&mut bindings, resolve_arg(&index, &a[1]), &c[1], start);
            }
        }
        for c in section_get.captures_iter(&code) {
            add(&mut bindings, resolve_arg(&index, &c[1]), &c[2], c.get(0).expect("match").start());
        }
        for a in attribute.captures_iter(&code) {
            let end = a.get(0).expect("match").end();
            // The class this attribute decorates (only attributes and modifiers in between).
            if let Some(c) = class_re.captures(&masked[end..])
                && masked[end..end + c.get(0).expect("match").start()].chars().all(|ch| ch.is_alphanumeric() || " \t\r\n[]().,=_\"".contains(ch))
            {
                add(&mut bindings, Some(a[2].split(':').map(str::to_string).collect()), &c[2], end);
            }
        }
    }
    // Section constants in the class itself.
    for c in &index.classes {
        if let Some((_, v)) = c.consts.iter().find(|(n, _)| n.to_ascii_lowercase().contains("section")) {
            bindings.push(Binding { path: v.split(':').map(str::to_string).collect(), class: c.name.clone(), file: c.file.clone(), line: c.line });
        }
    }
    let mut seen: Vec<(Vec<String>, String)> = Vec::new();
    for b in bindings {
        let key = (b.path.iter().map(|s| s.to_ascii_lowercase()).collect(), b.class.clone());
        if !seen.contains(&key) {
            seen.push(key);
            index.bindings.push(b);
        }
    }
    index
}

#[cfg(test)]
mod tests {
    use super::*;

    const OPTIONS: &str = r#"
namespace Shop.Api.Options;

/// <summary>Outgoing mail settings.</summary>
public sealed class SmtpOptions
{
    public const string SectionName = "Smtp";

    /// <summary>The server's host name, e.g. <c>smtp.example.com</c>.</summary>
    public string Host { get; set; } = "";
    public int Port { get; set; } = 25;
    [ConfigurationKeyName("use_ssl")]
    public bool UseSsl { get; init; }
    public RetryOptions Retry { get; set; } = new();
    public List<string> Recipients { get; } = new();
    public Dictionary<string, RetryOptions> PerHost { get; set; } = new();
    public static int Ignored { get; set; }
    private string Hidden { get; set; } = "";
    public string Computed => Host + ":" + Port;
    public DeliveryMode Mode { get; set; }

    public void Send() { var x = new { Fake = 1 }; }
}

public class RetryOptions
{
    public int Count { get; set; } // "comment with { braces"
    public TimeSpan Delay { get; set; }
}

public enum DeliveryMode { Network, [Obsolete] PickupDirectory = 2, Disabled }

public record FeatureFlags(bool Beta, [property: ConfigurationKeyName("dark")] bool DarkMode, int Max = 3);

[OptionsSection("Payments:Stripe")]
public class StripeOptions { public string ApiKey { get; set; } = ""; }

public class CacheOptions { public int Minutes { get; set; } }
public class JwtSettings { public string Issuer { get; set; } = ""; }
"#;

    const PROGRAM: &str = r#"
var builder = WebApplication.CreateBuilder(args);
// builder.Services.Configure<Commented>(builder.Configuration.GetSection("Nope"));
builder.Services.Configure<SmtpOptions>(builder.Configuration.GetSection(SmtpOptions.SectionName));
builder.Services.AddOptions<CacheOptions>()
    .BindConfiguration("Cache")
    .ValidateOnStart();
builder.Services.AddOptions<FeatureFlags>().Bind(builder.Configuration.GetSection(nameof(FeatureFlags)));
var jwt = builder.Configuration.GetRequiredSection("Auth:Jwt").Get<JwtSettings>();
"#;

    fn index() -> OptionsIndex {
        scan_sources(&[("/w/Options.cs".into(), OPTIONS.into()), ("/w/Program.cs".into(), PROGRAM.into())])
    }

    fn k(s: &str) -> Seg {
        Seg::Key(s.into())
    }

    #[test]
    fn finds_classes_properties_and_docs() {
        let ix = index();
        let smtp = ix.class("SmtpOptions").unwrap();
        let keys: Vec<&str> = smtp.props.iter().map(|p| p.key.as_str()).collect();
        assert_eq!(keys, ["Host", "Port", "use_ssl", "Retry", "Recipients", "PerHost", "Mode"]);
        assert_eq!(smtp.doc.as_deref(), Some("Outgoing mail settings."));
        let host = &smtp.props[0];
        assert_eq!(host.doc.as_deref(), Some("The server's host name, e.g. smtp.example.com."));
        assert_eq!((host.line, host.col), (9, 18));
        assert_eq!(smtp.consts, [("SectionName".to_string(), "Smtp".to_string())]);
        assert_eq!(ix.class("RetryOptions").unwrap().props.len(), 2);
        let flags = ix.class("FeatureFlags").unwrap();
        assert_eq!(flags.props.iter().map(|p| (p.key.as_str(), p.ty.as_str())).collect::<Vec<_>>(), [("Beta", "bool"), ("dark", "bool"), ("Max", "int")]);
        assert_eq!(ix.enum_named("DeliveryMode").unwrap().members, ["Network", "PickupDirectory", "Disabled"]);
    }

    #[test]
    fn finds_bindings_of_every_style() {
        let ix = index();
        let mut found: Vec<(String, String)> = ix.bindings.iter().map(|b| (b.path.join(":"), b.class.clone())).collect();
        found.sort();
        assert_eq!(
            found,
            [
                ("Auth:Jwt".to_string(), "JwtSettings".to_string()),
                ("Cache".into(), "CacheOptions".into()),
                ("FeatureFlags".into(), "FeatureFlags".into()),
                ("Payments:Stripe".into(), "StripeOptions".into()),
                ("Smtp".into(), "SmtpOptions".into()),
            ]
        );
        let smtp = ix.bindings.iter().find(|b| b.class == "SmtpOptions").unwrap();
        assert_eq!((smtp.file.to_str().unwrap(), smtp.line), ("/w/Program.cs", 3));
    }

    #[test]
    fn paths_resolve_through_properties_lists_and_maps() {
        let ix = index();
        let r = ix.resolve(&[k("smtp")]);
        assert!(matches!(&r[..], [Resolved { prop: None, shape: Shape::Class(c), .. }] if c.name == "SmtpOptions"));
        let r = ix.resolve(&[k("Smtp"), k("Retry"), k("Count")]);
        assert!(matches!(&r[..], [Resolved { prop: Some((c, p)), shape: Shape::Scalar(t), .. }] if c.name == "RetryOptions" && p.name == "Count" && t == "int"));
        let r = ix.resolve(&[k("Smtp"), k("PerHost"), k("mail.example.com"), k("Delay")]);
        assert!(matches!(&r[..], [Resolved { shape: Shape::Scalar(t), .. }] if t == "TimeSpan"));
        let r = ix.resolve(&[k("Smtp"), k("Recipients"), Seg::Index(0)]);
        assert!(matches!(&r[..], [Resolved { shape: Shape::Scalar(t), .. }] if t == "string"));
        let r = ix.resolve(&[k("Smtp"), k("Mode")]);
        assert!(matches!(&r[..], [Resolved { shape: Shape::Enum(e), .. }] if e.name == "DeliveryMode"));
        let r = ix.resolve(&[k("Payments"), k("Stripe"), k("ApiKey")]);
        assert_eq!(r.len(), 1);
        assert!(ix.resolve(&[k("Smtp"), k("Unknown")]).is_empty());
        assert!(ix.resolve(&[k("Payments")]).is_empty());
    }

    #[test]
    fn scans_a_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        std::fs::create_dir_all(root.join("src/obj")).unwrap();
        std::fs::write(root.join("src/Options.cs"), OPTIONS).unwrap();
        std::fs::write(root.join("src/Program.cs"), PROGRAM).unwrap();
        std::fs::write(root.join("src/obj/Generated.cs"), "public class Generated { public const string SectionName = \"Gen\"; }").unwrap();
        let ix = scan(&root);
        assert!(ix.class("Generated").is_none());
        assert_eq!(ix.bindings.iter().find(|b| b.class == "SmtpOptions").unwrap().file, root.join("src/Program.cs"));
    }
}
