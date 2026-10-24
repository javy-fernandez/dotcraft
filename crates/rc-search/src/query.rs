//! A text query shared by Find in Files, Replace in Files and the editor's find bar: plain
//! text or regex, case sensitivity and whole words, compiled once. Empty matches are skipped;
//! `^`/`$` work per line (also with CRLF).

use regex::bytes::{Captures, Regex, RegexBuilder};
use regex_syntax::hir::{Class, Hir, HirKind};
use std::fmt;
use std::ops::Range;

/// What to look for.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct QueryOptions {
    pub pattern: String,
    /// Treat `pattern` as a regular expression (Rust `regex` syntax).
    pub regex: bool,
    pub case_sensitive: bool,
    /// Whole words only: a letter, digit or `_` at an edge of the match must not touch
    /// another one (like `\b`).
    pub whole_word: bool,
}

impl QueryOptions {
    /// Plain, case-insensitive text.
    pub fn text(pattern: &str) -> QueryOptions {
        QueryOptions { pattern: pattern.to_string(), ..Default::default() }
    }
}

/// Why a query cannot be built.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QueryError {
    Empty,
    /// The regex does not compile; the text is the parser's (English) message.
    InvalidRegex(String),
}

impl fmt::Display for QueryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            QueryError::Empty => f.write_str("empty search pattern"),
            QueryError::InvalidRegex(e) => write!(f, "invalid regular expression: {e}"),
        }
    }
}

impl std::error::Error for QueryError {}

/// A compiled query. Cheap to clone.
#[derive(Clone, Debug)]
pub struct Query {
    options: QueryOptions,
    re: Regex,
    spans_lines: bool,
}

impl Query {
    pub fn new(options: QueryOptions) -> Result<Query, QueryError> {
        if options.pattern.is_empty() {
            return Err(QueryError::Empty);
        }
        let src = if options.regex { options.pattern.clone() } else { regex::escape(&options.pattern) };
        let re = RegexBuilder::new(&src)
            .case_insensitive(!options.case_sensitive)
            .multi_line(true)
            .crlf(true)
            .size_limit(64 << 20)
            .build()
            .map_err(|e| QueryError::InvalidRegex(e.to_string()))?;
        let spans_lines = regex_syntax::ParserBuilder::new()
            .case_insensitive(!options.case_sensitive)
            .multi_line(true)
            .crlf(true)
            .build()
            .parse(&src)
            .map_or(true, |h| can_match_newline(&h));
        Ok(Query { options, re, spans_lines })
    }

    pub fn options(&self) -> &QueryOptions {
        &self.options
    }

    /// True when a match may contain a line break (then searches read whole files).
    pub fn may_span_lines(&self) -> bool {
        self.spans_lines
    }

    /// The next non-empty match starting at byte `at` or later (byte range).
    pub fn find_at(&self, hay: &[u8], mut at: usize) -> Option<Range<usize>> {
        while at <= hay.len() {
            let m = self.re.find_at(hay, at)?;
            if self.accept(hay, m.start(), m.end()) {
                return Some(m.range());
            }
            at = next_char(hay, m.start());
        }
        None
    }

    /// Like [`find_at`](Self::find_at) with capture groups (for replacements).
    pub(crate) fn captures_at<'h>(&self, hay: &'h [u8], mut at: usize) -> Option<Captures<'h>> {
        while at <= hay.len() {
            let caps = self.re.captures_at(hay, at)?;
            let m = caps.get(0)?;
            if self.accept(hay, m.start(), m.end()) {
                return Some(caps);
            }
            at = next_char(hay, m.start());
        }
        None
    }

    /// All matches as byte ranges.
    pub fn find_bytes(&self, hay: &[u8]) -> Vec<Range<usize>> {
        let mut out = Vec::new();
        let mut at = 0;
        while let Some(m) = self.find_at(hay, at) {
            at = m.end;
            out.push(m);
        }
        out
    }

    /// All matches in `text` as char ranges (the editor's find bar).
    pub fn find_all(&self, text: &str) -> Vec<Range<usize>> {
        let mut conv = CharOffsets::new(text);
        self.find_bytes(text.as_bytes()).into_iter().map(|r| conv.at(r.start)..conv.at(r.end)).collect()
    }

    /// The text that replaces one match: `$1`, `${name}` and `$$` expand in regex mode, as do
    /// `\n`, `\t`, `\r`, `\\` and `\$`; plain mode inserts `replacement` as is.
    pub(crate) fn expand(&self, caps: &Captures<'_>, replacement: &str, dst: &mut Vec<u8>) {
        if self.options.regex {
            caps.expand(unescape_replacement(replacement).as_bytes(), dst);
        } else {
            dst.extend_from_slice(replacement.as_bytes());
        }
    }

    fn accept(&self, hay: &[u8], start: usize, end: usize) -> bool {
        if start == end {
            return false;
        }
        if !self.options.whole_word {
            return true;
        }
        // Like `\b`: a word character at an edge of the match must not touch another one.
        let word = |c: Option<char>| c.is_some_and(is_word);
        !(word(char_at(hay, start)) && word(char_before(hay, start))) && !(word(char_before(hay, end)) && word(char_at(hay, end)))
    }
}

/// Translates Java-style escapes in a regex replacement into `regex`'s expansion syntax.
fn unescape_replacement(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            Some('$') => out.push_str("$$"),
            Some(o) => {
                out.push('\\');
                out.push(o);
            }
            None => out.push('\\'),
        }
    }
    out
}

fn can_match_newline(h: &Hir) -> bool {
    match h.kind() {
        HirKind::Empty | HirKind::Look(_) => false,
        HirKind::Literal(l) => l.0.contains(&b'\n'),
        HirKind::Class(Class::Unicode(c)) => c.ranges().iter().any(|r| r.start() <= '\n' && '\n' <= r.end()),
        HirKind::Class(Class::Bytes(c)) => c.ranges().iter().any(|r| r.start() <= b'\n' && b'\n' <= r.end()),
        HirKind::Repetition(r) => can_match_newline(&r.sub),
        HirKind::Capture(c) => can_match_newline(&c.sub),
        HirKind::Concat(v) | HirKind::Alternation(v) => v.iter().any(can_match_newline),
    }
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn utf8_width(b: u8) -> usize {
    match b {
        0xF0.. => 4,
        0xE0.. => 3,
        0xC0.. => 2,
        _ => 1,
    }
}

/// Byte position of the char after the one at `i` (or `i + 1` at the end).
fn next_char(hay: &[u8], i: usize) -> usize {
    i + hay.get(i).map_or(1, |&b| utf8_width(b))
}

fn char_at(hay: &[u8], i: usize) -> Option<char> {
    let b = *hay.get(i)?;
    let end = (i + utf8_width(b)).min(hay.len());
    std::str::from_utf8(&hay[i..end]).ok()?.chars().next()
}

fn char_before(hay: &[u8], i: usize) -> Option<char> {
    let mut s = i.checked_sub(1)?;
    while s > 0 && i - s < 4 && hay[s] & 0xC0 == 0x80 {
        s -= 1;
    }
    std::str::from_utf8(&hay[s..i]).ok()?.chars().next()
}

/// Converts increasing byte offsets of a string into char offsets in one pass.
pub(crate) struct CharOffsets<'a> {
    text: &'a str,
    byte: usize,
    chars: usize,
}

impl<'a> CharOffsets<'a> {
    pub(crate) fn new(text: &'a str) -> Self {
        CharOffsets { text, byte: 0, chars: 0 }
    }

    /// Char offset of byte `b` (must not decrease between calls).
    pub(crate) fn at(&mut self, b: usize) -> usize {
        debug_assert!(b >= self.byte);
        self.chars += self.text.get(self.byte..b).map_or(0, |s| s.chars().count());
        self.byte = b;
        self.chars
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(pattern: &str, regex: bool, case_sensitive: bool, whole_word: bool) -> Query {
        Query::new(QueryOptions { pattern: pattern.into(), regex, case_sensitive, whole_word }).unwrap()
    }

    #[test]
    fn plain_text_is_literal_and_case_insensitive_by_default() {
        let query = q("a.b", false, false, false);
        assert_eq!(query.find_all("A.B axb a.b"), [0..3, 8..11]);
        assert!(!query.may_span_lines());
    }

    #[test]
    fn case_sensitivity() {
        assert_eq!(q("Foo", false, true, false).find_all("foo Foo FOO"), vec![4..7]);
        assert_eq!(q("Foo", false, false, false).find_all("foo Foo FOO").len(), 3);
    }

    #[test]
    fn whole_words() {
        let query = q("foo", false, false, true);
        assert_eq!(query.find_all("foo foobar _foo foo() xfoo foo"), [0..3, 16..19, 27..30]);
        // A rejected candidate does not hide a later one starting inside it.
        assert_eq!(q("aa", false, false, true).find_all("aaa aa"), vec![4..6]);
        // Non-word edges of the pattern are fine.
        assert_eq!(q("(x)", false, false, true).find_all("f(x) (x)"), [1..4, 5..8]);
        // Unicode letters count as word characters.
        assert_eq!(q("año", false, false, true).find_all("año años"), vec![0..3]);
    }

    #[test]
    fn regex_mode_and_offsets_in_chars() {
        let query = q(r"\bv\w+", true, true, false);
        assert_eq!(query.find_all("ñ var válido"), [2..5, 6..12]);
        assert!(!query.may_span_lines());
        assert!(q(r"a\sb", true, false, false).may_span_lines());
        assert!(q(r"a[^x]b", true, false, false).may_span_lines());
        assert!(!q(r"a.b", true, false, false).may_span_lines());
        assert!(q(r"(?s)a.b", true, false, false).may_span_lines());
    }

    #[test]
    fn anchors_work_per_line_with_crlf() {
        assert_eq!(q(r"^\w+$", true, false, false).find_all("ab\r\ncd\nef"), [0..2, 4..6, 7..9]);
    }

    #[test]
    fn empty_matches_are_skipped() {
        assert_eq!(q("x*", true, false, false).find_all("ab xx c"), vec![3..5]);
    }

    #[test]
    fn errors() {
        assert_eq!(Query::new(QueryOptions::text("")).unwrap_err(), QueryError::Empty);
        assert!(matches!(Query::new(QueryOptions { pattern: "(".into(), regex: true, ..Default::default() }), Err(QueryError::InvalidRegex(_))));
    }

    #[test]
    fn replacement_escapes() {
        assert_eq!(unescape_replacement(r"$1\n\$\\x\q"), "$1\n$$\\x\\q");
    }
}
