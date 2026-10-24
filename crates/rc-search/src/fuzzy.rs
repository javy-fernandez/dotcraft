//! Fuzzy matching shared by every ranked list (Go to File, actions, symbols, recent files):
//! `nucleo` scores the alignment and IntelliJ-like rules classify it, so an exact name beats a
//! prefix, a prefix beats a camel-hump match (`OrdSer` → `OrderService`) and those beat a
//! plain subsequence.

use nucleo_matcher::pattern::{Atom, AtomKind, CaseMatching, Normalization};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use std::cell::RefCell;

/// Points between two match kinds; nucleo's own score for a pattern stays well below it.
pub(crate) const KIND_STEP: u32 = 10_000;

/// How a candidate matched, from worst to best.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MatchKind {
    /// Pattern characters scattered inside words.
    Subsequence,
    /// Every run of matched characters starts a word or a camel hump.
    CamelHump,
    /// The candidate starts with the pattern (ignoring case).
    Prefix,
    /// The candidate is the pattern (ignoring case).
    Exact,
}

impl MatchKind {
    pub(crate) fn bonus(self) -> u32 {
        match self {
            MatchKind::Subsequence => 0,
            MatchKind::CamelHump => KIND_STEP,
            MatchKind::Prefix => 2 * KIND_STEP,
            MatchKind::Exact => 3 * KIND_STEP,
        }
    }
}

/// A successful match: higher `score` ranks first, `indices` are the matched char positions
/// (sorted, for highlighting).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FuzzyMatch {
    pub score: u32,
    pub indices: Vec<u32>,
    pub kind: MatchKind,
}

/// A prepared pattern. Whitespace separates words that must all match (in any order); case is
/// ignored.
#[derive(Clone, Debug)]
pub struct FuzzyPattern {
    atoms: Vec<Atom>,
    lower: String,
}

impl FuzzyPattern {
    pub fn new(pattern: &str) -> FuzzyPattern {
        let words: Vec<&str> = pattern.split_whitespace().collect();
        let atoms = words
            .iter()
            .map(|w| Atom::new(w, CaseMatching::Ignore, Normalization::Smart, AtomKind::Fuzzy, false))
            .collect();
        FuzzyPattern { atoms, lower: words.join(" ").to_lowercase() }
    }

    pub fn is_empty(&self) -> bool {
        self.atoms.is_empty()
    }

    /// The pattern lowercased, words joined by single spaces.
    pub fn text(&self) -> &str {
        &self.lower
    }
}

/// Reusable matcher (keeps nucleo's scratch memory between calls). Not `Sync`: keep one per
/// thread.
pub struct FuzzyMatcher {
    matcher: Matcher,
    buf: Vec<char>,
}

impl Default for FuzzyMatcher {
    fn default() -> Self {
        Self::new()
    }
}

impl FuzzyMatcher {
    /// A matcher for names (actions, symbols, file names).
    pub fn new() -> FuzzyMatcher {
        let mut config = Config::DEFAULT;
        config.prefer_prefix = true;
        FuzzyMatcher { matcher: Matcher::new(config), buf: Vec::new() }
    }

    /// A matcher tuned for `/`-separated paths (bonus after each `/`).
    pub fn for_paths() -> FuzzyMatcher {
        let mut config = Config::DEFAULT.match_paths();
        config.prefer_prefix = true;
        FuzzyMatcher { matcher: Matcher::new(config), buf: Vec::new() }
    }

    /// Matches `candidate` against `pattern`; `None` when some pattern word does not match.
    /// An empty pattern matches everything with score 0.
    pub fn matches(&mut self, candidate: &str, pattern: &FuzzyPattern) -> Option<FuzzyMatch> {
        if pattern.is_empty() {
            return Some(FuzzyMatch { score: 0, indices: Vec::new(), kind: MatchKind::Subsequence });
        }
        let FuzzyMatcher { matcher, buf } = self;
        let hay = Utf32Str::new(candidate, buf);
        let mut indices = Vec::with_capacity(pattern.lower.len());
        let mut base = 0u32;
        for atom in &pattern.atoms {
            base += u32::from(atom.indices(hay, matcher, &mut indices)?);
        }
        indices.sort_unstable();
        indices.dedup();
        let kind = classify(candidate, hay, &pattern.lower, &indices);
        Some(FuzzyMatch { score: base + kind.bonus(), indices, kind })
    }

    /// Like [`score`], reusing this matcher.
    pub fn score(&mut self, candidate: &str, pattern: &str) -> Option<(u32, Vec<u32>)> {
        self.matches(candidate, &FuzzyPattern::new(pattern)).map(|m| (m.score, m.indices))
    }
}

thread_local! {
    static MATCHER: RefCell<FuzzyMatcher> = RefCell::new(FuzzyMatcher::new());
}

/// Scores one candidate: `(score, matched char indices)`, or `None` when it does not match.
/// Exact > prefix > camel hump / word start > subsequence; ties go to nucleo's score.
pub fn score(candidate: &str, pattern: &str) -> Option<(u32, Vec<u32>)> {
    MATCHER.with(|m| m.borrow_mut().score(candidate, pattern))
}

/// Ranks `candidates` by [`score`] (best first, shorter candidates first on ties) and keeps the
/// best `limit`; each result carries the candidate's position in the input.
pub fn rank<'a>(candidates: impl IntoIterator<Item = &'a str>, pattern: &str, limit: usize) -> Vec<(usize, FuzzyMatch)> {
    let pattern = FuzzyPattern::new(pattern);
    MATCHER.with(|m| {
        let mut m = m.borrow_mut();
        let mut out: Vec<(usize, usize, FuzzyMatch)> = candidates
            .into_iter()
            .enumerate()
            .filter_map(|(i, c)| m.matches(c, &pattern).map(|fm| (i, c.chars().count(), fm)))
            .collect();
        out.sort_by(|a, b| b.2.score.cmp(&a.2.score).then(a.1.cmp(&b.1)).then(a.0.cmp(&b.0)));
        out.truncate(limit);
        out.into_iter().map(|(i, _, fm)| (i, fm)).collect()
    })
}

/// Lowercase-insensitive comparison: `Some(true)` equal, `Some(false)` prefix, `None` neither.
pub(crate) fn prefix_ignore_case(candidate: &str, lower_pattern: &str) -> Option<bool> {
    let mut hay = candidate.chars().flat_map(char::to_lowercase);
    for p in lower_pattern.chars() {
        if hay.next()? != p {
            return None;
        }
    }
    Some(hay.next().is_none())
}

fn classify(candidate: &str, hay: Utf32Str<'_>, lower: &str, indices: &[u32]) -> MatchKind {
    match prefix_ignore_case(candidate, lower) {
        Some(true) => return MatchKind::Exact,
        Some(false) => return MatchKind::Prefix,
        None => {}
    }
    if is_hump_match(hay, indices) { MatchKind::CamelHump } else { MatchKind::Subsequence }
}

/// True when every run of consecutive matched positions begins at a word start.
pub(crate) fn is_hump_match(hay: Utf32Str<'_>, indices: &[u32]) -> bool {
    !indices.is_empty()
        && indices.iter().enumerate().all(|(k, &i)| (k > 0 && indices[k - 1] + 1 == i) || is_word_start(hay, i))
}

/// Word starts: the first char, separators and what follows them, `Order|Service`,
/// `XML|Parser`, and letter/digit transitions.
pub(crate) fn is_word_start(hay: Utf32Str<'_>, i: u32) -> bool {
    if i == 0 {
        return true;
    }
    let (p, c) = (hay.get(i - 1), hay.get(i));
    if !c.is_alphanumeric() || !p.is_alphanumeric() {
        return true;
    }
    if c.is_uppercase() {
        if p.is_lowercase() || p.is_numeric() {
            return true;
        }
        let next = i + 1;
        return p.is_uppercase() && (next as usize) < hay.len() && hay.get(next).is_lowercase();
    }
    c.is_numeric() != p.is_numeric()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind(candidate: &str, pattern: &str) -> Option<MatchKind> {
        FuzzyMatcher::new().matches(candidate, &FuzzyPattern::new(pattern)).map(|m| m.kind)
    }

    #[test]
    fn kinds_follow_intellij_rules() {
        assert_eq!(kind("OrderService", "orderservice"), Some(MatchKind::Exact));
        assert_eq!(kind("OrderService", "orde"), Some(MatchKind::Prefix));
        assert_eq!(kind("OrderService", "OrdSer"), Some(MatchKind::CamelHump));
        assert_eq!(kind("PlaceOrder", "ord"), Some(MatchKind::CamelHump));
        assert_eq!(kind("XMLParser", "xp"), Some(MatchKind::CamelHump));
        assert_eq!(kind("Coordinator", "ord"), Some(MatchKind::Subsequence));
        assert_eq!(kind("Coordinator", "xyz"), None);
    }

    #[test]
    fn prefix_beats_hump_beats_subsequence() {
        let names = ["Coordinator", "PlaceOrder", "OrderService", "Ord"];
        let ranked: Vec<&str> = rank(names, "ord", 10).into_iter().map(|(i, _)| names[i]).collect();
        assert_eq!(ranked, ["Ord", "OrderService", "PlaceOrder", "Coordinator"]);
    }

    #[test]
    fn camel_humps_rank_above_scattered_letters() {
        let names = ["ObservableDictionaryServer", "OrderService", "ProviderSession"];
        let ranked = rank(names, "OrdSer", 10);
        assert_eq!(names[ranked[0].0], "OrderService");
        assert_eq!(ranked[0].1.indices, vec![0, 1, 2, 5, 6, 7]);
    }

    #[test]
    fn score_returns_indices_and_words_match_in_any_order() {
        let (s, idx) = score("Go to File", "file go").unwrap();
        assert!(s > 0);
        assert_eq!(idx, vec![0, 1, 6, 7, 8, 9]);
        assert!(score("Go to File", "file gx").is_none());
        assert_eq!(score("anything", ""), Some((0, vec![])));
    }

    #[test]
    fn word_starts() {
        let mut buf = Vec::new();
        let hay = Utf32Str::new("getHTTPResponse2x_ok", &mut buf);
        let starts: Vec<u32> = (0..hay.len() as u32).filter(|&i| is_word_start(hay, i)).collect();
        // g, H, R(esponse), 2, x, _, o
        assert_eq!(starts, vec![0, 3, 7, 15, 16, 17, 18]);
    }
}
