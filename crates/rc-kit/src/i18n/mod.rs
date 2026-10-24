//! Interface translations.
//!
//! The source language is English: every user-facing string is written in English in the
//! code and wrapped in [`tr`] (plain text) or [`trf`] (text with `{name}` placeholders).
//! The active language is global, so the worker thread can translate too.
//!
//! Translations live in `locales/<code>.json`, embedded in the binary: one object per area of
//! the code, each mapping the exact English text to its translation. A missing entry falls
//! back to English. `es.json` is the reference: every other locale has the same keys.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt::Display;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum Lang {
    #[default]
    En,
    Es,
    Fr,
    Pt,
    De,
    Zh,
    Ja,
    Pl,
}

impl Lang {
    pub const ALL: [Lang; 8] = [Lang::En, Lang::Es, Lang::Fr, Lang::Pt, Lang::De, Lang::Pl, Lang::Zh, Lang::Ja];

    /// Name of the language written in that language (for the selector).
    pub fn native_name(self) -> &'static str {
        match self {
            Lang::En => "English",
            Lang::Es => "Español",
            Lang::Fr => "Français",
            Lang::Pt => "Português (Brasil)",
            Lang::De => "Deutsch",
            Lang::Zh => "简体中文",
            Lang::Ja => "日本語",
            Lang::Pl => "Polski",
        }
    }

    /// The embedded `locales/*.json` (None for English, the source language).
    fn source(self) -> Option<&'static str> {
        match self {
            Lang::En => None,
            Lang::Es => Some(include_str!("../../../../locales/es.json")),
            Lang::Fr => Some(include_str!("../../../../locales/fr.json")),
            Lang::Pt => Some(include_str!("../../../../locales/pt-BR.json")),
            Lang::De => Some(include_str!("../../../../locales/de.json")),
            Lang::Zh => Some(include_str!("../../../../locales/zh-Hans.json")),
            Lang::Ja => Some(include_str!("../../../../locales/ja.json")),
            Lang::Pl => Some(include_str!("../../../../locales/pl.json")),
        }
    }

    /// Language of the operating system, English if it isn't one of ours.
    pub fn system() -> Lang {
        sys_locale::get_locale().map_or(Lang::En, |l| Lang::from_locale(&l))
    }

    /// `es-ES`, `pt_BR`, `zh-Hans-CN`… → its language. Chinese is always Simplified.
    pub fn from_locale(locale: &str) -> Lang {
        let primary = locale.split(['-', '_']).next().unwrap_or("").to_ascii_lowercase();
        match primary.as_str() {
            "es" => Lang::Es,
            "fr" => Lang::Fr,
            "pt" => Lang::Pt,
            "de" => Lang::De,
            "zh" => Lang::Zh,
            "ja" => Lang::Ja,
            "pl" => Lang::Pl,
            _ => Lang::En,
        }
    }

    fn index(self) -> usize {
        Lang::ALL.iter().position(|&l| l == self).unwrap_or(0)
    }
}

static LANG: AtomicU8 = AtomicU8::new(0);

pub fn set_lang(lang: Lang) {
    LANG.store(lang.index() as u8, Ordering::Relaxed);
}

pub fn lang() -> Lang {
    Lang::ALL.get(LANG.load(Ordering::Relaxed) as usize).copied().unwrap_or_default()
}

type Table = HashMap<&'static str, &'static str>;

/// One area of a locale file: its name and its (English, translation) entries, in file order.
type Area = (String, Vec<(String, String)>);

/// Areas of a locale file, in file order.
fn parse(source: &str) -> serde_json::Result<Vec<Area>> {
    let areas: serde_json::Map<String, serde_json::Value> = serde_json::from_str(source)?;
    Ok(areas
        .into_iter()
        .map(|(area, entries)| {
            let entries = match entries {
                serde_json::Value::Object(m) => {
                    m.into_iter().filter_map(|(k, v)| Some((k, v.as_str()?.to_string()))).collect()
                }
                _ => Vec::new(),
            };
            (area, entries)
        })
        .collect())
}

/// The translations of `lang`, parsed once (the strings live for the whole run).
fn table(lang: Lang) -> &'static Table {
    static TABLES: [OnceLock<Table>; Lang::ALL.len()] = [const { OnceLock::new() }; Lang::ALL.len()];
    TABLES[lang.index()].get_or_init(|| {
        let Some(source) = lang.source() else { return Table::new() };
        let areas = parse(source).unwrap_or_else(|e| {
            eprintln!("invalid locale {lang:?}: {e}");
            Vec::new()
        });
        let leak = |s: String| -> &'static str { Box::leak(s.into_boxed_str()) };
        areas.into_iter().flat_map(|(_, entries)| entries).map(|(k, v)| (leak(k), leak(v))).collect()
    })
}

/// `en` in the active language.
pub fn tr(en: &'static str) -> &'static str {
    match lang() {
        Lang::En => en,
        l => table(l).get(en).copied().unwrap_or(en),
    }
}

/// Like [`tr`] for words whose translation depends on where they appear (e.g. the "Edit"
/// menu is «Edición» but the "Edit" action is «Editar»). The table key is
/// `"{context}|{en}"`; without an entry the plain translation of `en` is used.
pub fn tr_ctx(context: &'static str, en: &'static str) -> &'static str {
    match lang() {
        Lang::En => en,
        l => {
            let key = format!("{context}|{en}");
            table(l).get(key.as_str()).copied().unwrap_or_else(|| tr(en))
        }
    }
}

/// Like [`tr`], replacing each `{name}` placeholder with its value:
/// `trf("{n} rows", &[("n", &count)])`.
pub fn trf(template: &'static str, args: &[(&str, &dyn Display)]) -> String {
    let mut out = tr(template).to_string();
    for (name, value) in args {
        out = out.replace(&format!("{{{name}}}"), &value.to_string());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeSet, HashSet};

    fn placeholders(s: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut rest = s;
        while let Some(i) = rest.find('{') {
            let Some(j) = rest[i..].find('}') else { break };
            out.push(rest[i..i + j + 1].to_string());
            rest = &rest[i + j + 1..];
        }
        out.sort();
        out
    }

    fn entries(lang: Lang) -> Vec<(String, String)> {
        let source = lang.source().unwrap();
        let areas = parse(source).unwrap_or_else(|e| panic!("{lang:?}: {e}"));
        // One entry per line: a key repeated inside an area (JSON keeps the last) shows up here.
        let lines = source.lines().filter(|l| l.starts_with("    \"")).count();
        let entries: Vec<_> = areas.into_iter().flat_map(|(_, e)| e).collect();
        assert_eq!(lines, entries.len(), "{lang:?}: a key is repeated inside an area");
        entries
    }

    #[test]
    fn keys_are_unique_and_every_locale_has_the_reference_keys() {
        let reference: BTreeSet<String> = entries(Lang::Es).into_iter().map(|(k, _)| k).collect();
        for lang in Lang::ALL.into_iter().filter(|&l| l != Lang::En) {
            let mut seen = HashSet::new();
            let entries = entries(lang);
            for (en, _) in &entries {
                assert!(seen.insert(en.clone()), "{lang:?}: duplicated translation key {en:?}");
            }
            let keys: BTreeSet<String> = entries.into_iter().map(|(k, _)| k).collect();
            let missing: Vec<_> = reference.difference(&keys).collect();
            let extra: Vec<_> = keys.difference(&reference).collect();
            assert!(missing.is_empty() && extra.is_empty(), "{lang:?}: missing {missing:?}, extra {extra:?}");
        }
    }

    #[test]
    fn translations_keep_placeholders() {
        for lang in Lang::ALL.into_iter().filter(|&l| l != Lang::En) {
            for (en, text) in entries(lang) {
                let en_text = en.split_once('|').map_or(en.as_str(), |(_, t)| t);
                assert_eq!(placeholders(en_text), placeholders(&text), "{lang:?}: placeholders differ in {en:?}");
                assert!(!text.trim().is_empty(), "{lang:?}: empty translation for {en:?}");
            }
        }
    }

    /// Rust string literal starting right after its opening quote: (text, rest after the
    /// closing quote). Only the escapes the UI texts use.
    fn literal(s: &str) -> Option<(String, &str)> {
        let mut out = String::new();
        let mut chars = s.char_indices().peekable();
        while let Some((i, c)) = chars.next() {
            match c {
                '"' => return Some((out, &s[i + 1..])),
                '\\' => match chars.next()?.1 {
                    'n' => out.push('\n'),
                    't' => out.push('\t'),
                    // Line continuation: the newline and the next line's indentation are skipped.
                    '\n' => {
                        while chars.next_if(|(_, c)| c.is_whitespace()).is_some() {}
                    }
                    other => out.push(other),
                },
                c => out.push(c),
            }
        }
        None
    }

    /// Every `tr("…")`, `trf("…")` and `tr_ctx("…", "…")` in the code has an entry: a text
    /// left out of the locale files would show in English in every language.
    #[test]
    fn every_text_in_the_code_has_a_translation() {
        let keys: HashSet<String> = entries(Lang::Es).into_iter().map(|(k, _)| k).collect();
        let mut missing = BTreeSet::new();
        // Every crate of the workspace translates through this module.
        let crates = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/.."));
        let mut dirs: Vec<std::path::PathBuf> =
            std::fs::read_dir(crates).unwrap().flatten().map(|e| e.path().join("src")).filter(|p| p.is_dir()).collect();
        while let Some(dir) = dirs.pop() {
            for entry in std::fs::read_dir(&dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    dirs.push(path);
                    continue;
                }
                if path.extension().is_none_or(|e| e != "rs") {
                    continue;
                }
                let source = std::fs::read_to_string(&path).unwrap();
                let code: String = source
                    .split("#[cfg(test)]\nmod ")
                    .next()
                    .unwrap_or("")
                    .lines()
                    .filter(|l| !l.trim_start().starts_with("//"))
                    .map(|l| format!("{l}\n"))
                    .collect();
                for call in ["tr(", "trf(", "tr_ctx("] {
                    for (at, _) in code.match_indices(call) {
                        if code[..at].chars().next_back().is_some_and(|c| c.is_alphanumeric() || c == '_') {
                            continue;
                        }
                        let Some(rest) = code[at + call.len()..].trim_start().strip_prefix('"') else { continue };
                        let Some((first, rest)) = literal(rest) else { continue };
                        let key = if call == "tr_ctx(" {
                            let Some(rest) = rest.trim_start().strip_prefix(',').map(str::trim_start).and_then(|r| r.strip_prefix('"')) else { continue };
                            let Some((en, _)) = literal(rest) else { continue };
                            if keys.contains(&format!("{first}|{en}")) { continue }
                            en
                        } else {
                            first
                        };
                        if !keys.contains(&key) {
                            missing.insert(format!("{}: {key:?}", path.display()));
                        }
                    }
                }
            }
        }
        assert!(missing.is_empty(), "texts without translation:\n{}", missing.into_iter().collect::<Vec<_>>().join("\n"));
    }

    #[test]
    fn system_locales_map_to_languages() {
        for (locale, lang) in [
            ("es-ES", Lang::Es),
            ("es_419", Lang::Es),
            ("fr-CA", Lang::Fr),
            ("pt-BR", Lang::Pt),
            ("pt_PT", Lang::Pt),
            ("de-AT", Lang::De),
            ("zh-Hans-CN", Lang::Zh),
            ("zh-TW", Lang::Zh),
            ("ja-JP", Lang::Ja),
            ("pl-PL", Lang::Pl),
            ("en-US", Lang::En),
            ("ko-KR", Lang::En),
            ("", Lang::En),
        ] {
            assert_eq!(Lang::from_locale(locale), lang, "{locale}");
        }
    }

    #[test]
    fn falls_back_to_english() {
        // Tests never change the global language, so it is the default (English) here.
        assert_eq!(trf("{a} of {b} (untranslated)", &[("a", &1), ("b", &"x")]), "1 of x (untranslated)");
    }
}
