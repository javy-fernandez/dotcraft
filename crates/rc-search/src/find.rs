//! Find in Files: walks the workspace in parallel (same rules as the index, plus a
//! [`FileMask`] and an optional folder scope), searches each file with `grep-searcher` and
//! streams one [`FindEvent::File`] per matching file to an [`Outbox`], then a
//! [`FindEvent::Done`]. Binary and huge files are skipped; the total number of matches is
//! capped; dropping the [`SearchHandle`] (or starting a new search on a [`Finder`]) cancels.

use crate::index::{relative_to, walk_builder, walker_threads};
use crate::mask::FileMask;
use crate::query::{Query, QueryError, QueryOptions};
use grep_matcher::{LineTerminator, Match, Matcher, NoCaptures, NoError};
use grep_searcher::{BinaryDetection, Searcher, SearcherBuilder, Sink, SinkContext, SinkContextKind, SinkMatch};
use ignore::WalkState;
use rc_core::inbox::Outbox;
use std::borrow::Cow;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// What and where to search.
#[derive(Clone, Debug)]
pub struct FindOptions {
    pub query: QueryOptions,
    pub mask: FileMask,
    /// Folder (or file) to search, absolute or relative to the root; `None` = whole root.
    pub scope: Option<PathBuf>,
    /// Stop after this many matches ([`FindSummary::truncated`] tells).
    pub max_results: usize,
    /// Lines of context kept before and after each matching line.
    pub context_lines: usize,
    /// Larger files are skipped.
    pub max_file_size: u64,
}

impl Default for FindOptions {
    fn default() -> Self {
        FindOptions {
            query: QueryOptions::default(),
            mask: FileMask::default(),
            scope: None,
            max_results: 10_000,
            context_lines: 0,
            max_file_size: 8 << 20,
        }
    }
}

/// A line with one or more matches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineMatch {
    /// 0-based line number.
    pub line: u32,
    /// The line without its line break.
    pub text: String,
    /// Matches in `text` as char ranges (a match spanning lines ends at the line's end).
    pub ranges: Vec<Range<u32>>,
    pub before: Vec<String>,
    pub after: Vec<String>,
}

/// All matches of one file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileMatches {
    pub path: PathBuf,
    /// Root-relative, `/`-separated.
    pub rel: String,
    pub lines: Vec<LineMatch>,
}

impl FileMatches {
    pub fn match_count(&self) -> usize {
        self.lines.iter().map(|l| l.ranges.len()).sum()
    }
}

/// How a search ended.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FindSummary {
    pub files_searched: usize,
    pub files_matched: usize,
    pub matches: usize,
    /// The result cap was reached.
    pub truncated: bool,
    pub cancelled: bool,
    pub elapsed: Duration,
}

/// Streamed results; `search` is the id of the [`SearchHandle`] that produced them.
#[derive(Clone, Debug)]
pub enum FindEvent {
    File { search: u64, file: FileMatches },
    Done { search: u64, summary: FindSummary },
}

impl FindEvent {
    pub fn search(&self) -> u64 {
        match self {
            FindEvent::File { search, .. } | FindEvent::Done { search, .. } => *search,
        }
    }
}

/// A running search; dropping it or calling [`cancel`](Self::cancel) stops it.
pub struct SearchHandle {
    id: u64,
    cancel: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl SearchHandle {
    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn is_finished(&self) -> bool {
        self.thread.as_ref().is_none_or(|t| t.is_finished())
    }

    /// Blocks until the search ends (tests).
    pub fn wait(mut self) {
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for SearchHandle {
    fn drop(&mut self) {
        if self.thread.is_some() {
            self.cancel();
        }
    }
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// Starts a search of `root` on a background thread. Fails only on a bad pattern.
pub fn find_in_files(root: &Path, options: FindOptions, out: Outbox<FindEvent>) -> Result<SearchHandle, QueryError> {
    let query = Query::new(options.query.clone())?;
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    let root = root.to_path_buf();
    let thread = std::thread::Builder::new()
        .name("rc-search-find".into())
        .spawn(move || {
            let summary = run_search(&root, &options, &query, &flag, id, &out);
            out.send(FindEvent::Done { search: id, summary });
        })
        .expect("spawn find thread");
    Ok(SearchHandle { id, cancel, thread: Some(thread) })
}

/// Keeps at most one search running: each [`start`](Self::start) cancels the previous one.
pub struct Finder {
    root: PathBuf,
    out: Outbox<FindEvent>,
    current: Option<SearchHandle>,
}

impl Finder {
    pub fn new(root: &Path, out: Outbox<FindEvent>) -> Finder {
        Finder { root: root.to_path_buf(), out, current: None }
    }

    /// Cancels the running search and starts a new one; returns its id.
    pub fn start(&mut self, options: FindOptions) -> Result<u64, QueryError> {
        self.cancel();
        let h = find_in_files(&self.root, options, self.out.clone())?;
        let id = h.id();
        self.current = Some(h);
        Ok(id)
    }

    pub fn cancel(&mut self) {
        if let Some(h) = self.current.take() {
            h.cancel();
        }
    }

    /// Id of the latest search (events with another id are stale).
    pub fn current(&self) -> Option<u64> {
        self.current.as_ref().map(SearchHandle::id)
    }
}

struct Counters {
    files_searched: AtomicUsize,
    files_matched: AtomicUsize,
    matches: AtomicUsize,
    truncated: AtomicBool,
}

fn run_search(root: &Path, opts: &FindOptions, query: &Query, cancel: &AtomicBool, id: u64, out: &Outbox<FindEvent>) -> FindSummary {
    let started = Instant::now();
    let start = match &opts.scope {
        Some(s) if s.is_absolute() => s.clone(),
        Some(s) => root.join(s),
        None => root.to_path_buf(),
    };
    let c = Counters {
        files_searched: AtomicUsize::new(0),
        files_matched: AtomicUsize::new(0),
        matches: AtomicUsize::new(0),
        truncated: AtomicBool::new(false),
    };
    let matcher = GrepQuery(query);
    walk_builder(&start).threads(walker_threads()).build_parallel().run(|| {
        let mut searcher = SearcherBuilder::new()
            .line_number(true)
            .multi_line(true)
            .binary_detection(BinaryDetection::quit(0))
            .before_context(opts.context_lines)
            .after_context(opts.context_lines)
            .build();
        let c = &c;
        let matcher = &matcher;
        Box::new(move |res| {
            if cancel.load(Ordering::Relaxed) || c.truncated.load(Ordering::Relaxed) {
                return WalkState::Quit;
            }
            let Ok(e) = res else { return WalkState::Continue };
            if !e.file_type().is_some_and(|t| t.is_file()) {
                return WalkState::Continue;
            }
            let Some(rel) = relative_to(root, e.path()) else { return WalkState::Continue };
            if !opts.mask.matches(&rel) || e.metadata().map_or(true, |m| m.len() > opts.max_file_size) {
                return WalkState::Continue;
            }
            c.files_searched.fetch_add(1, Ordering::Relaxed);
            let mut sink = FileSink { query, lines: Vec::new(), before: Vec::new(), binary: false, max: opts.max_results, counters: c, cancel };
            let _ = searcher.search_path(matcher, e.path(), &mut sink);
            if !sink.binary && !sink.lines.is_empty() && !cancel.load(Ordering::Relaxed) {
                c.files_matched.fetch_add(1, Ordering::Relaxed);
                out.send(FindEvent::File { search: id, file: FileMatches { path: e.path().to_path_buf(), rel, lines: sink.lines } });
            }
            WalkState::Continue
        })
    });
    let truncated = c.truncated.load(Ordering::Relaxed);
    FindSummary {
        files_searched: c.files_searched.load(Ordering::Relaxed),
        files_matched: c.files_matched.load(Ordering::Relaxed),
        matches: c.matches.load(Ordering::Relaxed).min(opts.max_results),
        truncated,
        cancelled: cancel.load(Ordering::Relaxed),
        elapsed: started.elapsed(),
    }
}

/// Adapts a [`Query`] to `grep-matcher`; line-oriented unless the query can match `\n`.
struct GrepQuery<'a>(&'a Query);

impl Matcher for GrepQuery<'_> {
    type Captures = NoCaptures;
    type Error = NoError;

    fn find_at(&self, haystack: &[u8], at: usize) -> Result<Option<Match>, NoError> {
        Ok(self.0.find_at(haystack, at).map(|r| Match::new(r.start, r.end)))
    }

    fn new_captures(&self) -> Result<NoCaptures, NoError> {
        Ok(NoCaptures::new())
    }

    fn line_terminator(&self) -> Option<LineTerminator> {
        (!self.0.may_span_lines()).then(|| LineTerminator::byte(b'\n'))
    }
}

struct FileSink<'a> {
    query: &'a Query,
    lines: Vec<LineMatch>,
    before: Vec<String>,
    binary: bool,
    max: usize,
    counters: &'a Counters,
    cancel: &'a AtomicBool,
}

fn trim_eol(b: &[u8]) -> &[u8] {
    let b = b.strip_suffix(b"\n").unwrap_or(b);
    b.strip_suffix(b"\r").unwrap_or(b)
}

fn char_len(b: &[u8]) -> u32 {
    match std::str::from_utf8(b) {
        Ok(s) => s.chars().count() as u32,
        Err(_) => String::from_utf8_lossy(b).chars().count() as u32,
    }
}

fn lossy(b: &[u8]) -> String {
    match String::from_utf8_lossy(b) {
        Cow::Borrowed(s) => s.to_string(),
        Cow::Owned(s) => s,
    }
}

impl Sink for FileSink<'_> {
    type Error = std::io::Error;

    fn matched(&mut self, _: &Searcher, m: &SinkMatch<'_>) -> Result<bool, std::io::Error> {
        if self.cancel.load(Ordering::Relaxed) {
            return Ok(false);
        }
        let bytes = m.bytes();
        let first = m.line_number().unwrap_or(1).saturating_sub(1) as u32;
        let mut starts = vec![0];
        starts.extend(bytes.iter().enumerate().filter(|&(i, &b)| b == b'\n' && i + 1 < bytes.len()).map(|(i, _)| i + 1));
        for r in self.query.find_bytes(bytes) {
            let li = starts.partition_point(|&s| s <= r.start) - 1;
            let ls = starts[li];
            let le = starts.get(li + 1).copied().unwrap_or(bytes.len());
            let line = trim_eol(&bytes[ls..le]);
            if self.counters.matches.fetch_add(1, Ordering::Relaxed) >= self.max {
                self.counters.truncated.store(true, Ordering::Relaxed);
                return Ok(false);
            }
            let number = first + li as u32;
            if self.lines.last().is_none_or(|l| l.line != number) {
                let before = std::mem::take(&mut self.before);
                self.lines.push(LineMatch { line: number, text: lossy(line), ranges: Vec::new(), before, after: Vec::new() });
            }
            let start = char_len(&line[..(r.start - ls).min(line.len())]);
            let end = char_len(&line[..(r.end - ls).min(line.len())]);
            if let Some(l) = self.lines.last_mut() {
                l.ranges.push(start..end.max(start));
            }
        }
        Ok(true)
    }

    fn context(&mut self, _: &Searcher, ctx: &SinkContext<'_>) -> Result<bool, std::io::Error> {
        let text = lossy(trim_eol(ctx.bytes()));
        match ctx.kind() {
            SinkContextKind::Before => self.before.push(text),
            SinkContextKind::After => {
                if let Some(l) = self.lines.last_mut() {
                    l.after.push(text);
                }
            }
            SinkContextKind::Other => {}
        }
        Ok(true)
    }

    fn binary_data(&mut self, _: &Searcher, _: u64) -> Result<bool, std::io::Error> {
        self.binary = true;
        Ok(false)
    }
}
