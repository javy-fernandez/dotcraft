//! Typo inspection (Rider's spelling checks): comments, string literals, prose and the names
//! declared in C#, checked against installed Hunspell dictionaries on a worker thread.
//!
//! The UI thread only hands over rope snapshots (cheap clones) after the text has been still
//! for a moment; parsing, word checks (cached per word) and suggestions run on the worker,
//! and stale results are dropped by version. Suggestions are computed lazily for the word at
//! the caret, since Hunspell takes tens of milliseconds per word.

use crate::app::DotCraftApp;
use rc_core::{Inbox, Waker};
use rc_kit::i18n::{tr, trf};
use rc_lsp::lsp_types as lsp;
use rc_syntax::{Hl, Lang, highlight_lexical, highlight_lines, parse_now};
use rc_text::Rope;
use serde::{Deserialize, Serialize};
use spellbook::Dictionary;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant, SystemTime};

/// How long the text must stay unchanged before it is checked again.
const CHECK_DELAY: Duration = Duration::from_millis(300);
/// Bigger documents (generated code, data dumps) are not checked.
const MAX_CHARS: usize = 2_000_000;
/// Suggestions offered per word.
const MAX_SUGGESTIONS: usize = 5;
/// Command of the context actions made here.
const FIX_COMMAND: &str = "dotcraft.spelling";

// ------------------------------------------------------------------ dictionaries

/// Folders searched for Hunspell pairs (`en_US.aff` + `en_US.dic`), DotCraft's own first.
pub fn dictionary_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![rc_core::paths::data_dir().join("dictionaries")];
    if let Some(paths) = std::env::var_os("DOTCRAFT_DICTIONARIES") {
        dirs.extend(std::env::split_paths(&paths));
    }
    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join("Library/Spelling"));
        dirs.push(home.join(".local/share/hunspell"));
    }
    dirs.extend(["/Library/Spelling", "/usr/share/hunspell", "/usr/share/myspell", "/usr/share/myspell/dicts", "/usr/local/share/hunspell", "/opt/homebrew/share/hunspell"].map(PathBuf::from));
    dirs
}

/// Installed Hunspell pairs by locale, sorted (the first folder wins for a locale).
pub fn available_dictionaries() -> Vec<(String, PathBuf, PathBuf)> {
    let mut found: Vec<(String, PathBuf, PathBuf)> = Vec::new();
    for dir in dictionary_dirs() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let aff = entry.path();
            if aff.extension().is_none_or(|ext| ext != "aff") {
                continue;
            }
            let dic = aff.with_extension("dic");
            let locale = aff.file_stem().unwrap_or_default().to_string_lossy().into_owned();
            if dic.is_file() && !found.iter().any(|(name, _, _)| name == &locale) {
                found.push((locale, aff, dic));
            }
        }
    }
    found.sort_by(|a, b| a.0.cmp(&b.0));
    found
}

/// Reads an `.aff` / `.dic` file: UTF-8, or the 8-bit encoding older dictionaries use.
fn read_dictionary_file(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    Ok(match String::from_utf8(bytes) {
        Ok(text) => text,
        // ISO 8859-1 maps every byte to the same code point.
        Err(e) => e.into_bytes().into_iter().map(char::from).collect(),
    })
}

pub fn load_dictionary(aff: &Path, dic: &Path) -> Result<Dictionary, String> {
    let aff = read_dictionary_file(aff)?;
    let dic = read_dictionary_file(dic)?;
    // spellbook reads UTF-8 only; the text above already is.
    let aff = aff.lines().map(|l| if l.trim_start().starts_with("SET ") { "SET UTF-8" } else { l }).collect::<Vec<_>>().join("\n");
    Dictionary::new(&aff, &dic).map_err(|e| e.to_string())
}

/// Suggestions for `word`: whole words only (no "he lo" splits), closest first.
pub fn suggestions(dictionary: &Dictionary, word: &str, identifier: bool) -> Vec<String> {
    let mut found = Vec::new();
    dictionary.suggest(word, &mut found);
    let mut seen = HashSet::new();
    let mut found: Vec<String> = found
        .into_iter()
        .filter(|s| !s.contains(' ') && !(identifier && (s.contains('-') || s.contains('\''))))
        .map(|s| match_case(word, &s))
        .filter(|s| s != word && seen.insert(s.to_lowercase()))
        .collect();
    found.sort_by_key(|s| distance(&word.to_lowercase(), &s.to_lowercase()));
    found.truncate(MAX_SUGGESTIONS);
    found
}

/// `suggestion` written like `word`: Capitalized or lowercase first letter.
fn match_case(word: &str, suggestion: &str) -> String {
    let upper = word.chars().next().is_some_and(char::is_uppercase);
    let mut chars = suggestion.chars();
    match chars.next() {
        Some(first) if upper => first.to_uppercase().chain(chars).collect(),
        Some(first) if first.is_uppercase() && suggestion.chars().skip(1).all(char::is_lowercase) => first.to_lowercase().chain(chars).collect(),
        _ => suggestion.to_string(),
    }
}

/// Edit distance (insertions, deletions, substitutions).
fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut prev = row[0];
        row[0] = i + 1;
        for (j, &cb) in b.iter().enumerate() {
            let cur = row[j + 1];
            row[j + 1] = (prev + usize::from(ca != cb)).min(row[j] + 1).min(cur + 1);
            prev = cur;
        }
    }
    row[b.len()]
}

// ------------------------------------------------------------------ custom words

/// The user dictionary, shared by every solution and every spelling language.
pub fn user_dictionary_path() -> PathBuf {
    rc_core::paths::data_dir().join("spelling").join("user-dictionary.txt")
}

/// The solution's shared dictionary: Rider's `<solution>.DotSettings` (its "Save to dictionary"
/// at solution level), so a team using both IDEs shares one word list.
pub fn solution_dictionary_path(ws: &rc_core::Workspace) -> Option<PathBuf> {
    match &ws.kind {
        rc_core::WorkspaceKind::Solution(p) | rc_core::WorkspaceKind::Project(p) => {
            let mut name = p.file_name()?.to_os_string();
            name.push(".DotSettings");
            Some(p.with_file_name(name))
        }
        rc_core::WorkspaceKind::Folder => None,
    }
}

const DOTSETTINGS_KEY: &str = "/Default/UserDictionary/Words/=";

/// The words saved to a dictionary file (a word list, or Rider's `.DotSettings`).
fn parse_words(path: &Path, text: &str) -> Vec<String> {
    if !is_dotsettings(path) {
        return text.lines().map(|l| l.trim().to_lowercase()).filter(|l| !l.is_empty() && !l.starts_with('#')).collect();
    }
    let mut words = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find(DOTSETTINGS_KEY) {
        rest = &rest[at + DOTSETTINGS_KEY.len()..];
        let Some(end) = rest.find('/') else { break };
        let entry = &rest[..end];
        let value = rest[end..].split_once('>').map(|(_, v)| v.trim_start());
        if value.is_some_and(|v| v.starts_with("True")) {
            words.push(entry.to_lowercase());
        }
        rest = &rest[end..];
    }
    words
}

fn is_dotsettings(path: &Path) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case("DotSettings"))
}

/// Saves `word` (lowercase, like Rider) to a dictionary file, atomically.
pub fn save_word(path: &Path, word: &str) -> Result<(), String> {
    let word = word.trim().to_lowercase();
    if word.chars().count() < 2 || !word.chars().all(|c| c.is_alphabetic() || c == '\'') {
        return Err(tr("Only words made of letters can be saved to the dictionary").into());
    }
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e.to_string()),
    };
    if parse_words(path, &text).contains(&word) {
        return Ok(());
    }
    let new = if !is_dotsettings(path) {
        let mut text = text;
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text + &word + "\n"
    } else {
        let entry = format!("\t<s:Boolean x:Key=\"{DOTSETTINGS_KEY}{word}/@EntryIndexedValue\">True</s:Boolean>");
        if text.trim().is_empty() {
            format!("<wpf:ResourceDictionary xml:space=\"preserve\" xmlns:x=\"http://schemas.microsoft.com/winfx/2006/xaml\" xmlns:s=\"clr-namespace:System;assembly=mscorlib\" xmlns:ss=\"urn:shemas-jetbrains-com:settings-storage-xaml\" xmlns:wpf=\"http://schemas.microsoft.com/winfx/2006/xaml/presentation\">\n{entry}</wpf:ResourceDictionary>")
        } else {
            let close = "</wpf:ResourceDictionary>";
            let Some(at) = text.rfind(close) else {
                return Err(trf("{file} is not a settings file DotCraft can update", &[("file", &path.file_name().unwrap_or_default().to_string_lossy())]));
            };
            let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
            let before = text[..at].trim_end_matches([' ', '\t']);
            let sep = if before.ends_with('\n') { "" } else { newline };
            format!("{before}{sep}{entry}{}", &text[at..])
        }
    };
    rc_core::paths::write_atomic(path, new.as_bytes()).map_err(|e| e.to_string())
}

/// Words every .NET codebase uses that general dictionaries lack (IntelliJ ships a similar
/// list); the user and solution dictionaries add to it.
const TECH_WORDS: &[&str] = &[
    "accessor", "accessors", "ajax", "api", "apis", "appsettings", "args", "arity", "ascii", "async", "auth", "authn", "authz", "awaitable",
    "backend", "behaviour", "blazor", "bool", "boolean", "booleans", "boxed", "builtin", "callback", "callbacks", "callee", "charset", "checkbox",
    "classname", "codebase", "codegen", "config", "configs", "const", "cors", "csharp", "csproj", "csrf", "css", "ctor", "ctors", "dbcontext",
    "dbset", "deque", "dequeue", "deserialization", "deserialize", "deserialized", "deserializer", "deserializes", "dict", "dotnet", "dropdown",
    "dto", "dtos", "efcore", "enqueue", "enum", "enumerable", "enums", "env", "errno", "foreach", "frontend", "func", "funcs", "getter", "getters",
    "github", "gitignore", "grpc", "guid", "guids", "hashset", "html", "http", "https", "idempotent", "impl", "init", "inline", "inlined", "int",
    "ints", "io", "iterable", "javascript", "json", "jwt", "kestrel", "lambda", "lambdas", "linq", "localhost", "lookup", "lookups", "metadata",
    "middleware", "middlewares", "mixin", "msbuild", "mssql", "mutex", "mutexes", "mysql", "namespace", "namespaces", "nuget", "nullable", "nunit",
    "oauth", "param", "params", "parsable", "plugin", "plugins", "postgres", "postgresql", "prepend", "prepended", "protobuf", "razor", "readonly",
    "realtime", "redis", "refactor", "refactoring", "regex", "regexes", "repo", "repos", "rethrow", "roslyn", "runtime", "runtimes", "sdk", "sdks",
    "serializable", "serializer", "serializers", "setter", "setters", "signalr", "sln", "slnx", "sql", "sqlite", "stderr", "stdin", "stdout",
    "struct", "structs", "subclass", "subfolder", "subfolders", "submodule", "textbox", "timestamp", "timestamps", "todo", "tooltip", "tooltips",
    "tuple", "tuples", "typeof", "typescript", "uint", "ulong", "unboxed", "unhandled", "unicode", "uri", "uris", "url", "urls", "username", "usings",
    "utf", "uuid", "validator", "validators", "viewmodel", "viewmodels", "webhook", "webhooks", "whitespace", "wildcard", "workspace", "xaml", "xml",
    "xunit", "yaml",
];

// ------------------------------------------------------------------ finding typos

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Misspelling {
    /// Char range in the checked text.
    pub start: usize,
    pub end: usize,
    pub word: String,
    /// Part of a declared name (C#).
    pub identifier: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Region {
    Comment,
    /// A string literal: prose with escape sequences.
    String,
    /// Prose of Markdown, HTML and text files.
    Text,
    Identifier,
}

/// Whether a file is checked at all: code comments and strings, Markdown, HTML text and
/// plain `.txt` files. Solution files and unknown formats are not.
pub fn is_checked(lang: Lang, path: &Path) -> bool {
    match lang {
        Lang::Sln => false,
        Lang::Plain => path.extension().is_some_and(|e| e.eq_ignore_ascii_case("txt")),
        _ => true,
    }
}

/// The misspelled words of a source file; `None` when `cancelled` said so midway.
fn find_in_source(rope: &Rope, lang: Lang, ok: &mut impl FnMut(&str) -> bool, cancelled: &impl Fn() -> bool) -> Option<Vec<Misspelling>> {
    let mut regions: Vec<(usize, usize, Region)> = Vec::new();
    let tree = parse_now(lang, rope, None);
    if lang == Lang::Plain {
        regions.push((0, rope.len_chars(), Region::Text));
    } else {
        let lines = rope.len_lines();
        let spans = match &tree {
            Some(tree) => highlight_lines(lang, tree, rope, 0, lines),
            None => highlight_lexical(lang, rope, 0, lines).unwrap_or_default(),
        };
        let strings = matches!(lang, Lang::CSharp | Lang::JavaScript | Lang::TypeScript | Lang::Tsx);
        let prose = lang == Lang::Markdown;
        for (line, spans) in spans.iter().enumerate() {
            if line % 256 == 0 && cancelled() {
                return None;
            }
            let base = rope.line_to_char(line);
            let len = rope.line(line).len_chars();
            let mut at = 0;
            let mut push = |start: usize, end: usize, region: Region| {
                if let Some(last) = regions.last_mut()
                    && last.2 == region
                    && last.1 == start
                {
                    last.1 = end;
                } else if start < end {
                    regions.push((start, end, region));
                }
            };
            for span in spans {
                if prose && span.start > at {
                    push(base + at, base + span.start, Region::Text);
                }
                let region = match span.hl {
                    Hl::Comment | Hl::DocComment => Some(Region::Comment),
                    Hl::String | Hl::Escape if strings => Some(Region::String),
                    Hl::Heading | Hl::Plain if prose => Some(Region::Text),
                    _ => None,
                };
                if let Some(region) = region {
                    push(base + span.start, base + span.end.min(len), region);
                }
                at = at.max(span.end);
            }
            if prose && at < len {
                push(base + at, base + len, Region::Text);
            }
        }
    }
    if let Some(tree) = &tree {
        match lang {
            Lang::CSharp => declared_names(tree, rope, &mut regions),
            Lang::Html => html_text(tree, rope, &mut regions),
            Lang::Markdown => without_code(tree, rope, &mut regions),
            _ => {}
        }
    }
    regions.sort_by_key(|r| r.0);
    let mut found = Vec::new();
    let mut buf = String::new();
    for (i, &(start, end, region)) in regions.iter().enumerate() {
        if i % 512 == 0 && cancelled() {
            return None;
        }
        buf.clear();
        buf.extend(rope.slice(start..end).chars());
        let chars: Vec<char> = buf.chars().collect();
        match region {
            Region::Identifier => check_identifier(&chars, start, ok, &mut found),
            Region::Comment | Region::Text => check_prose(&chars, start, false, ok, &mut found),
            Region::String => check_prose(&chars, start, true, ok, &mut found),
        }
    }
    Some(found)
}

/// Names introduced by C# declarations (types, members, parameters, locals): only these are
/// checked, never the names used from elsewhere, nor members that override a base one.
fn declared_names(tree: &rc_syntax::tree_sitter::Tree, rope: &Rope, out: &mut Vec<(usize, usize, Region)>) {
    const NAMED: &[&str] = &[
        "class_declaration", "struct_declaration", "interface_declaration", "enum_declaration", "record_declaration", "delegate_declaration",
        "method_declaration", "property_declaration", "event_declaration", "enum_member_declaration", "local_function_statement", "parameter",
        "variable_declarator", "type_parameter", "catch_declaration", "tuple_element", "from_clause",
    ];
    let mut cursor = tree.walk();
    loop {
        let node = cursor.node();
        let name = if NAMED.contains(&node.kind()) {
            let overrides = (0..node.child_count()).filter_map(|i| node.child(i)).any(|c| c.kind() == "modifier" && rope.byte_slice(c.byte_range()) == "override");
            if overrides { None } else { node.child_by_field_name("name") }
        } else if matches!(node.kind(), "single_variable_designation" | "foreach_statement") {
            node.child_by_field_name("left").or_else(|| (node.kind() == "single_variable_designation").then(|| node.named_child(0)).flatten())
        } else {
            None
        };
        if let Some(name) = name.filter(|n| n.kind() == "identifier") {
            out.push((rope.byte_to_char(name.start_byte()), rope.byte_to_char(name.end_byte()), Region::Identifier));
        }
        if cursor.goto_first_child() {
            continue;
        }
        while !cursor.goto_next_sibling() {
            if !cursor.goto_parent() {
                return;
            }
        }
    }
}

/// Markdown prose without its code blocks and raw HTML.
fn without_code(tree: &rc_syntax::tree_sitter::Tree, rope: &Rope, regions: &mut Vec<(usize, usize, Region)>) {
    let mut code = Vec::new();
    let mut cursor = tree.walk();
    'walk: loop {
        let node = cursor.node();
        let skip = matches!(node.kind(), "fenced_code_block" | "indented_code_block" | "html_block" | "minus_metadata" | "plus_metadata");
        if skip {
            code.push((rope.byte_to_char(node.start_byte()), rope.byte_to_char(node.end_byte())));
        }
        if !skip && cursor.goto_first_child() {
            continue;
        }
        while !cursor.goto_next_sibling() {
            if !cursor.goto_parent() {
                break 'walk;
            }
        }
    }
    if code.is_empty() {
        return;
    }
    let mut kept = Vec::with_capacity(regions.len());
    for &(start, end, region) in regions.iter() {
        let mut at = start;
        for &(a, b) in code.iter().filter(|(a, b)| *a < end && *b > start) {
            if a > at {
                kept.push((at, a, region));
            }
            at = at.max(b);
        }
        if at < end {
            kept.push((at, end, region));
        }
    }
    *regions = kept;
}

/// The text between HTML tags (not scripts, styles or attribute values).
fn html_text(tree: &rc_syntax::tree_sitter::Tree, rope: &Rope, out: &mut Vec<(usize, usize, Region)>) {
    let mut cursor = tree.walk();
    loop {
        let node = cursor.node();
        if node.kind() == "text" {
            out.push((rope.byte_to_char(node.start_byte()), rope.byte_to_char(node.end_byte()), Region::Text));
        }
        if node.kind() != "text" && cursor.goto_first_child() {
            continue;
        }
        while !cursor.goto_next_sibling() {
            if !cursor.goto_parent() {
                return;
            }
        }
    }
}

/// Words of prose (comments, strings, Markdown, commit messages). Markup tags, `code` spans
/// and tokens that look like code (paths, URLs, dotted names, numbers, `snake_case`) are
/// skipped; the rest is split at camel humps, like Rider.
fn check_prose(chars: &[char], base: usize, escapes: bool, ok: &mut impl FnMut(&str) -> bool, out: &mut Vec<Misspelling>) {
    let mut text = chars.to_vec();
    blank_markup(&mut text);
    if escapes {
        blank_escapes(&mut text);
    }
    let mut i = 0;
    while i < text.len() {
        if text[i].is_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        while i < text.len() && !text[i].is_whitespace() {
            i += 1;
        }
        // Surrounding punctuation is not part of the word: "(word)," or "**word**".
        let (mut a, mut b) = (start, i);
        while a < b && !text[a].is_alphanumeric() {
            a += 1;
        }
        while b > a && !text[b - 1].is_alphanumeric() {
            b -= 1;
        }
        if a == b || !text[a..b].iter().all(|&c| c.is_alphabetic() || c == '\'' || c == '’' || c == '-') {
            continue;
        }
        // Hyphenated words are checked part by part.
        let mut p = a;
        while p < b {
            while p < b && !text[p].is_alphabetic() {
                p += 1;
            }
            let q = (p..b).find(|&k| text[k] == '-').unwrap_or(b);
            check_humps(&text, p, q, base, false, ok, out);
            p = q;
        }
    }
}

/// Spaces out markup tags (`<see cref="X"/>`) and `code` spans, keeping offsets.
fn blank_markup(text: &mut [char]) {
    let mut i = 0;
    while i < text.len() {
        let close = match text[i] {
            '<' if text.get(i + 1).is_some_and(|c| c.is_ascii_alphabetic() || *c == '/') => '>',
            '`' => '`',
            _ => {
                i += 1;
                continue;
            }
        };
        match (i + 1..text.len()).find(|&k| text[k] == close) {
            Some(end) => {
                text[i..=end].fill(' ');
                i = end + 1;
            }
            None => i += 1,
        }
    }
}

/// Spaces out escape sequences of string literals (`\n` in "Hello\nWorld"); a doubled
/// backslash stays, so paths remain one token (skipped as code).
fn blank_escapes(text: &mut [char]) {
    let mut i = 0;
    while i + 1 < text.len() {
        if text[i] == '\\' {
            if text[i + 1] != '\\' {
                text[i..i + 2].fill(' ');
            }
            i += 2;
        } else {
            i += 1;
        }
    }
}

/// A declared name: split at underscores, digits and camel humps.
fn check_identifier(chars: &[char], base: usize, ok: &mut impl FnMut(&str) -> bool, out: &mut Vec<Misspelling>) {
    let mut i = 0;
    while i < chars.len() {
        if !chars[i].is_alphabetic() {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && chars[i].is_alphabetic() {
            i += 1;
        }
        check_humps(chars, start, i, base, true, ok, out);
    }
}

/// Splits `chars[start..end]` at camel humps (`fooBar`, `HTTPServer`) and checks each part.
fn check_humps(chars: &[char], start: usize, end: usize, base: usize, identifier: bool, ok: &mut impl FnMut(&str) -> bool, out: &mut Vec<Misspelling>) {
    let mut part = start;
    while part < end {
        let mut next = part + 1;
        while next < end {
            let (prev, cur) = (chars[next - 1], chars[next]);
            let hump = cur.is_uppercase() && prev.is_lowercase();
            let acronym_end = cur.is_uppercase() && prev.is_uppercase() && chars.get(next + 1).is_some_and(|c| c.is_lowercase()) && next + 1 < end;
            if hump || acronym_end {
                break;
            }
            next += 1;
        }
        let word: String = chars[part..next].iter().collect();
        let word = word.trim_matches(|c| c == '\'' || c == '’');
        let letters = word.chars().filter(|c| c.is_alphabetic()).count();
        // Short words and acronyms (`IO`, `HTTP`) are not checked.
        if letters >= 3 && !word.chars().all(|c| !c.is_lowercase()) && !ok(word) {
            out.push(Misspelling { start: base + part, end: base + part + word.chars().count(), word: word.to_string(), identifier });
        }
        part = next;
    }
}

/// The misspelled words of a commit message.
pub fn find_in_message(text: &str, ok: &mut impl FnMut(&str) -> bool) -> Vec<Misspelling> {
    let chars: Vec<char> = text.chars().collect();
    let mut found = Vec::new();
    check_prose(&chars, 0, false, ok, &mut found);
    found
}

/// Whether `word` is spelled right: saved words, the technical list, then the dictionary
/// (as written, or lowercase for capitalised words at the start of a sentence or a hump).
fn is_known(word: &str, dictionary: &Dictionary, custom: &HashSet<String>) -> bool {
    let lower = word.to_lowercase();
    custom.contains(&lower) || TECH_WORDS.binary_search(&lower.as_str()).is_ok() || dictionary.check(word) || dictionary.check(&lower)
}

// ------------------------------------------------------------------ worker

enum Request {
    Discover,
    /// `ticket` cancels the check once a newer one is asked for.
    Check { ticket: u64, generation: u64, doc: u64, version: u64, lang: Lang, languages: Vec<String>, rope: Rope, words: Vec<PathBuf> },
    CheckMessage { generation: u64, hash: u64, languages: Vec<String>, text: String, words: Vec<PathBuf> },
    Suggest { languages: Vec<String>, word: String, identifier: bool },
    Save { word: String, path: PathBuf },
}

enum Reply {
    Languages(Vec<String>),
    Checked { generation: u64, doc: u64, version: u64, words: Vec<Misspelling> },
    CheckedMessage { generation: u64, hash: u64, words: Vec<Misspelling> },
    Suggestions { word: String, list: Vec<String> },
    Saved { word: String, error: Option<String> },
}

struct Worker {
    out: rc_core::inbox::Outbox<Reply>,
    latest: Arc<AtomicU64>,
    /// Loaded dictionaries; `None` for a language without one (looked for again on Discover).
    dictionaries: HashMap<String, Option<Dictionary>>,
    /// Dictionary answers per word, for the current languages.
    known: HashMap<String, bool>,
    known_languages: Vec<String>,
    files: HashMap<PathBuf, (Option<SystemTime>, Vec<String>)>,
}

impl Worker {
    fn run(mut self, rx: mpsc::Receiver<Request>) {
        while let Ok(request) = rx.recv() {
            match request {
                Request::Discover => {
                    // A dictionary installed since is loaded on the next check.
                    self.dictionaries.retain(|_, d| d.is_some());
                    self.out.send(Reply::Languages(available_dictionaries().into_iter().map(|(name, _, _)| name).collect()));
                }
                Request::Check { ticket, generation, doc, version, lang, languages, rope, words } => {
                    if self.latest.load(Ordering::Relaxed) != ticket {
                        continue;
                    }
                    let latest = self.latest.clone();
                    let cancelled = || latest.load(Ordering::Relaxed) != ticket;
                    let custom = self.custom_words(&words);
                    let Some(mut ok) = self.checker(&languages, &custom) else { continue };
                    let found = find_in_source(&rope, lang, &mut ok, &cancelled);
                    drop(ok);
                    if let Some(words) = found {
                        self.out.send(Reply::Checked { generation, doc, version, words });
                    }
                }
                Request::CheckMessage { generation, hash, languages, text, words } => {
                    let custom = self.custom_words(&words);
                    let Some(mut ok) = self.checker(&languages, &custom) else { continue };
                    let words = find_in_message(&text, &mut ok);
                    drop(ok);
                    self.out.send(Reply::CheckedMessage { generation, hash, words });
                }
                Request::Suggest { languages, word, identifier } => {
                    self.load(&languages);
                    let mut list: Vec<String> = Vec::new();
                    for d in languages.iter().filter_map(|l| self.dictionaries.get(l)?.as_ref()) {
                        for s in suggestions(d, &word, identifier) {
                            if !list.contains(&s) {
                                list.push(s);
                            }
                        }
                    }
                    list.sort_by_key(|s| distance(&word.to_lowercase(), &s.to_lowercase()));
                    list.truncate(MAX_SUGGESTIONS);
                    self.out.send(Reply::Suggestions { word, list });
                }
                Request::Save { word, path } => {
                    let error = save_word(&path, &word).err();
                    self.files.remove(&path);
                    self.out.send(Reply::Saved { word, error });
                }
            }
        }
    }

    /// Loads the dictionaries of `languages` not tried yet.
    fn load(&mut self, languages: &[String]) {
        for language in languages {
            if self.dictionaries.contains_key(language) {
                continue;
            }
            let loaded = available_dictionaries().into_iter().find(|(name, _, _)| name == language).and_then(|(_, aff, dic)| match load_dictionary(&aff, &dic) {
                Ok(d) => Some(d),
                Err(e) => {
                    eprintln!("spelling dictionary {}: {e}", dic.display());
                    None
                }
            });
            self.dictionaries.insert(language.clone(), loaded);
        }
    }

    /// A word test with the per-word cache: a word is right when any of the dictionaries of
    /// `languages` knows it. `None` when none of them is installed.
    fn checker<'a>(&'a mut self, languages: &[String], custom: &'a HashSet<String>) -> Option<impl FnMut(&str) -> bool + 'a> {
        self.load(languages);
        if self.known_languages != languages || self.known.len() > 200_000 {
            self.known.clear();
            self.known_languages = languages.to_vec();
        }
        let dictionaries: Vec<&Dictionary> = languages.iter().filter_map(|l| self.dictionaries.get(l)?.as_ref()).collect();
        if dictionaries.is_empty() {
            return None;
        }
        let known = &mut self.known;
        let none = HashSet::new();
        Some(move |word: &str| {
            if custom.contains(&word.to_lowercase()) {
                return true;
            }
            if let Some(&k) = known.get(word) {
                return k;
            }
            let k = dictionaries.iter().any(|d| is_known(word, d, &none));
            known.insert(word.to_string(), k);
            k
        })
    }

    /// The saved words of these files, re-read when a file changes.
    fn custom_words(&mut self, paths: &[PathBuf]) -> HashSet<String> {
        let mut all = HashSet::new();
        for path in paths {
            let mtime = std::fs::metadata(path).and_then(|m| m.modified()).ok();
            let entry = self.files.entry(path.clone()).or_insert((None, Vec::new()));
            if entry.0 != mtime || mtime.is_none() {
                let words = std::fs::read_to_string(path).map(|t| parse_words(path, &t)).unwrap_or_default();
                *entry = (mtime, words);
            }
            all.extend(entry.1.iter().cloned());
        }
        all
    }
}

// ------------------------------------------------------------------ UI side

/// Spelling state of the app: the worker, results for the commit message and suggestions.
pub struct Spelling {
    requests: mpsc::Sender<Request>,
    replies: Inbox<Reply>,
    latest: Arc<AtomicU64>,
    /// Installed dictionaries (locales).
    pub installed: Vec<String>,
    /// The languages, and the dictionary files of the workspace, results are for.
    languages: Vec<String>,
    files: Vec<PathBuf>,
    root: Option<PathBuf>,
    /// Bumped when the languages or the saved words change: results of older ones are dropped.
    generation: u64,
    requested: Option<(u64, u64, u64)>,
    /// The document version waiting for the text to settle, and since when.
    settling: Option<((u64, u64), Instant)>,
    message_requested: Option<(u64, u64)>,
    /// Misspelled words of the commit message.
    pub message: Vec<Misspelling>,
    suggestions: HashMap<String, Vec<String>>,
    asked: HashSet<String>,
    /// ⌥⏎ waiting for the suggestions of the word at the caret: document, caret, screen spot.
    waiting: Option<(u64, usize, egui::Pos2)>,
    /// Typo actions to show with the language server's context actions.
    pub pending_actions: Option<Vec<lsp::CodeActionOrCommand>>,
    discovered: Instant,
}

impl Spelling {
    pub fn new(waker: Waker) -> Self {
        let replies = Inbox::new(waker);
        let latest = Arc::new(AtomicU64::new(0));
        let (requests, rx) = mpsc::channel();
        let worker = Worker { out: replies.sender(), latest: latest.clone(), dictionaries: HashMap::new(), known: HashMap::new(), known_languages: Vec::new(), files: HashMap::new() };
        let _ = std::thread::Builder::new().name("spelling".into()).spawn(move || worker.run(rx));
        let _ = requests.send(Request::Discover);
        Spelling {
            requests,
            replies,
            latest,
            installed: Vec::new(),
            languages: Vec::new(),
            files: vec![user_dictionary_path()],
            root: None,
            generation: 0,
            requested: None,
            settling: None,
            message_requested: None,
            message: Vec::new(),
            suggestions: HashMap::new(),
            asked: HashSet::new(),
            waiting: None,
            pending_actions: None,
            discovered: Instant::now(),
        }
    }

    /// Looks for installed dictionaries again while the Settings dialog shows them, so one
    /// added meanwhile appears (at most every few seconds).
    pub fn discover_if_stale(&mut self) {
        if self.discovered.elapsed() > Duration::from_secs(3) {
            self.discovered = Instant::now();
            let _ = self.requests.send(Request::Discover);
        }
    }

    /// Suggestions for `word`, if computed; asks the worker for them otherwise.
    fn suggestions_for(&mut self, word: &str, identifier: bool) -> Option<&Vec<String>> {
        if !self.suggestions.contains_key(word) && self.asked.insert(word.to_string()) {
            let _ = self.requests.send(Request::Suggest { languages: self.languages.clone(), word: word.to_string(), identifier });
        }
        self.suggestions.get(word)
    }
}

fn hash(text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut h);
    h.finish()
}

/// Once per frame: applies results and asks for new checks of the active document and the
/// commit message. Cheap when nothing changed (a few comparisons).
pub fn update(app: &mut DotCraftApp, ctx: &egui::Context) {
    let mut reopen = None;
    for reply in app.spelling.replies.drain() {
        match reply {
            Reply::Languages(list) => {
                if list != app.spelling.installed {
                    app.spelling.installed = list;
                    app.spelling.generation += 1;
                }
            }
            Reply::Checked { generation, doc, version, words } => {
                if generation != app.spelling.generation {
                    continue;
                }
                if let Some(d) = app.docs.list.iter_mut().find(|d| d.editor.id == doc)
                    && d.editor.buffer.version() == version
                {
                    d.editor.spelling = words.into_iter().map(|w| rc_editor::SpellingMark { start: w.start, end: w.end, word: w.word, identifier: w.identifier }).collect();
                }
            }
            Reply::CheckedMessage { generation, hash: h, words } => {
                if generation == app.spelling.generation && h == hash(&app.vcs.message) {
                    app.spelling.message = words;
                }
            }
            Reply::Suggestions { word, list } => {
                app.spelling.asked.remove(&word);
                app.spelling.suggestions.insert(word, list);
                if let Some((doc, caret, screen)) = app.spelling.waiting.take()
                    && app.docs.active_doc().is_some_and(|d| d.editor.id == doc && d.editor.caret() == caret)
                {
                    reopen = Some(screen);
                }
            }
            Reply::Saved { word, error } => match error {
                Some(e) => app.notify(e, true),
                None => {
                    // The word disappears everywhere at once; the next checks agree.
                    let lower = word.to_lowercase();
                    for d in &mut app.docs.list {
                        d.editor.spelling.retain(|w| w.word.to_lowercase() != lower);
                    }
                    app.spelling.message.retain(|w| w.word.to_lowercase() != lower);
                    app.spelling.generation += 1;
                }
            },
        }
    }
    if let Some(screen) = reopen {
        app.run_insight_action(rc_editor::Action::ShowIntentions, screen);
    }
    let sp = &mut app.spelling;
    if !app.settings.spellcheck_enabled {
        if sp.requested.take().is_some() | sp.message_requested.take().is_some() {
            sp.latest.fetch_add(1, Ordering::Relaxed);
            sp.message.clear();
            for d in &mut app.docs.list {
                d.editor.spelling.clear();
            }
        }
        return;
    }
    if sp.languages != app.settings.spelling_languages {
        sp.languages = app.settings.spelling_languages.clone();
        sp.generation += 1;
        sp.suggestions.clear();
        sp.asked.clear();
    }
    let root = app.workspace.as_ref().map(|w| &w.ws.root);
    if sp.root.as_ref() != root {
        sp.root = root.cloned();
        sp.files = std::iter::once(user_dictionary_path()).chain(app.workspace.as_ref().and_then(|w| solution_dictionary_path(&w.ws))).collect();
        sp.generation += 1;
    }

    // The commit message (short: checked on every change).
    if app.vcs.message.trim().is_empty() {
        sp.message.clear();
        sp.message_requested = None;
    } else {
        let key = (hash(&app.vcs.message), sp.generation);
        if sp.message_requested != Some(key) {
            sp.message_requested = Some(key);
            let _ = sp.requests.send(Request::CheckMessage { generation: sp.generation, hash: key.0, languages: sp.languages.clone(), text: app.vcs.message.clone(), words: sp.files.clone() });
        }
    }

    let Some(active) = app.docs.active else { return };
    let doc = &mut app.docs.list[active];
    if !is_checked(doc.editor.syntax.lang, &doc.path) || doc.editor.buffer.rope().len_chars() > MAX_CHARS {
        doc.editor.spelling.clear();
        return;
    }
    // Suggestions for the word at the caret, before ⌥⏎ asks for them.
    let caret = doc.editor.caret();
    if let Some(w) = doc.editor.spelling.iter().find(|w| w.start <= caret && caret <= w.end) {
        let (word, identifier) = (w.word.clone(), w.identifier);
        sp.suggestions_for(&word, identifier);
    }
    let key = (doc.editor.id, doc.editor.buffer.version(), sp.generation);
    if sp.requested == Some(key) {
        return;
    }
    let now = Instant::now();
    let since = match sp.settling {
        Some((k, since)) if k == (key.0, key.1) => since,
        _ => {
            sp.settling = Some(((key.0, key.1), now));
            now
        }
    };
    let waited = now - since;
    // A newly opened (or never checked) document is checked at once; edits wait a moment.
    let fresh = sp.requested.is_none_or(|r| r.0 != key.0) && doc.editor.spelling.is_empty();
    if !fresh && waited < CHECK_DELAY {
        ctx.request_repaint_after(CHECK_DELAY - waited);
        return;
    }
    sp.requested = Some(key);
    let ticket = sp.latest.fetch_add(1, Ordering::Relaxed) + 1;
    let _ = sp.requests.send(Request::Check {
        ticket,
        generation: key.2,
        doc: key.0,
        version: key.1,
        lang: doc.editor.syntax.lang,
        languages: sp.languages.clone(),
        rope: doc.editor.buffer.rope().clone(),
        words: sp.files.clone(),
    });
}

// ------------------------------------------------------------------ fixes

/// What a typo action does.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SpellingFix {
    /// Replaces the word in a document (comments, strings, prose).
    Replace { doc: u64, start: usize, end: usize, word: String, text: String },
    /// Renames the declared symbol (the language server updates every usage).
    Rename { doc: u64, pos: usize, name: String },
    /// Replaces the word in the commit message.
    ReplaceInMessage { start: usize, end: usize, word: String, text: String },
    /// Saves the word to the user dictionary, or the solution's (`solution`).
    Save { word: String, solution: bool },
}

impl SpellingFix {
    pub fn title(&self) -> String {
        match self {
            SpellingFix::Replace { text, .. } | SpellingFix::ReplaceInMessage { text, .. } => trf("Change to '{word}'", &[("word", text)]),
            SpellingFix::Rename { name, .. } => trf("Rename to '{name}'", &[("name", name)]),
            SpellingFix::Save { word, solution: false } => trf("Save '{word}' to the user dictionary", &[("word", &word.to_lowercase())]),
            SpellingFix::Save { word, solution: true } => trf("Save '{word}' to the solution dictionary", &[("word", &word.to_lowercase())]),
        }
    }

    fn action(&self) -> lsp::CodeActionOrCommand {
        let title = self.title();
        let command = lsp::Command { title: title.clone(), command: FIX_COMMAND.into(), arguments: Some(vec![serde_json::to_value(self).unwrap_or_default()]) };
        lsp::CodeActionOrCommand::CodeAction(lsp::CodeAction { title, kind: None, command: Some(command), ..Default::default() })
    }
}

/// The typo fix behind a Context Actions entry made here.
pub fn fix_of(action: &lsp::CodeActionOrCommand) -> Option<SpellingFix> {
    let cmd = match action {
        lsp::CodeActionOrCommand::Command(c) => c,
        lsp::CodeActionOrCommand::CodeAction(a) => a.command.as_ref()?,
    };
    (cmd.command == FIX_COMMAND).then(|| serde_json::from_value(cmd.arguments.as_ref()?.first()?.clone()).ok()).flatten()
}

/// The fixes for the typo at the caret of the active document, and whether its suggestions
/// are known yet; `None` when the caret is not on a typo.
pub fn fixes_at_caret(app: &mut DotCraftApp) -> Option<(Vec<SpellingFix>, bool)> {
    let doc = app.docs.active_doc()?;
    let caret = doc.editor.caret();
    let mark = doc.editor.spelling.iter().find(|w| w.start <= caret && caret <= w.end)?.clone();
    let id = doc.editor.id;
    let rope = doc.editor.buffer.rope().clone();
    let renames = mark.identifier && app.insight.ready() && crate::insight::is_csharp(doc);
    let solution = app.workspace.as_ref().and_then(|w| solution_dictionary_path(&w.ws)).is_some();
    let suggestions = app.spelling.suggestions_for(&mark.word, mark.identifier).cloned();
    let ready = suggestions.is_some();
    let mut fixes = Vec::new();
    for s in suggestions.unwrap_or_default() {
        if !mark.identifier {
            fixes.push(SpellingFix::Replace { doc: id, start: mark.start, end: mark.end, word: mark.word.clone(), text: s });
        } else if renames {
            // The whole name with the misspelled hump replaced.
            let is_name = |c: char| c.is_alphanumeric() || c == '_';
            let mut a = mark.start;
            while a > 0 && is_name(rope.char(a - 1)) {
                a -= 1;
            }
            let mut b = mark.end;
            while b < rope.len_chars() && is_name(rope.char(b)) {
                b += 1;
            }
            let name = format!("{}{}{}", rope.slice(a..mark.start), s, rope.slice(mark.end..b));
            fixes.push(SpellingFix::Rename { doc: id, pos: mark.start, name });
        }
    }
    fixes.push(SpellingFix::Save { word: mark.word.clone(), solution: false });
    if solution {
        fixes.push(SpellingFix::Save { word: mark.word, solution: true });
    }
    // Without a rename, a declared name only offers the dictionaries: nothing to wait for.
    Some((fixes, ready || (mark.identifier && !renames)))
}

/// The fixes for a typo of the commit message.
pub fn fixes_in_message(app: &mut DotCraftApp, typo: &Misspelling) -> Vec<SpellingFix> {
    let solution = app.workspace.as_ref().and_then(|w| solution_dictionary_path(&w.ws)).is_some();
    let mut fixes: Vec<SpellingFix> = app
        .spelling
        .suggestions_for(&typo.word, false)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|text| SpellingFix::ReplaceInMessage { start: typo.start, end: typo.end, word: typo.word.clone(), text })
        .collect();
    fixes.push(SpellingFix::Save { word: typo.word.clone(), solution: false });
    if solution {
        fixes.push(SpellingFix::Save { word: typo.word.clone(), solution: true });
    }
    fixes
}

/// ⌥⏎ on a typo: its fixes as context actions; `None` when the caret is not on one. When the
/// suggestions are still being computed, the popup opens as soon as they arrive.
pub fn context_actions(app: &mut DotCraftApp, screen: egui::Pos2) -> Option<Vec<lsp::CodeActionOrCommand>> {
    let (fixes, ready) = fixes_at_caret(app)?;
    if !ready {
        let doc = app.docs.active_doc()?;
        app.spelling.waiting = Some((doc.editor.id, doc.editor.caret(), screen));
        return Some(Vec::new());
    }
    Some(fixes.iter().map(SpellingFix::action).collect())
}

/// Applies a typo fix.
pub fn apply(app: &mut DotCraftApp, fix: SpellingFix) {
    match fix {
        SpellingFix::Replace { doc, start, end, word, text } => {
            if let Some(d) = app.docs.list.iter_mut().find(|d| d.editor.id == doc)
                && end <= d.editor.buffer.rope().len_chars()
                && d.editor.buffer.rope().slice(start..end) == word.as_str()
            {
                d.editor.replace(start, end, &text);
            }
        }
        SpellingFix::Rename { doc, pos, name } => {
            if let Some(d) = app.docs.list.iter_mut().find(|d| d.editor.id == doc) {
                app.insight.flush(d);
                app.insight.rename(d, pos, &name);
            }
        }
        SpellingFix::ReplaceInMessage { start, end, word, text } => {
            let message = &mut app.vcs.message;
            let byte = |c: usize| message.char_indices().nth(c).map_or(message.len(), |(b, _)| b);
            let (a, b) = (byte(start), byte(end));
            if message.get(a..b) == Some(word.as_str()) {
                message.replace_range(a..b, &text);
            }
        }
        SpellingFix::Save { word, solution } => {
            let path = if solution { app.workspace.as_ref().and_then(|w| solution_dictionary_path(&w.ws)) } else { Some(user_dictionary_path()) };
            if let Some(path) = path {
                let _ = app.spelling.requests.send(Request::Save { word, path });
            }
        }
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    fn dictionary() -> Dictionary {
        let words = ["hello", "world", "message", "name", "value", "comment", "string", "correct", "wrong", "get", "the", "ship", "some", "here", "with", "and", "for"];
        Dictionary::new("SET UTF-8\nTRY esianrtolcdugmphbyfvkwz\n", &format!("{}\n{}\n", words.len(), words.join("\n"))).unwrap()
    }

    fn find(text: &str, lang: Lang) -> Vec<(String, bool)> {
        let d = dictionary();
        let none = HashSet::new();
        let mut ok = |w: &str| is_known(w, &d, &none);
        find_in_source(&Rope::from_str(text), lang, &mut ok, &|| false).unwrap().into_iter().map(|m| (m.word, m.identifier)).collect()
    }

    #[test]
    fn csharp_checks_comments_strings_and_declared_names_only() {
        let text = "// helo comment\nclass WrongNme { string valu = \"wrld string\"; void Get(int prm) { Console.WriteLine(Foo.Barr); } }\n";
        let found = find(text, Lang::CSharp);
        assert!(found.contains(&("helo".into(), false)));
        assert!(found.contains(&("Nme".into(), true)));
        assert!(found.contains(&("valu".into(), true)));
        assert!(found.contains(&("prm".into(), true)));
        assert!(found.contains(&("wrld".into(), false)));
        // Keywords, types and names used from elsewhere are not checked.
        for word in ["class", "string", "void", "int", "Console", "Line", "Barr", "Foo"] {
            assert!(!found.iter().any(|(w, _)| w == word), "{word} was checked: {found:?}");
        }
    }

    #[test]
    fn overrides_and_code_like_tokens_are_skipped() {
        let text = "/// <summary>Get the <see cref=\"Wrld\"/> valu</summary>\nclass A { public override string Nmee() => \"https://exampl.com/pth C:\\\\tmp\\\\fil nme_x a.b.c `wrld`\"; }";
        let found = find(text, Lang::CSharp);
        let words: Vec<&str> = found.iter().map(|(w, _)| w.as_str()).collect();
        assert_eq!(words, ["valu"]);
    }

    #[test]
    fn camel_humps_acronyms_and_short_words() {
        let text = "// DotCraftNme HTTPServr IO ab Hello\n";
        let words: Vec<String> = find(text, Lang::CSharp).into_iter().map(|(w, _)| w).collect();
        assert!(words.contains(&"Nme".to_string()));
        assert!(words.contains(&"Servr".to_string()));
        assert!(!words.iter().any(|w| w == "IO" || w == "ab" || w == "HTTP" || w == "Hello"));
    }

    #[test]
    fn markdown_prose_but_not_code_and_xml_comments_only() {
        let md = find("# Helo\n\nSome wrld here with `cde` and:\n\n```cs\nlet wrng = 1;\n```\n\n    var nmee = 2;\n", Lang::Markdown);
        let words: Vec<&str> = md.iter().map(|(w, _)| w.as_str()).collect();
        assert!(words.contains(&"Helo") && words.contains(&"wrld"), "{words:?}");
        assert_eq!(words, ["Helo", "wrld"]);
        let xml = find("<Project><!-- helo --><PropertyGroup><Nme>wrld</Nme></PropertyGroup></Project>", Lang::Xml);
        assert_eq!(xml, [("helo".to_string(), false)]);
        let html = find("<p class=\"btn-primry\">Helo <b>wrld</b></p><script>var nmee = 1;</script>", Lang::Html);
        let words: Vec<&str> = html.iter().map(|(w, _)| w.as_str()).collect();
        assert_eq!(words, ["Helo", "wrld"]);
    }

    #[test]
    fn javascript_comments_and_strings_without_identifiers() {
        let found = find("// helo comment\nconst wrongNme = \"wrld\";", Lang::JavaScript);
        assert_eq!(found, [("helo".to_string(), false), ("wrld".to_string(), false)]);
    }

    #[test]
    fn commit_messages_and_saved_words() {
        let d = dictionary();
        let custom = HashSet::from(["dotcraft".to_string()]);
        let mut ok = |w: &str| is_known(w, &d, &custom);
        let found = find_in_message("Ship DotCraft mesage for namespace usings", &mut ok);
        assert_eq!(found.iter().map(|m| m.word.as_str()).collect::<Vec<_>>(), ["Dot", "Craft", "mesage"]);
        assert_eq!((found[2].start, found[2].end), (14, 20));
        assert!(suggestions(&d, "mesage", false).contains(&"message".to_string()));
        assert_eq!(suggestions(&d, "Nme", true).first().map(String::as_str), Some("Name"));
    }

    #[test]
    fn tech_words_are_sorted_for_binary_search() {
        assert!(TECH_WORDS.windows(2).all(|w| w[0] < w[1]), "{:?}", TECH_WORDS.windows(2).find(|w| w[0] >= w[1]));
    }

    #[test]
    fn words_are_saved_to_word_lists_and_rider_settings() {
        let dir = tempfile::tempdir().unwrap();
        let list = dir.path().join("spelling").join("user-dictionary.txt");
        save_word(&list, "DotCraft").unwrap();
        save_word(&list, "dotcraft").unwrap();
        save_word(&list, "Roslynator").unwrap();
        assert_eq!(std::fs::read_to_string(&list).unwrap(), "dotcraft\nroslynator\n");
        assert!(save_word(&list, "a.b").is_err());

        let settings = dir.path().join("App.sln.DotSettings");
        save_word(&settings, "Nme").unwrap();
        let text = std::fs::read_to_string(&settings).unwrap();
        assert!(text.starts_with("<wpf:ResourceDictionary") && text.ends_with("</wpf:ResourceDictionary>"));
        assert_eq!(parse_words(&settings, &text), ["nme"]);
        // A file Rider wrote keeps its other settings.
        let rider = "<wpf:ResourceDictionary xml:space=\"preserve\">\r\n\t<s:Boolean x:Key=\"/Default/CodeStyle/X/@EntryValue\">True</s:Boolean>\r\n\t<s:Boolean x:Key=\"/Default/UserDictionary/Words/=wrld/@EntryIndexedValue\">True</s:Boolean>\r\n\t<s:Boolean x:Key=\"/Default/UserDictionary/Words/=gone/@EntryIndexedValue\">False</s:Boolean></wpf:ResourceDictionary>";
        std::fs::write(&settings, rider).unwrap();
        save_word(&settings, "helo").unwrap();
        let text = std::fs::read_to_string(&settings).unwrap();
        assert!(text.contains("CodeStyle/X"));
        assert_eq!(parse_words(&settings, &text), ["wrld", "helo"]);
        std::fs::write(&settings, "not xml").unwrap();
        assert!(save_word(&settings, "helo").is_err());
    }

    #[test]
    fn eight_bit_dictionaries_are_read() {
        let dir = tempfile::tempdir().unwrap();
        let (aff, dic) = (dir.path().join("xx_XX.aff"), dir.path().join("xx_XX.dic"));
        std::fs::write(&aff, b"SET ISO8859-1\n").unwrap();
        std::fs::write(&dic, b"2\ncaf\xe9\nhello\n").unwrap();
        let d = load_dictionary(&aff, &dic).unwrap();
        assert!(d.check("café") && d.check("hello") && !d.check("helo"));
    }

    /// A sample C# file with three typos, and an `en_US` dictionary for it in `home`.
    pub fn sample(home: &Path, dir: &Path) -> PathBuf {
        let words = ["calculates", "the", "total", "of", "order", "applies", "discount", "to", "received", "amount", "price", "calculator", "apply", "describe", "receive", "pry", "fix", "orders"];
        let dictionaries = home.join("dictionaries");
        std::fs::create_dir_all(&dictionaries).unwrap();
        std::fs::write(dictionaries.join("en_US.aff"), "SET UTF-8\nTRY esianrtolcdugmphbyfvkwz\n").unwrap();
        std::fs::write(dictionaries.join("en_US.dic"), format!("{}\n{}\n", words.len(), words.join("\n"))).unwrap();
        let file = dir.join("PriceCalculator.cs");
        std::fs::write(
            &file,
            "namespace Shop.Core;\n\n/// <summary>Calculates the total of an order.</summary>\npublic class PriceCalculatr\n{\n    // Applies the discount to the recieved amount.\n    public decimal Apply(decimal amount, decimal discount) => amount - discount;\n\n    public string Describe() => \"Total pryce of the order\";\n}\n",
        )
        .unwrap();
        file
    }

    fn typos(app: &DotCraftApp) -> Vec<String> {
        app.docs.active_doc().map(|d| d.editor.spelling.iter().map(|w| w.word.clone()).collect()).unwrap_or_default()
    }

    fn put_caret(app: &mut DotCraftApp, word: &str) {
        let doc = app.docs.active_doc_mut().unwrap();
        let text = doc.editor.buffer.rope().to_string();
        let at = doc.editor.buffer.rope().byte_to_char(text.find(word).unwrap() + 1);
        doc.editor.set_selection(rc_text::Selection::point(at));
    }

    fn run_until(h: &mut egui_kittest::Harness<'static, DotCraftApp>, what: &str, done: impl Fn(&DotCraftApp) -> bool) {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(30) {
            h.run_steps(1);
            if done(h.state()) {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("timed out waiting for {what}");
    }

    #[test]
    fn typos_are_underlined_fixed_and_saved_in_the_app() {
        let _lock = crate::screens::HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home = tempfile::tempdir().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let file = sample(home.path(), dir.path());
        let mut app = crate::screens::test_app(home.path(), true);
        app.open_file(&file);
        let mut h = crate::screens::harness(app, egui::vec2(1000.0, 700.0));
        run_until(&mut h, "typos", |a| typos(a).len() == 3);
        assert_eq!(typos(h.state()), ["Calculatr", "recieved", "pryce"]);
        assert!(h.state().docs.active_doc().unwrap().editor.spelling[0].identifier);

        // ⌥⏎ on a typo waits for its suggestions, then shows its fixes.
        put_caret(h.state_mut(), "recieved");
        h.state_mut().run_insight_action(rc_editor::Action::ShowIntentions, egui::pos2(300.0, 300.0));
        run_until(&mut h, "context actions", |a| matches!(a.dialog, Some(crate::ui::dialogs::Dialog::CodeActions { .. })));
        let Some(crate::ui::dialogs::Dialog::CodeActions { list, .. }) = h.state_mut().dialog.take() else { unreachable!() };
        let fixes: Vec<SpellingFix> = list.iter().filter_map(fix_of).collect();
        let titles: Vec<String> = fixes.iter().map(SpellingFix::title).collect();
        assert_eq!(titles.first().map(String::as_str), Some("Change to 'received'"), "{titles:?}");
        assert_eq!(titles.last().map(String::as_str), Some("Save 'recieved' to the user dictionary"));
        apply(h.state_mut(), fixes[0].clone());
        assert!(h.state().docs.active_doc().unwrap().editor.buffer.rope().to_string().contains("the received amount"));
        run_until(&mut h, "recheck", |a| typos(a) == ["Calculatr", "pryce"]);

        // A declared name without the language server: only the dictionaries are offered.
        put_caret(h.state_mut(), "Calculatr");
        let (fixes, ready) = fixes_at_caret(h.state_mut()).unwrap();
        assert!(ready);
        assert_eq!(fixes, [SpellingFix::Save { word: "Calculatr".into(), solution: false }]);

        // Saving a word removes its underline and writes the user dictionary.
        put_caret(h.state_mut(), "pryce");
        apply(h.state_mut(), SpellingFix::Save { word: "pryce".into(), solution: false });
        run_until(&mut h, "saved word", |a| typos(a) == ["Calculatr"]);
        assert_eq!(std::fs::read_to_string(home.path().join("spelling/user-dictionary.txt")).unwrap(), "pryce\n");
        h.run_steps(20);
        assert_eq!(typos(h.state()), ["Calculatr"]);

        // Turned off: no underlines.
        h.state_mut().settings.spellcheck_enabled = false;
        h.run_steps(2);
        assert!(typos(h.state()).is_empty());
    }
}
