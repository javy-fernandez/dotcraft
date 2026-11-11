//! Code insight for `appsettings*.json`: completion of keys and values from the bundled JSON
//! schema and from the options classes bound to configuration sections, documentation for
//! hovers, problems (wrong types, unknown enum values) and the declaration a key binds to.
//!
//! Everything here is plain data on byte offsets; the app formats and localizes it.

pub mod json;
pub mod options;
pub mod schema;

use json::{Kind, Node, Seg, Spot};
use options::{OptionsIndex, Shape};
use schema::{Doc, Problem, Schema};
use std::ops::Range;
use std::path::{Path, PathBuf};

/// `appsettings.json`, `appsettings.Development.json`…
pub fn is_appsettings(path: &Path) -> bool {
    path.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).is_some_and(|n| n.starts_with("appsettings") && n.ends_with(".json"))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemKind {
    /// A key the schema names.
    Key,
    /// A property of a bound options class.
    Property,
    /// An enum value.
    Enum,
    /// `true`, `false`, a default value.
    Literal,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub label: String,
    /// Snippet inserted (`$0` marks the caret).
    pub insert: String,
    /// Greyed after the label (the options class).
    pub detail: String,
    /// Right-aligned (the type).
    pub type_text: String,
    pub doc: Option<String>,
    pub kind: ItemKind,
    /// The schema's default value.
    pub is_default: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Completions {
    /// Byte where the typed text starts (after an opening quote).
    pub anchor: usize,
    /// Bytes an accepted item replaces.
    pub range: Range<usize>,
    pub items: Vec<Item>,
}

/// A property of an options class a key binds to.
#[derive(Clone, Debug, PartialEq)]
pub struct Bound {
    pub class: String,
    pub prop: Option<String>,
    pub ty: Option<String>,
    pub doc: Option<String>,
    /// Declaration: file, 0-based line and column.
    pub file: PathBuf,
    pub line: usize,
    pub col: usize,
    /// Where the section is bound (`Configure<T>`…), for the class itself.
    pub binding: Option<(PathBuf, usize)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct KeyInfo {
    /// The configuration path (`Logging:LogLevel:Default`).
    pub path: String,
    /// The key's bytes (with quotes).
    pub range: Range<usize>,
    pub schema: Option<Doc>,
    pub bound: Vec<Bound>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Finding {
    pub range: Range<usize>,
    pub problem: Problem,
}

/// The type an options property expects, as a JSON type (`None`: anything goes).
fn expected_type(shape: &Shape<'_>) -> Option<&'static str> {
    match shape {
        Shape::Class(_) | Shape::Map(_) => Some("object"),
        Shape::List(_) => Some("array"),
        Shape::Enum(_) => Some("string"),
        Shape::Scalar(t) => match t.as_str() {
            "bool" | "Boolean" => Some("boolean"),
            "int" | "long" | "short" | "byte" | "uint" | "ulong" | "ushort" | "sbyte" | "Int32" | "Int64" | "Int16" | "UInt32" | "UInt64" | "Byte" => Some("integer"),
            "double" | "float" | "decimal" | "Double" | "Single" | "Decimal" => Some("number"),
            _ => None,
        },
    }
}

/// Whether a value fits an options property's type the way the binder converts it.
fn check_shape(shape: &Shape<'_>, n: &Node) -> Option<Problem> {
    if matches!(n.kind, Kind::Null) {
        return None;
    }
    if let Shape::Enum(e) = shape {
        let text = match &n.kind {
            Kind::String(s) => s.trim().to_string(),
            Kind::Number(s) => return (!s.parse::<i64>().is_ok()).then(|| Problem::Enum { allowed: e.members.clone() }),
            _ => return Some(Problem::Type { expected: vec!["string".into()], found: schema::type_name(n) }),
        };
        // Flags enums take "A, B".
        let ok = text.parse::<i64>().is_ok() || text.split(',').all(|p| e.members.iter().any(|m| m.eq_ignore_ascii_case(p.trim())));
        return (!ok).then(|| Problem::Enum { allowed: e.members.clone() });
    }
    let t = expected_type(shape)?;
    // Collections also bind from objects with "0", "1"… keys.
    let ok = schema::fits_type(n, t) || t == "array" && matches!(n.kind, Kind::Object(_));
    (!ok).then(|| Problem::Type { expected: vec![t.to_string()], found: schema::type_name(n) })
}

fn json_type_of_shape(shape: &Shape<'_>) -> Option<&'static str> {
    match shape {
        // Strings, TimeSpan, Uri…: written as strings.
        Shape::Scalar(_) => expected_type(shape).or(Some("string")),
        _ => expected_type(shape),
    }
}

/// `"Name": …` with the caret where the value goes.
fn key_snippet(name: &str, ty: Option<&str>) -> String {
    let name = name.replace('\\', "\\\\").replace('"', "\\\"").replace('$', "\\$");
    match ty {
        Some("object") => format!("\"{name}\": {{\n\t$0\n}}"),
        Some("array") => format!("\"{name}\": [$0]"),
        Some("string") => format!("\"{name}\": \"$0\""),
        _ => format!("\"{name}\": $0"),
    }
}

fn literal(v: &serde_json::Value) -> String {
    let s = v.to_string();
    s.replace('$', "\\$")
}

/// Completion at byte `b`.
pub fn complete(text: &str, b: usize, schema: &Schema, opts: &OptionsIndex) -> Option<Completions> {
    let root = json::parse(text)?;
    match json::spot_at(&root, text, b)? {
        Spot::Key { path, range, has_colon, siblings, .. } => {
            let mut items: Vec<Item> = Vec::new();
            let taken = |name: &str, items: &[Item]| siblings.iter().any(|s| s.eq_ignore_ascii_case(name)) || items.iter().any(|i| i.label.eq_ignore_ascii_case(name));
            // A key goes before another member: separate them.
            let rest = text[range.end..].trim_start();
            let comma = if !has_colon && rest.starts_with('"') { "," } else { "" };
            for r in opts.resolve(&path) {
                let Shape::Class(c) = r.shape else { continue };
                for p in &c.props {
                    if taken(&p.key, &items) {
                        continue;
                    }
                    let ty = json_type_of_shape(&opts.shape(&p.ty));
                    let insert = if has_colon { format!("\"{}\"", p.key) } else { key_snippet(&p.key, ty) + comma };
                    items.push(Item { label: p.key.clone(), insert, detail: c.name.clone(), type_text: p.ty.clone(), doc: p.doc.clone(), kind: ItemKind::Property, is_default: false });
                }
            }
            for k in schema.keys(&path) {
                if taken(&k.name, &items) {
                    continue;
                }
                let ty = if k.doc.types.len() == 1 { k.doc.types.first().map(String::as_str) } else if !k.doc.enums.is_empty() { Some("string") } else { None };
                let insert = if has_colon { format!("\"{}\"", k.name) } else { key_snippet(&k.name, ty) + comma };
                items.push(Item { label: k.name.clone(), insert, detail: String::new(), type_text: k.doc.types.join(" | "), doc: k.doc.description.clone(), kind: ItemKind::Key, is_default: false });
            }
            let anchor = if text[range.clone()].starts_with('"') { range.start + 1 } else { range.start };
            (!items.is_empty()).then_some(Completions { anchor, range, items })
        }
        Spot::Value { path, range, quoted, .. } => {
            let mut items: Vec<Item> = Vec::new();
            let push = |items: &mut Vec<Item>, label: String, insert: String, kind: ItemKind, doc: Option<String>, is_default: bool| {
                if let Some(i) = items.iter_mut().find(|i| i.label == label) {
                    i.is_default |= is_default;
                } else {
                    items.push(Item { label, insert, detail: String::new(), type_text: String::new(), doc, kind, is_default });
                }
            };
            for r in opts.resolve(&path) {
                match &r.shape {
                    Shape::Enum(e) => {
                        for m in &e.members {
                            push(&mut items, m.clone(), format!("\"{m}\""), ItemKind::Enum, None, false);
                        }
                    }
                    Shape::Scalar(t) if expected_type(&r.shape) == Some("boolean") || t == "bool" => {
                        for v in ["true", "false"] {
                            push(&mut items, v.into(), v.into(), ItemKind::Literal, None, false);
                        }
                    }
                    _ => {}
                }
            }
            let doc = schema.describe(&schema.at(&path));
            for e in &doc.enums {
                push(&mut items, schema::value_text(e), literal(e), ItemKind::Enum, None, doc.default.as_ref() == Some(e));
            }
            if doc.types.iter().any(|t| t == "boolean") {
                for v in [true, false] {
                    push(&mut items, v.to_string(), v.to_string(), ItemKind::Literal, None, doc.default == Some(serde_json::Value::Bool(v)));
                }
            }
            if let Some(d) = doc.default.as_ref().filter(|d| !d.is_object() && !d.is_array()) {
                push(&mut items, schema::value_text(d), literal(d), ItemKind::Literal, None, true);
            }
            // Inside quotes only strings make sense.
            if quoted {
                items.retain(|i| i.insert.starts_with('"'));
            }
            let anchor = if quoted { range.start + 1 } else { range.start };
            (!items.is_empty()).then_some(Completions { anchor, range, items })
        }
    }
}

/// Problems of the whole file.
pub fn validate(text: &str, schema: &Schema, opts: &OptionsIndex) -> Vec<Finding> {
    let mut out = Vec::new();
    let Some(root) = json::parse(text) else { return out };
    fn go(n: &Node, path: &mut Vec<Seg>, schema: &Schema, opts: &OptionsIndex, out: &mut Vec<Finding>) {
        if !path.is_empty() {
            if let Kind::Invalid(w) = &n.kind {
                out.push(Finding { range: n.range.clone(), problem: Problem::Bare(w.clone()) });
                return;
            }
            let nodes = schema.at(path);
            let mut problem = if nodes.is_empty() { None } else { schema.check(&nodes, n) };
            if problem.is_none() {
                problem = opts.resolve(path).iter().filter(|r| r.prop.is_some() || !matches!(path.last(), Some(Seg::Key(_)))).find_map(|r| check_shape(&r.shape, n));
            }
            if let Some(problem) = problem {
                out.push(Finding { range: n.range.clone(), problem });
                return;
            }
        }
        match &n.kind {
            Kind::Object(members) => {
                for m in members {
                    if let Some(v) = &m.value {
                        path.push(Seg::Key(m.key.clone()));
                        go(v, path, schema, opts, out);
                        path.pop();
                    }
                }
            }
            Kind::Array(items) => {
                for (i, v) in items.iter().enumerate() {
                    path.push(Seg::Index(i));
                    go(v, path, schema, opts, out);
                    path.pop();
                }
            }
            _ => {}
        }
    }
    go(&root, &mut Vec::new(), schema, opts, &mut out);
    out
}

/// The key under byte `b` (on its name or its value): documentation and bound properties.
pub fn key_at(text: &str, b: usize, schema: &Schema, opts: &OptionsIndex) -> Option<KeyInfo> {
    let root = json::parse(text)?;
    let (path, m) = json::member_at(&root, b)?;
    let nodes = schema.at(&path);
    let doc = (!nodes.is_empty()).then(|| schema.describe(&nodes));
    let mut bound = Vec::new();
    for r in opts.resolve(&path) {
        let b = match r.prop {
            Some((c, p)) => Bound { class: c.name.clone(), prop: Some(p.name.clone()), ty: Some(p.ty.clone()), doc: p.doc.clone(), file: c.file.clone(), line: p.line, col: p.col, binding: None },
            None => match r.shape {
                Shape::Class(c) if r.binding.path.len() == path.len() => {
                    Bound { class: c.name.clone(), prop: None, ty: None, doc: c.doc.clone(), file: c.file.clone(), line: c.line, col: c.col, binding: Some((r.binding.file.clone(), r.binding.line)) }
                }
                _ => continue,
            },
        };
        if !bound.contains(&b) {
            bound.push(b);
        }
    }
    Some(KeyInfo { path: json::path_string(&path), range: m.key_range.clone(), schema: doc, bound })
}

#[cfg(test)]
mod tests {
    use super::*;

    const OPTIONS: &str = r#"
public class SmtpOptions
{
    public const string SectionName = "Smtp";
    /// <summary>Host name.</summary>
    public string Host { get; set; } = "";
    public int Port { get; set; }
    public bool UseSsl { get; set; }
    public Mode Mode { get; set; }
}
public enum Mode { Network, Pickup }
"#;

    fn opts() -> OptionsIndex {
        options::scan_sources(&[("/w/SmtpOptions.cs".into(), OPTIONS.into())])
    }

    fn at(src: &str) -> (String, usize) {
        let b = src.find('|').unwrap();
        (src.replacen('|', "", 1), b)
    }

    fn labels(c: &Completions) -> Vec<&str> {
        c.items.iter().map(|i| i.label.as_str()).collect()
    }

    #[test]
    fn completes_schema_keys_with_snippets() {
        let (text, b) = at("{\n  \"Logging\": {},\n  \"|\"\n}");
        let c = complete(&text, b, Schema::appsettings(), &OptionsIndex::default()).unwrap();
        let l = labels(&c);
        assert!(l.contains(&"Kestrel") && l.contains(&"AllowedHosts") && !l.contains(&"Logging"), "{l:?}");
        assert_eq!((c.anchor, &text[c.range.clone()]), (b, "\"\""));
        let kestrel = c.items.iter().find(|i| i.label == "Kestrel").unwrap();
        assert_eq!(kestrel.insert, "\"Kestrel\": {\n\t$0\n}");
        assert!(kestrel.doc.as_deref().unwrap().contains("Kestrel"));
        assert_eq!(c.items.iter().find(|i| i.label == "AllowedHosts").unwrap().insert, "\"AllowedHosts\": \"$0\"");
        // Before another member a comma follows.
        let (text, b) = at("{ \"Logging\": { \"Lo|\" \"Console\": {} } }");
        let c = complete(&text, b, Schema::appsettings(), &OptionsIndex::default()).unwrap();
        assert_eq!(c.items.iter().find(|i| i.label == "LogLevel").unwrap().insert, "\"LogLevel\": {\n\t$0\n},");
    }

    #[test]
    fn completes_enum_values() {
        let (text, b) = at("{ \"Logging\": { \"LogLevel\": { \"Default\": \"|\" } } }");
        let c = complete(&text, b, Schema::appsettings(), &OptionsIndex::default()).unwrap();
        assert_eq!(labels(&c), ["Trace", "Debug", "Information", "Warning", "Error", "Critical", "None"]);
        assert_eq!(c.items[2].insert, "\"Information\"");
        let (text, b) = at("{ \"Kestrel\": { \"Endpoints\": { \"Http\": { \"Certificate\": { \"AllowInvalid\": | } } } } }");
        let c = complete(&text, b, Schema::appsettings(), &OptionsIndex::default()).unwrap();
        assert_eq!(labels(&c), ["true", "false"]);
        assert!(c.items[1].is_default);
    }

    #[test]
    fn completes_options_properties_and_values() {
        let (text, b) = at("{ \"Smtp\": { \"Host\": \"x\", | } }");
        let c = complete(&text, b, Schema::appsettings(), &opts()).unwrap();
        assert_eq!(labels(&c), ["Port", "UseSsl", "Mode"]);
        assert_eq!(c.items[0].insert, "\"Port\": $0");
        assert_eq!(c.items[0].detail, "SmtpOptions");
        let (text, b) = at("{ \"Smtp\": { \"Mode\": \"|\" } }");
        let c = complete(&text, b, Schema::appsettings(), &opts()).unwrap();
        assert_eq!(labels(&c), ["Network", "Pickup"]);
        let (text, b) = at("{ \"Smtp\": { \"Ho|\": \"x\" } }");
        let c = complete(&text, b, Schema::appsettings(), &opts()).unwrap();
        assert_eq!(c.items.iter().find(|i| i.label == "Host").unwrap().insert, "\"Host\"");
    }

    #[test]
    fn finds_wrong_types_and_values() {
        let text = r#"{
  "Logging": { "LogLevel": { "Default": "Info", "Microsoft": "warning" } },
  "AllowedHosts": ["*"],
  "Kestrel": { "Endpoints": { "Http": { "Url": "http://*:5000", "Protocols": "Http4" } } },
  "Smtp": { "Port": "25", "UseSsl": "maybe", "Mode": "Network", "Host": Warning },
  "MyOwnSection": { "Anything": [1, 2] }
}"#;
        let found = validate(text, Schema::appsettings(), &opts());
        let got: Vec<(&str, &Problem)> = found.iter().map(|f| (&text[f.range.clone()], &f.problem)).collect();
        assert_eq!(got.len(), 5, "{got:#?}");
        assert!(matches!(got[0], ("\"Info\"", Problem::Enum { .. })));
        assert!(matches!(got[1], ("[\"*\"]", Problem::Type { expected, found: "array" }) if expected == &["string"]));
        assert!(matches!(got[2], ("\"Http4\"", Problem::Enum { .. })));
        assert!(matches!(got[3], ("\"maybe\"", Problem::Type { expected, .. }) if expected == &["boolean"]));
        assert!(matches!(got[4], ("Warning", Problem::Bare(_))));
    }

    #[test]
    fn keys_lead_to_their_options() {
        let text = "{ \"Smtp\": { \"Host\": \"x\" }, \"Logging\": {} }";
        let k = key_at(text, text.find("Host").unwrap(), Schema::appsettings(), &opts()).unwrap();
        assert_eq!(k.path, "Smtp:Host");
        assert_eq!(k.bound.len(), 1);
        let b = &k.bound[0];
        assert_eq!((b.class.as_str(), b.prop.as_deref(), b.line, b.col, b.doc.as_deref()), ("SmtpOptions", Some("Host"), 5, 18, Some("Host name.")));
        let k = key_at(text, text.find("Smtp").unwrap(), Schema::appsettings(), &opts()).unwrap();
        assert!(matches!(&k.bound[..], [Bound { prop: None, line: 1, .. }]));
        let k = key_at(text, text.find("Logging").unwrap(), Schema::appsettings(), &opts()).unwrap();
        assert!(k.bound.is_empty() && k.schema.unwrap().description.is_some());
        assert!(is_appsettings(Path::new("/a/appsettings.Development.json")) && !is_appsettings(Path::new("/a/settings.json")));
    }
}
