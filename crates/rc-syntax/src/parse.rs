//! Parsing: synchronous helper and the background parser thread.

use crate::lang::Lang;
use crate::structure::{FoldRange, folding_ranges};
use crossbeam_channel::{Receiver, Sender, unbounded};
use rc_text::Rope;
use std::collections::HashMap;
use std::sync::Arc;
use tree_sitter::{Parser, Tree};

/// Parses `rope`, reusing `old` (already edited to match) for an incremental parse.
pub fn parse_now(lang: Lang, rope: &Rope, old: Option<&Tree>) -> Option<Tree> {
    let language = lang.language()?;
    let mut parser = Parser::new();
    parser.set_language(&language).ok()?;
    parse_with(&mut parser, rope, old)
}

fn parse_with(parser: &mut Parser, rope: &Rope, old: Option<&Tree>) -> Option<Tree> {
    let mut read = |byte: usize, _| -> &[u8] {
        if byte >= rope.len_bytes() {
            return &[];
        }
        let (chunk, chunk_start, _, _) = rope.chunk_at_byte(byte);
        &chunk.as_bytes()[byte - chunk_start..]
    };
    parser.parse_with_options(&mut read, old, None)
}

/// A finished parse.
pub struct ParseResult {
    pub doc: u64,
    pub version: u64,
    pub tree: Tree,
    pub folds: Vec<FoldRange>,
    /// Time the parse took, for diagnostics and benchmarks.
    pub micros: u64,
}

struct Job {
    doc: u64,
    lang: Lang,
    rope: Rope,
    version: u64,
    old: Option<Tree>,
}

/// One background thread parsing documents. Jobs queued while it is busy collapse to the
/// newest per document.
pub struct ParserPool {
    jobs: Sender<Job>,
    results: Receiver<ParseResult>,
}

impl ParserPool {
    /// Starts the thread; `wake` is called after every result (e.g. `ctx.request_repaint`).
    pub fn new(wake: Arc<dyn Fn() + Send + Sync>) -> Self {
        let (jobs, job_rx) = unbounded::<Job>();
        let (result_tx, results) = unbounded();
        std::thread::Builder::new()
            .name("dotcraft-parser".into())
            .spawn(move || run(job_rx, result_tx, wake))
            .expect("spawn parser thread");
        ParserPool { jobs, results }
    }

    pub fn submit(&self, doc: u64, lang: Lang, rope: Rope, version: u64, old: Option<Tree>) {
        let _ = self.jobs.send(Job { doc, lang, rope, version, old });
    }

    /// Results ready now (non-blocking).
    pub fn poll(&self) -> Vec<ParseResult> {
        self.results.try_iter().collect()
    }

    /// Waits for the next result (tests and benchmarks).
    pub fn recv_timeout(&self, timeout: std::time::Duration) -> Option<ParseResult> {
        self.results.recv_timeout(timeout).ok()
    }
}

fn run(jobs: Receiver<Job>, results: Sender<ParseResult>, wake: Arc<dyn Fn() + Send + Sync>) {
    let mut parsers: HashMap<Lang, Parser> = HashMap::new();
    while let Ok(first) = jobs.recv() {
        // Keep only the newest job of each document.
        let mut latest: Vec<Job> = vec![first];
        for job in jobs.try_iter() {
            match latest.iter_mut().find(|j| j.doc == job.doc) {
                Some(slot) if job.version >= slot.version => *slot = job,
                Some(_) => {}
                None => latest.push(job),
            }
        }
        for job in latest {
            let Some(language) = job.lang.language() else { continue };
            let parser = parsers.entry(job.lang).or_insert_with(|| {
                let mut p = Parser::new();
                p.set_language(&language).expect("grammar version");
                p
            });
            let t0 = std::time::Instant::now();
            let Some(tree) = parse_with(parser, &job.rope, job.old.as_ref()) else { continue };
            let micros = t0.elapsed().as_micros() as u64;
            let folds = folding_ranges(job.lang, &tree, &job.rope);
            if results.send(ParseResult { doc: job.doc, version: job.version, tree, folds, micros }).is_err() {
                return;
            }
            wake();
        }
    }
}
