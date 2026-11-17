//! Offline spelling analysis for source files and commit messages.

use rc_syntax::{Hl, Lang, highlight_lines, parse_now};
use rc_text::Rope;
use spellbook::Dictionary;
use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, mpsc};

use rc_core::{Inbox, Waker};

enum Request {
    Discover,
    Check {
        generation: u64,
        doc: u64,
        version: u64,
        source_language: Lang,
        language: String,
        text: String,
        project: Option<PathBuf>,
    },
    CheckCommit {
        generation: u64,
        version: u64,
        language: String,
        text: String,
        project: Option<PathBuf>,
    },
    AddWord {
        language: String,
        word: String,
        project: Option<PathBuf>,
        project_scope: bool,
    },
}

pub enum Reply {
    Languages(Vec<String>),
    Checked {
        doc: u64,
        version: u64,
        words: Vec<Misspelling>,
    },
    CheckedCommit {
        version: u64,
        words: Vec<Misspelling>,
    },
    Added {
        word: String,
        error: Option<String>,
    },
    MissingDictionary {
        language: String,
    },
}

pub struct SpellingService {
    requests: mpsc::Sender<Request>,
    replies: Inbox<Reply>,
    generation: Arc<AtomicU64>,
    commit_generation: Arc<AtomicU64>,
}

impl SpellingService {
    pub fn new(waker: Waker) -> Self {
        let replies = Inbox::new(waker);
        let out = replies.sender();
        let generation = Arc::new(AtomicU64::new(0));
        let latest = generation.clone();
        let commit_generation = Arc::new(AtomicU64::new(0));
        let latest_commit = commit_generation.clone();
        let (requests, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut loaded: std::collections::HashMap<String, Dictionary> =
                std::collections::HashMap::new();
            while let Ok(request) = rx.recv() {
                let language = match &request {
                    Request::Discover => {
                        out.send(Reply::Languages(
                            available_dictionaries()
                                .into_iter()
                                .map(|(name, _, _)| name)
                                .collect(),
                        ));
                        continue;
                    }
                    Request::Check { language, .. }
                    | Request::CheckCommit { language, .. }
                    | Request::AddWord { language, .. } => language,
                };
                if !loaded.contains_key(language) {
                    let pair = available_dictionaries()
                        .into_iter()
                        .find(|(name, _, _)| name == language);
                    if let Some((_, aff, dic)) = pair
                        && let Ok(dictionary) = load_dictionary(&aff, &dic)
                    {
                        loaded.insert(language.clone(), dictionary);
                    }
                }
                let Some(dictionary) = loaded.get(language) else {
                    out.send(Reply::MissingDictionary {
                        language: language.clone(),
                    });
                    continue;
                };
                match request {
                    Request::Discover => {}
                    Request::Check {
                        generation,
                        doc,
                        version,
                        source_language,
                        language,
                        text,
                        project,
                    } => {
                        if latest.load(Ordering::Relaxed) != generation {
                            continue;
                        }
                        let mut paths = Vec::new();
                        if let Some(path) = user_words_path(&language) {
                            paths.push(path);
                        }
                        if let Some(root) = project.as_deref()
                            && let Some(path) = project_words_path(root, &language)
                        {
                            paths.push(path);
                        }
                        let custom = read_custom_words(&paths);
                        let words = check_source_cancelled(
                            &text,
                            source_language,
                            dictionary,
                            &custom,
                            || latest.load(Ordering::Relaxed) != generation,
                        );
                        if latest.load(Ordering::Relaxed) == generation {
                            out.send(Reply::Checked {
                                doc,
                                version,
                                words,
                            });
                        }
                    }
                    Request::CheckCommit {
                        generation,
                        version,
                        language,
                        text,
                        project,
                    } => {
                        if latest_commit.load(Ordering::Relaxed) != generation {
                            continue;
                        }
                        let mut paths = Vec::new();
                        if let Some(path) = user_words_path(&language) {
                            paths.push(path);
                        }
                        if let Some(root) = project.as_deref()
                            && let Some(path) = project_words_path(root, &language)
                        {
                            paths.push(path);
                        }
                        let words =
                            check_commit_message(&text, dictionary, &read_custom_words(&paths));
                        if latest_commit.load(Ordering::Relaxed) == generation {
                            out.send(Reply::CheckedCommit { version, words });
                        }
                    }
                    Request::AddWord {
                        language,
                        word,
                        project,
                        project_scope,
                    } => {
                        let path = if project_scope {
                            project
                                .as_deref()
                                .and_then(|root| project_words_path(root, &language))
                        } else {
                            user_words_path(&language)
                        };
                        let error = path.map_or_else(
                            || Some("Dictionary path is unavailable".to_string()),
                            |path| add_custom_word(&path, &word).err(),
                        );
                        out.send(Reply::Added { word, error });
                    }
                }
            }
        });
        let service = Self {
            requests,
            replies,
            generation,
            commit_generation,
        };
        let _ = service.requests.send(Request::Discover);
        service
    }

    pub fn check(
        &self,
        doc: u64,
        version: u64,
        source_language: Lang,
        language: &str,
        text: String,
        project: Option<PathBuf>,
    ) {
        let generation = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        let _ = self.requests.send(Request::Check {
            generation,
            doc,
            version,
            source_language,
            language: language.to_string(),
            text,
            project,
        });
    }

    pub fn check_commit(
        &self,
        version: u64,
        language: &str,
        text: String,
        project: Option<PathBuf>,
    ) {
        let generation = self.commit_generation.fetch_add(1, Ordering::Relaxed) + 1;
        let _ = self.requests.send(Request::CheckCommit {
            generation,
            version,
            language: language.to_string(),
            text,
            project,
        });
    }

    pub fn add_word(
        &self,
        language: &str,
        word: &str,
        project: Option<PathBuf>,
        project_scope: bool,
    ) {
        let _ = self.requests.send(Request::AddWord {
            language: language.to_string(),
            word: word.to_string(),
            project,
            project_scope,
        });
    }

    pub fn drain(&self) -> Vec<Reply> {
        self.replies.drain()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceKind {
    Comment,
    String,
    Identifier,
    Text,
    CommitMessage,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Misspelling {
    pub start: usize,
    pub end: usize,
    pub word: String,
    pub kind: SourceKind,
    pub suggestions: Vec<String>,
}

/// Installed Hunspell pairs. A caller should run discovery and loading on a worker thread.
pub fn available_dictionaries() -> Vec<(String, PathBuf, PathBuf)> {
    let mut dirs = Vec::new();
    if let Some(home) = dirs::data_dir() {
        dirs.push(home.join("hunspell"));
        dirs.push(home.join("myspell"));
    }
    if let Some(paths) = std::env::var_os("DOTCRAFT_DICTIONARIES") {
        dirs.extend(std::env::split_paths(&paths));
    }
    dirs.extend([
        PathBuf::from("/usr/share/hunspell"),
        PathBuf::from("/usr/share/myspell"),
        PathBuf::from("/usr/local/share/hunspell"),
    ]);
    let mut found = Vec::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let aff = entry.path();
            if aff.extension().is_none_or(|ext| ext != "aff") {
                continue;
            }
            let dic = aff.with_extension("dic");
            if dic.is_file() {
                let locale = aff
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                if !found.iter().any(|(name, _, _)| name == &locale) {
                    found.push((locale, aff, dic));
                }
            }
        }
    }
    found.sort_by(|a, b| a.0.cmp(&b.0));
    found
}

pub fn load_dictionary(aff: &Path, dic: &Path) -> Result<Dictionary, String> {
    let aff = std::fs::read_to_string(aff).map_err(|e| e.to_string())?;
    let dic = std::fs::read_to_string(dic).map_err(|e| e.to_string())?;
    Dictionary::new(&aff, &dic).map_err(|e| e.to_string())
}

pub fn suggestions(dictionary: &Dictionary, word: &str) -> Vec<String> {
    let mut suggestions = Vec::new();
    dictionary.suggest(word, &mut suggestions);
    suggestions.truncate(8);
    suggestions
}

pub fn user_words_path(locale: &str) -> Option<PathBuf> {
    let locale = safe_locale(locale)?;
    Some(
        dirs::config_dir()?
            .join("dotcraft")
            .join("spelling")
            .join(format!("{locale}.txt")),
    )
}

pub fn project_words_path(root: &Path, locale: &str) -> Option<PathBuf> {
    let locale = safe_locale(locale)?;
    Some(
        root.join(".dotcraft")
            .join("spelling")
            .join(format!("{locale}.txt")),
    )
}

fn safe_locale(locale: &str) -> Option<&str> {
    (!locale.is_empty()
        && locale
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'))
    .then_some(locale)
}

pub fn read_custom_words(paths: &[PathBuf]) -> HashSet<String> {
    paths
        .iter()
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .flat_map(|content| content.lines().map(str::to_lowercase).collect::<Vec<_>>())
        .filter(|word| !word.is_empty())
        .collect()
}

/// Save a custom word. The caller must do this on a worker thread.
pub fn add_custom_word(path: &Path, word: &str) -> Result<(), String> {
    let word = word.trim().to_lowercase();
    if word.len() < 2 || !word.chars().all(char::is_alphabetic) {
        return Err("Only alphabetic words can be added to a dictionary".into());
    }
    if read_custom_words(&[path.to_path_buf()]).contains(&word) {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    writeln!(file, "{word}").map_err(|e| e.to_string())
}

pub fn check_commit_message(
    text: &str,
    dictionary: &Dictionary,
    custom: &HashSet<String>,
) -> Vec<Misspelling> {
    let chars: Vec<char> = text.chars().collect();
    let mut result = Vec::new();
    scan_natural(
        &chars,
        0,
        chars.len(),
        0,
        SourceKind::CommitMessage,
        dictionary,
        custom,
        &mut result,
    );
    result
}

/// Parses and checks a snapshot. Parsing and dictionary lookup must run off the UI thread.
#[cfg(test)]
fn check_csharp(text: &str, dictionary: &Dictionary, custom: &HashSet<String>) -> Vec<Misspelling> {
    check_source_cancelled(text, Lang::CSharp, dictionary, custom, || false)
}

fn check_source_cancelled(
    text: &str,
    source_language: Lang,
    dictionary: &Dictionary,
    custom: &HashSet<String>,
    cancelled: impl Fn() -> bool,
) -> Vec<Misspelling> {
    let rope = Rope::from_str(text);
    let spans = parse_now(source_language, &rope, None)
        .map(|tree| highlight_lines(source_language, &tree, &rope, 0, rope.len_lines()))
        .unwrap_or_else(|| vec![Vec::new(); rope.len_lines()]);
    let mut result = Vec::new();
    for (line, line_spans) in spans.iter().enumerate() {
        if cancelled() {
            return Vec::new();
        }
        let chars: Vec<char> = rope.line(line).chars().collect();
        let mut roles = vec![0u8; chars.len()];
        for span in line_spans {
            let role = match span.hl {
                Hl::Comment | Hl::DocComment => 1,
                Hl::String => 2,
                Hl::Keyword | Hl::Preproc | Hl::Tag | Hl::XmlAttr | Hl::Code | Hl::Link => 3,
                _ => 0,
            };
            for cell in roles
                .iter_mut()
                .take(span.end.min(chars.len()))
                .skip(span.start)
            {
                *cell = role;
            }
        }
        let base = rope.line_to_char(line);
        let mut cursor = 0;
        while cursor < chars.len() {
            let role = roles[cursor];
            let end = (cursor + 1..chars.len())
                .find(|&i| roles[i] != role)
                .unwrap_or(chars.len());
            match role {
                1 => scan_natural(
                    &chars,
                    cursor,
                    end,
                    base,
                    SourceKind::Comment,
                    dictionary,
                    custom,
                    &mut result,
                ),
                2 => scan_natural(
                    &chars,
                    cursor,
                    end,
                    base,
                    SourceKind::String,
                    dictionary,
                    custom,
                    &mut result,
                ),
                0 if matches!(
                    source_language,
                    Lang::CSharp | Lang::JavaScript | Lang::TypeScript | Lang::Tsx
                ) =>
                {
                    scan_identifiers(
                        &chars,
                        cursor,
                        end,
                        base,
                        source_language,
                        dictionary,
                        custom,
                        &mut result,
                    )
                }
                0 => scan_natural(
                    &chars,
                    cursor,
                    end,
                    base,
                    SourceKind::Text,
                    dictionary,
                    custom,
                    &mut result,
                ),
                _ => {}
            }
            cursor = end;
        }
    }
    result
}

fn scan_natural(
    chars: &[char],
    start: usize,
    end: usize,
    base: usize,
    kind: SourceKind,
    dictionary: &Dictionary,
    custom: &HashSet<String>,
    out: &mut Vec<Misspelling>,
) {
    let mut i = start;
    while i < end {
        if !chars[i].is_alphabetic() {
            i += 1;
            continue;
        }
        let first = i;
        while i < end && chars[i].is_alphabetic() {
            i += 1;
        }
        record(chars, first, i, base, kind, dictionary, custom, out);
    }
}

fn scan_identifiers(
    chars: &[char],
    start: usize,
    end: usize,
    base: usize,
    source_language: Lang,
    dictionary: &Dictionary,
    custom: &HashSet<String>,
    out: &mut Vec<Misspelling>,
) {
    let mut i = start;
    while i < end {
        if !(chars[i].is_alphabetic() || chars[i] == '_') {
            i += 1;
            continue;
        }
        let first = i;
        while i < end && (chars[i].is_alphanumeric() || chars[i] == '_') {
            i += 1;
        }
        let token: String = chars[first..i].iter().collect();
        if (source_language == Lang::CSharp && CSHARP_KEYWORDS.contains(&token.as_str()))
            || (source_language != Lang::CSharp && JAVASCRIPT_KEYWORDS.contains(&token.as_str()))
        {
            continue;
        }
        let mut part = first;
        while part < i {
            while part < i && !chars[part].is_alphabetic() {
                part += 1;
            }
            if part >= i {
                break;
            }
            let mut next = part + 1;
            while next < i && chars[next].is_alphabetic() {
                let camel = chars[next].is_uppercase() && chars[next - 1].is_lowercase();
                let acronym = next + 1 < i
                    && chars[next].is_uppercase()
                    && chars[next - 1].is_uppercase()
                    && chars[next + 1].is_lowercase();
                if camel || acronym {
                    break;
                }
                next += 1;
            }
            record(
                chars,
                part,
                next,
                base,
                SourceKind::Identifier,
                dictionary,
                custom,
                out,
            );
            part = next;
        }
    }
}

fn record(
    chars: &[char],
    first: usize,
    end: usize,
    base: usize,
    kind: SourceKind,
    dictionary: &Dictionary,
    custom: &HashSet<String>,
    out: &mut Vec<Misspelling>,
) {
    if end - first < 3 {
        return;
    }
    let word: String = chars[first..end].iter().collect();
    let normalized = word.to_lowercase();
    if custom.contains(&normalized) || dictionary.check(&word) || dictionary.check(&normalized) {
        return;
    }
    let alternatives = suggestions(dictionary, &word).into_iter().take(5).collect();
    out.push(Misspelling {
        start: base + first,
        end: base + end,
        word,
        kind,
        suggestions: alternatives,
    });
}

const CSHARP_KEYWORDS: &[&str] = &[
    "abstract",
    "as",
    "async",
    "await",
    "base",
    "bool",
    "break",
    "byte",
    "case",
    "catch",
    "char",
    "checked",
    "class",
    "const",
    "continue",
    "decimal",
    "default",
    "delegate",
    "do",
    "double",
    "dynamic",
    "else",
    "enum",
    "event",
    "explicit",
    "extern",
    "false",
    "finally",
    "fixed",
    "float",
    "for",
    "foreach",
    "from",
    "get",
    "global",
    "goto",
    "if",
    "implicit",
    "in",
    "int",
    "interface",
    "internal",
    "is",
    "lock",
    "long",
    "namespace",
    "new",
    "null",
    "object",
    "operator",
    "out",
    "override",
    "params",
    "partial",
    "private",
    "protected",
    "public",
    "readonly",
    "record",
    "ref",
    "return",
    "sbyte",
    "sealed",
    "set",
    "short",
    "sizeof",
    "static",
    "string",
    "struct",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "typeof",
    "uint",
    "ulong",
    "unchecked",
    "unsafe",
    "ushort",
    "using",
    "var",
    "virtual",
    "void",
    "volatile",
    "while",
    "where",
    "with",
    "yield",
];

const JAVASCRIPT_KEYWORDS: &[&str] = &[
    "abstract",
    "any",
    "as",
    "async",
    "await",
    "boolean",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "constructor",
    "continue",
    "debugger",
    "declare",
    "default",
    "delete",
    "do",
    "else",
    "enum",
    "export",
    "extends",
    "false",
    "finally",
    "for",
    "from",
    "function",
    "get",
    "if",
    "implements",
    "import",
    "in",
    "instanceof",
    "interface",
    "let",
    "module",
    "namespace",
    "new",
    "null",
    "number",
    "of",
    "package",
    "private",
    "protected",
    "public",
    "readonly",
    "return",
    "set",
    "static",
    "string",
    "super",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "type",
    "typeof",
    "undefined",
    "unknown",
    "var",
    "void",
    "while",
    "with",
    "yield",
];

#[cfg(test)]
mod tests {
    use super::*;

    fn dictionary() -> Dictionary {
        Dictionary::new(
            "SET UTF-8\n",
            "8\nhello\nworld\nmessage\nname\nvalue\ncomment\nstring\ncorrect\n",
        )
        .unwrap()
    }

    #[test]
    fn checks_comments_strings_and_camel_case_identifiers() {
        let text = "// helo comment\nclass WrongNme { string value = \"wrld string\"; }\n";
        let found = check_csharp(text, &dictionary(), &HashSet::new());
        assert!(
            found
                .iter()
                .any(|m| m.word == "helo" && m.kind == SourceKind::Comment)
        );
        assert!(
            found
                .iter()
                .any(|m| m.word == "Nme" && m.kind == SourceKind::Identifier)
        );
        assert!(
            found
                .iter()
                .any(|m| m.word == "wrld" && m.kind == SourceKind::String)
        );
        assert!(
            !found
                .iter()
                .any(|m| m.word == "class" || m.word == "string")
        );
    }

    #[test]
    fn custom_words_apply_to_commit_messages() {
        let text = "Ship DotCraft mesage";
        let custom = HashSet::from(["dotcraft".to_string()]);
        let found = check_commit_message(text, &dictionary(), &custom);
        assert_eq!(
            found.iter().map(|m| m.word.as_str()).collect::<Vec<_>>(),
            ["Ship", "mesage"]
        );
        assert!(!suggestions(&dictionary(), "mesage").is_empty());
    }

    #[test]
    fn checks_javascript_and_markdown_without_reporting_keywords() {
        let javascript = "// helo comment\nconst wrongNme = \"wrld\";";
        let found = check_source_cancelled(
            javascript,
            Lang::JavaScript,
            &dictionary(),
            &HashSet::new(),
            || false,
        );
        assert!(
            found
                .iter()
                .any(|word| word.word == "helo" && word.kind == SourceKind::Comment)
        );
        assert!(
            found
                .iter()
                .any(|word| word.word == "Nme" && word.kind == SourceKind::Identifier)
        );
        assert!(
            found
                .iter()
                .any(|word| word.word == "wrld" && word.kind == SourceKind::String)
        );
        assert!(!found.iter().any(|word| word.word == "const"));
        let markdown = check_source_cancelled(
            "Helo world",
            Lang::Markdown,
            &dictionary(),
            &HashSet::new(),
            || false,
        );
        assert!(
            markdown
                .iter()
                .any(|word| word.word == "Helo" && word.kind == SourceKind::Text)
        );
    }

    #[test]
    fn custom_words_are_persistent_and_locale_paths_are_safe() {
        let dir = tempfile::tempdir().unwrap();
        let path = project_words_path(dir.path(), "pt_BR").unwrap();
        add_custom_word(&path, "DotCraft").unwrap();
        add_custom_word(&path, "dotcraft").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "dotcraft\n");
        assert!(read_custom_words(&[path]).contains("dotcraft"));
        assert!(project_words_path(dir.path(), "../escape").is_none());
    }

    #[test]
    fn worker_checks_with_an_installed_dictionary_without_network() {
        if !available_dictionaries()
            .iter()
            .any(|(language, _, _)| language == "en_US")
        {
            return;
        }
        let service = SpellingService::new(Waker::noop());
        service.check(7, 11, Lang::CSharp, "en_US", "// helo world".into(), None);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            for reply in service.drain() {
                if let Reply::Checked {
                    doc,
                    version,
                    words,
                } = reply
                {
                    assert_eq!((doc, version), (7, 11));
                    assert!(words.iter().any(|word| word.word == "helo"));
                    return;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        panic!("spelling worker did not return its result");
    }
}
