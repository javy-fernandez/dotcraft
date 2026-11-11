//! The JSON schema of `appsettings*.json` (SchemaStore's, bundled so it works offline): what can
//! go at a configuration path, the documentation of a key, and the type and allowed values of a
//! value. Only the parts of draft-07 that schema uses: `$ref` (local pointers), `properties`,
//! `patternProperties`, `additionalProperties`, `items`, `allOf`/`anyOf`/`oneOf`, `type`, `enum`,
//! `description`/`title`/`default`.
//!
//! Configuration keys are case-insensitive and the configuration binder converts strings, so a
//! value is accepted when the binder would accept it (`"true"` for a boolean, any letter case
//! for an enum value).

use super::json::{Kind, Node, Seg};
use regex::Regex;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// SchemaStore's `appsettings.json` schema (Apache-2.0, see `assets/schemas/NOTICE.md`).
pub const APPSETTINGS_SCHEMA: &str = include_str!("../../../../assets/schemas/appsettings.json");

pub struct Schema {
    root: Value,
}

/// What the schema says about a key or a value.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Doc {
    pub description: Option<String>,
    /// JSON types (`string`, `object`…), in the schema's order.
    pub types: Vec<String>,
    pub enums: Vec<Value>,
    pub default: Option<Value>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct KeyDoc {
    pub name: String,
    pub doc: Doc,
}

/// Why a value does not fit its schema.
#[derive(Clone, Debug, PartialEq)]
pub enum Problem {
    Type { expected: Vec<String>, found: &'static str },
    Enum { allowed: Vec<String> },
    /// A bare word where a value goes (a string without quotes).
    Bare(String),
}

/// The JSON type of a value, as schemas name them.
pub fn type_name(n: &Node) -> &'static str {
    match &n.kind {
        Kind::Object(_) => "object",
        Kind::Array(_) => "array",
        Kind::String(_) => "string",
        Kind::Number(_) => "number",
        Kind::Bool(_) => "boolean",
        Kind::Null => "null",
        Kind::Invalid(_) => "invalid",
    }
}

/// `n` is of JSON type `t`, or a string the configuration binder converts to it.
pub fn fits_type(n: &Node, t: &str) -> bool {
    let s = match &n.kind {
        Kind::String(s) => Some(s.trim()),
        _ => None,
    };
    match t {
        "object" => matches!(n.kind, Kind::Object(_)),
        "array" => matches!(n.kind, Kind::Array(_)),
        "string" => matches!(n.kind, Kind::String(_) | Kind::Number(_) | Kind::Bool(_)),
        "boolean" => matches!(n.kind, Kind::Bool(_)) || s.is_some_and(|s| s.eq_ignore_ascii_case("true") || s.eq_ignore_ascii_case("false")),
        "integer" => matches!(&n.kind, Kind::Number(x) if !x.contains(['.', 'e', 'E'])) || s.is_some_and(|s| s.parse::<i64>().is_ok()),
        "number" => matches!(n.kind, Kind::Number(_)) || s.is_some_and(|s| s.parse::<f64>().is_ok()),
        "null" => matches!(n.kind, Kind::Null),
        _ => true,
    }
}

/// A scalar as text (enum comparisons).
fn scalar_text(n: &Node) -> Option<String> {
    match &n.kind {
        Kind::String(s) => Some(s.clone()),
        Kind::Number(s) => Some(s.clone()),
        Kind::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// An enum value as written in the schema (without quotes for strings).
pub fn value_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn regex(pattern: &str) -> Option<Regex> {
    static CACHE: OnceLock<Mutex<HashMap<String, Option<Regex>>>> = OnceLock::new();
    let mut cache = CACHE.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    cache.entry(pattern.to_string()).or_insert_with(|| Regex::new(pattern).ok()).clone()
}

/// The name a simple `patternProperties` pattern stands for (`^(nlog|Nlog|NLog)$` → `NLog`).
fn pattern_name(pattern: &str) -> Option<String> {
    let inner = pattern.strip_prefix('^')?.strip_suffix('$')?;
    let inner = inner.strip_prefix('(').and_then(|s| s.strip_suffix(')')).unwrap_or(inner);
    let alts: Vec<&str> = inner.split('|').collect();
    if alts.iter().any(|a| a.is_empty() || !a.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')) {
        return None;
    }
    // Prefer the PascalCase spelling .NET configuration uses.
    let best = alts.iter().filter(|a| a.starts_with(|c: char| c.is_ascii_uppercase())).max_by_key(|a| a.chars().filter(|c| c.is_ascii_uppercase()).count()).or(alts.first())?;
    Some(best.to_string())
}

impl Schema {
    pub fn parse(text: &str) -> anyhow::Result<Schema> {
        Ok(Schema { root: serde_json::from_str(text)? })
    }

    /// The bundled `appsettings.json` schema.
    pub fn appsettings() -> &'static Schema {
        static SCHEMA: OnceLock<Schema> = OnceLock::new();
        SCHEMA.get_or_init(|| Schema::parse(APPSETTINGS_SCHEMA).expect("bundled schema is valid JSON"))
    }

    fn deref(&self, v: &Value) -> Option<&Value> {
        let r = v.get("$ref")?.as_str()?;
        let pointer = r.strip_prefix('#')?;
        self.root.pointer(pointer)
    }

    /// `v` and everything it stands for: its `$ref` target and the members of its
    /// `allOf`/`anyOf`/`oneOf` (all of them: completion offers every alternative).
    fn flatten<'a>(&'a self, v: &'a Value, out: &mut Vec<&'a Value>, depth: usize) {
        if depth > 32 || !v.is_object() || out.iter().any(|o| std::ptr::eq(*o, v)) {
            return;
        }
        out.push(v);
        if let Some(t) = self.deref(v) {
            self.flatten(t, out, depth + 1);
        }
        for k in ["allOf", "anyOf", "oneOf"] {
            for s in v.get(k).and_then(Value::as_array).into_iter().flatten() {
                self.flatten(s, out, depth + 1);
            }
        }
    }

    fn flat<'a>(&'a self, nodes: &[&'a Value]) -> Vec<&'a Value> {
        let mut out = Vec::new();
        for n in nodes {
            self.flatten(n, &mut out, 0);
        }
        out
    }

    /// The schemas of the value at `path` (empty: the schema knows nothing about it).
    pub fn at(&self, path: &[Seg]) -> Vec<&Value> {
        let mut nodes = vec![&self.root];
        for seg in path {
            nodes = self.child(&nodes, seg);
            if nodes.is_empty() {
                break;
            }
        }
        nodes
    }

    /// The schemas of `seg` inside a value described by `nodes`.
    pub fn child<'a>(&'a self, nodes: &[&'a Value], seg: &Seg) -> Vec<&'a Value> {
        let mut out = Vec::new();
        for s in self.flat(nodes) {
            match seg {
                Seg::Key(k) => {
                    let props = s.get("properties").and_then(Value::as_object);
                    let exact = props.and_then(|p| p.get(k.as_str()).or_else(|| p.iter().find(|(n, _)| n.eq_ignore_ascii_case(k)).map(|(_, v)| v)));
                    if let Some(v) = exact {
                        out.push(v);
                        continue;
                    }
                    let mut matched = false;
                    for (pat, v) in s.get("patternProperties").and_then(Value::as_object).into_iter().flatten() {
                        if regex(pat).is_some_and(|r| r.is_match(k)) {
                            out.push(v);
                            matched = true;
                        }
                    }
                    if !matched && let Some(v) = s.get("additionalProperties").filter(|v| v.is_object()) {
                        out.push(v);
                    }
                }
                Seg::Index(i) => match s.get("items") {
                    Some(v @ Value::Object(_)) => out.push(v),
                    Some(Value::Array(list)) => out.extend(list.get(*i)),
                    _ => {}
                },
            }
        }
        out
    }

    /// The keys the schema names for the object at `path`, with their documentation.
    pub fn keys(&self, path: &[Seg]) -> Vec<KeyDoc> {
        let nodes = self.at(path);
        let mut out: Vec<KeyDoc> = Vec::new();
        let push = |name: String, v: &Value, out: &mut Vec<KeyDoc>| {
            if !out.iter().any(|k| k.name.eq_ignore_ascii_case(&name)) {
                out.push(KeyDoc { name, doc: self.describe(&[v]) });
            }
        };
        for s in self.flat(&nodes) {
            for (name, v) in s.get("properties").and_then(Value::as_object).into_iter().flatten() {
                push(name.clone(), v, &mut out);
            }
            for (pat, v) in s.get("patternProperties").and_then(Value::as_object).into_iter().flatten() {
                if let Some(name) = pattern_name(pat) {
                    push(name, v, &mut out);
                }
            }
        }
        out
    }

    /// Documentation, types and allowed values of the schemas `nodes`.
    pub fn describe(&self, nodes: &[&Value]) -> Doc {
        let mut doc = Doc::default();
        for s in self.flat(nodes) {
            if doc.description.is_none() {
                doc.description = s.get("description").or_else(|| s.get("title")).and_then(Value::as_str).map(str::trim).filter(|d| !d.is_empty()).map(str::to_string);
            }
            match s.get("type") {
                Some(Value::String(t)) => doc.types.push(t.clone()),
                Some(Value::Array(ts)) => doc.types.extend(ts.iter().filter_map(Value::as_str).map(str::to_string)),
                _ => {}
            }
            for e in s.get("enum").and_then(Value::as_array).into_iter().flatten() {
                if !doc.enums.contains(e) {
                    doc.enums.push(e.clone());
                }
            }
            if doc.default.is_none() {
                doc.default = s.get("default").cloned();
            }
        }
        let mut seen = Vec::new();
        doc.types.retain(|t| if seen.contains(t) { false } else { seen.push(t.clone()); true });
        doc
    }

    /// Whether `n` fits one of the schemas `nodes` (any of them, like alternatives).
    pub fn check(&self, nodes: &[&Value], n: &Node) -> Option<Problem> {
        let mut problems = Vec::new();
        for s in nodes {
            problems.push(self.check_one(s, n, 0)?);
        }
        merge(problems)
    }

    fn check_one(&self, s: &Value, n: &Node, depth: usize) -> Option<Problem> {
        if depth > 32 || !s.is_object() || matches!(n.kind, Kind::Null) {
            return None;
        }
        if let Some(t) = self.deref(s)
            && let Some(p) = self.check_one(t, n, depth + 1)
        {
            return Some(p);
        }
        for a in s.get("allOf").and_then(Value::as_array).into_iter().flatten() {
            if let Some(p) = self.check_one(a, n, depth + 1) {
                return Some(p);
            }
        }
        for k in ["anyOf", "oneOf"] {
            if let Some(alts) = s.get(k).and_then(Value::as_array).filter(|a| !a.is_empty()) {
                let mut problems = Vec::new();
                for a in alts {
                    match self.check_one(a, n, depth + 1) {
                        None => {
                            problems.clear();
                            break;
                        }
                        Some(p) => problems.push(p),
                    }
                }
                if let Some(p) = merge(problems) {
                    return Some(p);
                }
            }
        }
        let types: Vec<String> = match s.get("type") {
            Some(Value::String(t)) => vec![t.clone()],
            Some(Value::Array(ts)) => ts.iter().filter_map(Value::as_str).map(str::to_string).collect(),
            _ => Vec::new(),
        };
        if !types.is_empty() && !types.iter().any(|t| fits_type(n, t)) {
            return Some(Problem::Type { expected: types, found: type_name(n) });
        }
        if let Some(list) = s.get("enum").and_then(Value::as_array)
            && let Some(text) = scalar_text(n)
            && !list.iter().any(|e| value_text(e).eq_ignore_ascii_case(text.trim()))
        {
            return Some(Problem::Enum { allowed: list.iter().map(value_text).collect() });
        }
        None
    }
}

/// One problem for failed alternatives: allowed values win (more precise), types are joined.
fn merge(problems: Vec<Problem>) -> Option<Problem> {
    if let Some(p) = problems.iter().find(|p| matches!(p, Problem::Enum { .. })) {
        return Some(p.clone());
    }
    let mut expected: Vec<String> = Vec::new();
    let mut found = "";
    for p in &problems {
        if let Problem::Type { expected: e, found: f } = p {
            found = f;
            for t in e {
                if !expected.contains(t) {
                    expected.push(t.clone());
                }
            }
        }
    }
    (!problems.is_empty()).then_some(Problem::Type { expected, found })
}

#[cfg(test)]
mod tests {
    use super::super::json::{self, Seg};
    use super::*;

    fn k(s: &str) -> Seg {
        Seg::Key(s.into())
    }

    fn names(path: &[Seg]) -> Vec<String> {
        Schema::appsettings().keys(path).into_iter().map(|k| k.name).collect()
    }

    fn check(path: &[Seg], value: &str) -> Option<Problem> {
        let s = Schema::appsettings();
        s.check(&s.at(path), &json::parse(value).unwrap())
    }

    #[test]
    fn keys_at_the_root_and_inside_sections() {
        let root = names(&[]);
        for key in ["Kestrel", "Logging", "AllowedHosts", "ConnectionStrings", "Serilog", "NLog"] {
            assert!(root.contains(&key.to_string()), "{key} in {root:?}");
        }
        assert!(names(&[k("Logging")]).contains(&"LogLevel".to_string()));
        // Case-insensitive keys, `$ref`s and `additionalProperties` (named endpoints).
        assert!(names(&[k("kestrel"), k("Endpoints"), k("Https")]).contains(&"Certificate".to_string()));
        assert!(names(&[k("Kestrel"), k("Endpoints"), k("Https"), k("Certificate")]).contains(&"AllowInvalid".to_string()));
        // A provider's own section (additionalProperties of Logging).
        assert_eq!(names(&[k("Logging"), k("Seq")]), ["LogLevel"]);
    }

    #[test]
    fn documentation_and_values() {
        let s = Schema::appsettings();
        let doc = s.describe(&s.at(&[k("Logging"), k("LogLevel"), k("Default")]));
        assert_eq!(doc.types, ["string"]);
        assert_eq!(doc.enums.iter().map(value_text).collect::<Vec<_>>(), ["Trace", "Debug", "Information", "Warning", "Error", "Critical", "None"]);
        assert!(s.describe(&s.at(&[k("AllowedHosts")])).description.unwrap().contains("host filtering"));
        let protocols = s.describe(&s.at(&[k("Kestrel"), k("EndpointDefaults"), k("Protocols")]));
        assert!(protocols.enums.contains(&Value::String("Http1AndHttp2".into())));
        // Array items.
        let ssl = s.describe(&s.at(&[k("Kestrel"), k("EndpointDefaults"), k("SslProtocols"), Seg::Index(0)]));
        assert!(ssl.enums.contains(&Value::String("Tls13".into())));
    }

    #[test]
    fn values_are_checked_like_the_binder() {
        let level = [k("Logging"), k("LogLevel"), k("Default")];
        assert_eq!(check(&level, "\"Information\""), None);
        assert_eq!(check(&level, "\"information\""), None);
        assert!(matches!(check(&level, "\"Info\""), Some(Problem::Enum { allowed }) if allowed.len() == 7));
        assert!(matches!(check(&[k("Logging")], "42"), Some(Problem::Type { expected, found: "number" }) if expected == ["object"]));
        let allow = [k("Kestrel"), k("Endpoints"), k("Https"), k("Certificate"), k("AllowInvalid")];
        assert_eq!(check(&allow, "true"), None);
        assert_eq!(check(&allow, "\"false\""), None);
        assert!(matches!(check(&allow, "\"yes\""), Some(Problem::Type { .. })));
        // anyOf: Serilog's MinimumLevel is a level or an object.
        assert_eq!(check(&[k("Serilog"), k("MinimumLevel")], "\"Warning\""), None);
        assert_eq!(check(&[k("Serilog"), k("MinimumLevel")], "{ \"Default\": \"Debug\" }"), None);
        assert!(check(&[k("Serilog"), k("MinimumLevel")], "\"Loud\"").is_some());
        // Unknown sections are not checked.
        assert!(Schema::appsettings().at(&[k("MyApp"), k("Anything")]).is_empty());
    }

    #[test]
    fn pattern_names() {
        assert_eq!(pattern_name("^(nlog|Nlog|NLog)$").as_deref(), Some("NLog"));
        assert_eq!(pattern_name("^WebOptimizer$").as_deref(), Some("WebOptimizer"));
        assert_eq!(pattern_name("^Enrich:((?<X>[a-z]\\w*))$"), None);
    }
}
