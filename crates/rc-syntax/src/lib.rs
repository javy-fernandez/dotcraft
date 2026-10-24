//! Syntax services on top of tree-sitter: languages, incremental background parsing over
//! rope snapshots, highlighting of visible ranges, folding ranges and bracket matching.
//!
//! The UI thread owns a [`Syntax`] per document. Every edit is applied to its current tree at
//! once ([`Syntax::edit`], so highlights stay aligned while typing) and a parse job is queued
//! for the [`ParserPool`] thread, which only parses the newest version of each document and
//! sends the tree back; results for older versions are dropped.

mod highlight;
mod lang;
mod parse;
mod structure;
pub mod sample;

pub use highlight::{Hl, Span, highlight_lines};
pub use lang::Lang;
pub use parse::{ParseResult, ParserPool, parse_now};
pub use structure::{FoldKind, FoldRange, folding_ranges, matching_bracket};

use rc_text::{Edit, Rope};
use tree_sitter::{InputEdit, Point, Tree};

/// Syntax state of one open document.
pub struct Syntax {
    pub lang: Lang,
    tree: Option<Tree>,
    /// Version of the text `tree` was parsed from (after edits, it is only approximately
    /// right until the next result arrives).
    tree_version: u64,
    /// Version of the newest text, the one the pending job parses.
    version: u64,
    folds: Vec<FoldRange>,
    /// Bumped whenever highlights may have changed (new tree, edit).
    generation: u64,
}

impl Syntax {
    pub fn new(lang: Lang) -> Self {
        Syntax { lang, tree: None, tree_version: 0, version: 0, folds: Vec::new(), generation: 0 }
    }

    pub fn tree(&self) -> Option<&Tree> {
        self.tree.as_ref()
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn folds(&self) -> &[FoldRange] {
        &self.folds
    }

    /// Whether the tree matches the newest text exactly.
    pub fn is_current(&self) -> bool {
        self.tree.is_some() && self.tree_version == self.version
    }

    /// Updates the current tree with edits just applied to the text (now at `version`).
    pub fn edit(&mut self, edits: &[Edit], version: u64) {
        self.version = version;
        self.generation += 1;
        if let Some(tree) = &mut self.tree {
            for e in edits {
                tree.edit(&input_edit(e));
            }
        }
        // Fold lines shift with the edit until the new ones arrive.
        for e in edits {
            let delta = e.new_end_point.row as isize - e.old_end_point.row as isize;
            if delta == 0 {
                continue;
            }
            let row = e.start_point.row;
            for f in &mut self.folds {
                if f.start_line > row {
                    f.start_line = (f.start_line as isize + delta).max(row as isize) as usize;
                }
                if f.end_line > row {
                    f.end_line = (f.end_line as isize + delta).max(f.start_line as isize) as usize;
                }
            }
        }
    }

    /// Queues a parse of `rope` (at `version`, the newest text) on the pool.
    pub fn request_parse(&mut self, pool: &ParserPool, doc: u64, rope: Rope, version: u64) {
        self.version = version;
        if self.lang == Lang::Plain {
            return;
        }
        pool.submit(doc, self.lang, rope, version, self.tree.clone());
    }

    /// Takes a pool result; returns false (and drops it) when it is for an old version.
    pub fn accept(&mut self, result: ParseResult) -> bool {
        if result.version != self.version {
            return false;
        }
        self.tree = Some(result.tree);
        self.tree_version = result.version;
        self.folds = result.folds;
        self.generation += 1;
        true
    }

    /// Parses synchronously (tests, tiny files, the first frame of a document).
    pub fn parse_blocking(&mut self, rope: &Rope, version: u64) {
        self.version = version;
        if let Some(tree) = parse_now(self.lang, rope, self.tree.as_ref()) {
            self.folds = folding_ranges(self.lang, &tree, rope);
            self.tree = Some(tree);
            self.tree_version = version;
            self.generation += 1;
        }
    }
}

pub fn input_edit(e: &Edit) -> InputEdit {
    let p = |bp: rc_text::BytePoint| Point { row: bp.row, column: bp.column };
    InputEdit {
        start_byte: e.start_byte,
        old_end_byte: e.old_end_byte,
        new_end_byte: e.new_end_byte,
        start_position: p(e.start_point),
        old_end_position: p(e.old_end_point),
        new_end_position: p(e.new_end_point),
    }
}
