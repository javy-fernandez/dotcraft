//! Go to File: fuzzy search over an [`IndexSnapshot`] with IntelliJ-like ranking. Matches on
//! the file name always rank above matches that need the folders; within each, exact >
//! prefix > camel hump > subsequence, then the caller's boost (recent files), then shorter
//! paths. `core/order` filters by folder and `Foo.cs:42` carries a line number.

use crate::fuzzy::{FuzzyMatch, FuzzyMatcher, FuzzyPattern, MatchKind, prefix_ignore_case};
use crate::index::{FileEntry, FileIndex, IndexSnapshot};
use crossbeam_channel::{Receiver, Sender, unbounded};
use rc_core::inbox::Outbox;
use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// Score gap between a file-name match and a path-only match.
const TIER: u32 = 1_000_000;

/// A parsed Go to File query.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileQuery {
    /// Fuzzy pattern for the file name (empty for `core/`, which lists a folder's files).
    pub name: String,
    /// Folder pattern from `dir/name`, matched against the folder part of the path.
    pub dir: Option<String>,
    /// Other whitespace-separated words; each must match somewhere in the path.
    pub filters: Vec<String>,
    /// 1-based line from `Foo.cs:42`, `Foo.cs:42:7`, `Foo.cs(42,7)` or `Foo.cs#L42`.
    pub line: Option<u32>,
    /// 1-based column, when given after the line.
    pub column: Option<u32>,
}

impl FileQuery {
    /// Parses what the user typed. Backslashes count as `/`.
    pub fn parse(input: &str) -> FileQuery {
        let (text, line, column) = split_line_suffix(input.trim());
        let text = text.replace('\\', "/");
        let mut words: Vec<&str> = text.split_whitespace().collect();
        let main = words.pop().unwrap_or("");
        let (dir, name) = match main.rsplit_once('/') {
            Some((d, n)) => {
                let d = d.trim_matches('/');
                ((!d.is_empty()).then(|| d.to_string()), n.to_string())
            }
            None => (None, main.to_string()),
        };
        FileQuery { name, dir, filters: words.into_iter().map(str::to_string).collect(), line, column }
    }

    /// True when nothing but (maybe) a line number was typed.
    pub fn is_empty(&self) -> bool {
        self.name.is_empty() && self.dir.is_none() && self.filters.is_empty()
    }
}

fn digits(s: &str) -> Option<u32> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

/// Splits `Foo.cs:42[:7]`, `Foo.cs(42[,7])` and `Foo.cs#L42`; a trailing `:` is dropped.
fn split_line_suffix(s: &str) -> (&str, Option<u32>, Option<u32>) {
    if let Some((head, n)) = s.rsplit_once("#L")
        && let Some(line) = digits(n)
    {
        return (head, Some(line), None);
    }
    if let Some(inner) = s.strip_suffix(')')
        && let Some((head, args)) = inner.rsplit_once('(')
    {
        let (l, c) = args.split_once(',').map_or((args, None), |(l, c)| (l, Some(c.trim())));
        if let Some(line) = digits(l.trim()) {
            return (head, Some(line), c.and_then(digits));
        }
    }
    let s = s.strip_suffix(':').unwrap_or(s);
    if let Some((head, last)) = s.rsplit_once(':')
        && let Some(n) = digits(last)
    {
        if let Some((head2, prev)) = head.rsplit_once(':')
            && let Some(line) = digits(prev)
        {
            return (head2, Some(line), Some(n));
        }
        return (head, Some(n), None);
    }
    (s, None, None)
}

/// One ranked file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileMatch {
    pub entry: FileEntry,
    pub score: u32,
    /// Matched char positions in [`FileEntry::rel`], sorted.
    pub indices: Vec<u32>,
    pub kind: MatchKind,
    /// True when the name pattern matched the file name (not only the path).
    pub in_name: bool,
}

/// The best matches of a query.
#[derive(Clone, Debug, Default)]
pub struct FileSearchResults {
    /// Number of the request (from [`FileSearchWorker::search`]; 0 for direct calls).
    pub generation: u64,
    pub query: FileQuery,
    /// Best first, at most the requested limit.
    pub matches: Vec<FileMatch>,
    /// How many files matched in total.
    pub total: usize,
}

thread_local! {
    static MATCHERS: RefCell<(FuzzyMatcher, FuzzyMatcher)> = RefCell::new((FuzzyMatcher::new(), FuzzyMatcher::for_paths()));
}

/// Ranks the snapshot's files for `query`. `boost` adds points per file (e.g. a few thousand
/// for recent files; [`crate::fuzzy::MatchKind`]s are 10 000 apart). An empty query returns the
/// boosted files only, best boost first. Takes about a millisecond per thousand files.
pub fn search_files(snapshot: &IndexSnapshot, query: &FileQuery, boost: &dyn Fn(&FileEntry) -> u32, limit: usize) -> FileSearchResults {
    let mut found: Vec<FileMatch> = if query.is_empty() {
        snapshot
            .files()
            .iter()
            .filter_map(|e| {
                let b = boost(e);
                (b > 0).then(|| FileMatch { entry: e.clone(), score: b, indices: Vec::new(), kind: MatchKind::Subsequence, in_name: true })
            })
            .collect()
    } else {
        MATCHERS.with(|m| {
            let (names, paths) = &mut *m.borrow_mut();
            let q = Prepared::new(query);
            snapshot.files().iter().filter_map(|e| q.rate(e, names, paths, boost)).collect()
        })
    };
    let total = found.len();
    let order = |a: &FileMatch, b: &FileMatch| {
        b.score.cmp(&a.score).then(a.entry.rel().len().cmp(&b.entry.rel().len())).then(a.entry.rel().cmp(b.entry.rel()))
    };
    if found.len() > limit && limit > 0 {
        found.select_nth_unstable_by(limit - 1, order);
    }
    found.truncate(limit);
    found.sort_by(order);
    FileSearchResults { generation: 0, query: query.clone(), matches: found, total }
}

struct Prepared {
    name: FuzzyPattern,
    name_lower: String,
    dir: Option<FuzzyPattern>,
    filters: Vec<FuzzyPattern>,
}

/// nucleo's part of a score, without the kind bonus.
fn raw(m: &FuzzyMatch) -> u32 {
    m.score - m.kind.bonus()
}

impl Prepared {
    fn new(q: &FileQuery) -> Prepared {
        Prepared {
            name: FuzzyPattern::new(&q.name),
            name_lower: q.name.to_lowercase(),
            dir: q.dir.as_deref().map(|d| FuzzyPattern::new(&format!("{d}/"))),
            filters: q.filters.iter().map(|f| FuzzyPattern::new(f)).collect(),
        }
    }

    fn rate(&self, e: &FileEntry, names: &mut FuzzyMatcher, paths: &mut FuzzyMatcher, boost: &dyn Fn(&FileEntry) -> u32) -> Option<FileMatch> {
        let mut indices = Vec::new();
        let mut extra = 0;
        if let Some(dir) = &self.dir {
            let m = paths.matches(e.dir(), dir)?;
            extra += raw(&m) / 2;
            indices.extend(m.indices);
        }
        for f in &self.filters {
            let m = paths.matches(e.rel(), f)?;
            extra += raw(&m) / 2;
            indices.extend(m.indices);
        }
        let (tier, kind, score) = if self.name.is_empty() {
            (2 * TIER, MatchKind::Subsequence, 0)
        } else if let Some(m) = names.matches(e.name(), &self.name) {
            let offset = e.dir().chars().count() as u32;
            indices.extend(m.indices.iter().map(|i| i + offset));
            let stem = e.name().rsplit_once('.').map_or(e.name(), |(s, _)| s);
            let kind = if prefix_ignore_case(stem, &self.name_lower) == Some(true) { MatchKind::Exact } else { m.kind };
            (2 * TIER, kind, raw(&m) + kind.bonus())
        } else if self.dir.is_none() {
            let m = paths.matches(e.rel(), &self.name)?;
            indices.extend(m.indices);
            (TIER, m.kind, m.score)
        } else {
            return None;
        };
        indices.sort_unstable();
        indices.dedup();
        let score = tier.saturating_add(score).saturating_add(extra).saturating_add(boost(e));
        Some(FileMatch { entry: e.clone(), score, indices, kind, in_name: tier == 2 * TIER })
    }
}

/// Points added per file by the caller (recent files), shareable with the worker thread.
pub type Boost = Arc<dyn Fn(&FileEntry) -> u32 + Send + Sync>;

/// A query for [`FileSearchWorker`].
#[derive(Clone)]
pub struct FileSearchRequest {
    pub text: String,
    pub limit: usize,
    pub boost: Option<Boost>,
}

/// Runs Go to File queries on a background thread against the index's latest snapshot. Only
/// the newest pending query is answered; results go to the outbox (waking the UI). Dropping
/// the worker stops its thread.
pub struct FileSearchWorker {
    tx: Option<Sender<(u64, FileSearchRequest)>>,
    generation: AtomicU64,
}

impl FileSearchWorker {
    pub fn new(index: FileIndex, out: Outbox<FileSearchResults>) -> FileSearchWorker {
        let (tx, rx) = unbounded();
        std::thread::Builder::new()
            .name("rc-search-files".into())
            .spawn(move || worker_loop(index, rx, out))
            .expect("spawn file search thread");
        FileSearchWorker { tx: Some(tx), generation: AtomicU64::new(0) }
    }

    /// Queues a query and returns its generation (echoed in the results).
    pub fn search(&self, request: FileSearchRequest) -> u64 {
        let id = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        if let Some(tx) = &self.tx {
            let _ = tx.send((id, request));
        }
        id
    }
}

impl Drop for FileSearchWorker {
    fn drop(&mut self) {
        self.tx.take();
    }
}

fn worker_loop(index: FileIndex, rx: Receiver<(u64, FileSearchRequest)>, out: Outbox<FileSearchResults>) {
    while let Ok(mut req) = rx.recv() {
        while let Ok(newer) = rx.try_recv() {
            req = newer;
        }
        let (id, r) = req;
        let none = |_: &FileEntry| 0;
        let boost: &dyn Fn(&FileEntry) -> u32 = match &r.boost {
            Some(b) => b.as_ref(),
            None => &none,
        };
        let mut res = search_files(&index.snapshot(), &FileQuery::parse(&r.text), boost, r.limit);
        res.generation = id;
        if !out.send(res) {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn snap(paths: &[&str]) -> IndexSnapshot {
        IndexSnapshot::from_rel_paths(Path::new("/ws"), paths.iter().copied())
    }

    fn top(s: &IndexSnapshot, q: &str) -> Vec<String> {
        search_files(s, &FileQuery::parse(q), &|_| 0, 10).matches.into_iter().map(|m| m.entry.rel().to_string()).collect()
    }

    #[test]
    fn parses_line_suffixes() {
        let q = FileQuery::parse("Foo.cs:42");
        assert_eq!((q.name.as_str(), q.line, q.column), ("Foo.cs", Some(42), None));
        let q = FileQuery::parse("Foo.cs:42:7");
        assert_eq!((q.name.as_str(), q.line, q.column), ("Foo.cs", Some(42), Some(7)));
        let q = FileQuery::parse("Foo.cs(12,3)");
        assert_eq!((q.name.as_str(), q.line, q.column), ("Foo.cs", Some(12), Some(3)));
        let q = FileQuery::parse("Foo.cs#L9");
        assert_eq!((q.name.as_str(), q.line), ("Foo.cs", Some(9)));
        let q = FileQuery::parse("Foo.cs:");
        assert_eq!((q.name.as_str(), q.line), ("Foo.cs", None));
        let q = FileQuery::parse("Foo:bar");
        assert_eq!((q.name.as_str(), q.line), ("Foo:bar", None));
    }

    #[test]
    fn parses_folders_and_filters() {
        let q = FileQuery::parse(r"src\core/order");
        assert_eq!(q.dir.as_deref(), Some("src/core"));
        assert_eq!(q.name, "order");
        let q = FileQuery::parse("core/");
        assert_eq!((q.dir.as_deref(), q.name.as_str()), (Some("core"), ""));
        let q = FileQuery::parse("tests order");
        assert_eq!(q.filters, ["tests"]);
        assert!(FileQuery::parse("  :12").is_empty());
    }

    #[test]
    fn camel_hump_on_name() {
        let s = snap(&["src/Ordinary/Server.cs", "src/Core/OrderService.cs", "src/Core/OrderSerializerTests.cs", "docs/word-search.md"]);
        assert_eq!(top(&s, "OrdSer")[0], "src/Core/OrderService.cs");
    }

    #[test]
    fn name_match_beats_path_match() {
        let s = snap(&["Order/Program.cs", "src/OrderProgram.cs"]);
        assert_eq!(top(&s, "orderprog"), ["src/OrderProgram.cs", "Order/Program.cs"]);
        let r = search_files(&s, &FileQuery::parse("orderprog"), &|_| 0, 10);
        assert!(r.matches[0].in_name && !r.matches[1].in_name);
    }

    #[test]
    fn exact_then_prefix_then_hump_then_subsequence() {
        let s = snap(&["a/Coordinator.cs", "a/PlaceOrder.cs", "a/OrderService.cs", "a/Order.cs"]);
        assert_eq!(top(&s, "order"), ["a/Order.cs", "a/OrderService.cs", "a/PlaceOrder.cs"]);
        assert_eq!(top(&s, "ord"), ["a/Order.cs", "a/OrderService.cs", "a/PlaceOrder.cs", "a/Coordinator.cs"]);
    }

    #[test]
    fn folder_segments_filter() {
        let s = snap(&["src/Core/OrderService.cs", "src/Web/OrderController.cs", "src/Web/Core.cs"]);
        assert_eq!(top(&s, "core/order"), ["src/Core/OrderService.cs"]);
        assert_eq!(top(&s, "web/"), ["src/Web/Core.cs", "src/Web/OrderController.cs"]);
    }

    #[test]
    fn indices_point_into_the_relative_path() {
        let s = snap(&["src/Core/OrderService.cs"]);
        let r = search_files(&s, &FileQuery::parse("core/OrdSer:10"), &|_| 0, 10);
        let m = &r.matches[0];
        let hl: String = m.entry.rel().chars().enumerate().filter(|(i, _)| m.indices.contains(&(*i as u32))).map(|(_, c)| c).collect();
        assert_eq!(hl, "Core/OrdSer");
        assert_eq!(r.query.line, Some(10));
    }

    #[test]
    fn boost_lifts_recent_files() {
        let s = snap(&["a/OrderService.cs", "b/OrderService.cs"]);
        let recent = |e: &FileEntry| if e.rel().starts_with("b/") { 5_000 } else { 0 };
        let r = search_files(&s, &FileQuery::parse("ordser"), &recent, 10);
        assert_eq!(r.matches[0].entry.rel(), "b/OrderService.cs");
        let r = search_files(&s, &FileQuery::parse(""), &recent, 10);
        assert_eq!(r.matches.len(), 1);
    }

    #[test]
    fn limit_keeps_the_best() {
        let paths: Vec<String> = (0..50).map(|i| format!("d/File{i:02}.cs")).collect();
        let s = IndexSnapshot::from_rel_paths(Path::new("/ws"), paths.iter().map(String::as_str));
        let r = search_files(&s, &FileQuery::parse("file"), &|_| 0, 3);
        assert_eq!(r.total, 50);
        assert_eq!(r.matches.len(), 3);
        assert_eq!(r.matches[0].entry.rel(), "d/File00.cs");
        let r = search_files(&s, &FileQuery::parse("file07"), &|_| 0, 3);
        assert_eq!(r.matches[0].entry.rel(), "d/File07.cs");
    }
}
