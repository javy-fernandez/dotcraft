//! The state of one open document in the editor: text, selection, syntax, folding, scroll
//! position and the decorations the IDE attaches (diagnostics, semantic colours, inlay hints,
//! breakpoints, gutter marks, VCS changes, search matches).

use crate::popup::Popups;
use rc_syntax::{Hl, Lang, ParserPool, Span, Syntax};
use rc_text::commands::{self, Edited, Indent};
use rc_text::{Buffer, ChangeSet, Edit, EditKind, Range, Selection};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Error,
    Warning,
    Info,
    Hint,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Diagnostic {
    /// Char range in the current text (kept up to date across edits).
    pub start: usize,
    pub end: usize,
    pub severity: Severity,
    pub message: String,
    pub code: Option<String>,
    /// Unused code: drawn dimmed instead of underlined.
    pub unnecessary: bool,
    pub deprecated: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct InlayHint {
    /// Char offset the hint is shown before.
    pub pos: usize,
    pub label: String,
    pub pad_left: bool,
    pub pad_right: bool,
}

/// Icons of the gutter's icon column (test runners, implementations…).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GutterIcon {
    RunTest,
    TestPassed,
    TestFailed,
    RunApp,
    Implemented,
    Implementing,
    Overridden,
    Overriding,
    Bookmark,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BreakpointKind {
    Normal,
    Conditional,
    Disabled,
    /// Bound by the debugger.
    Verified,
    Invalid,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineChange {
    Added,
    Modified,
    /// Lines were deleted after this line.
    Deleted,
}

/// What the editor did this frame, for the application to react to.
#[derive(Clone, Debug, PartialEq)]
pub enum EditorEvent {
    /// The text changed (edits are queued in [`EditorState::take_edits`]).
    Changed,
    /// A character that may trigger completion or signature help was typed.
    Typed(char),
    CaretMoved,
    /// Click in the breakpoint area of the gutter (0-based line).
    GutterClicked { line: usize },
    /// Right-click on a line number (0-based line).
    GutterSecondaryClicked { line: usize, screen: egui::Pos2 },
    /// Click on an icon of the gutter's icon column.
    GutterIconClicked { line: usize, icon: GutterIcon },
    /// ⌘/Ctrl+click on a char offset.
    NavigateAt(usize),
    /// The mouse rests over a char offset (after the hover delay).
    HoverAt { pos: usize, screen: egui::Pos2 },
    /// The mouse left the hovered word.
    HoverEnd,
    /// The editor gained keyboard focus.
    Focused,
    /// Text was pasted.
    Pasted,
}

/// One open document view.
pub struct EditorState {
    pub id: u64,
    pub path: Option<PathBuf>,
    pub buffer: Buffer,
    pub selection: Selection,
    pub syntax: Syntax,
    pub indent: Indent,
    pub read_only: bool,
    // --- view
    pub(crate) scroll: egui::Vec2,
    pub(crate) scroll_to_caret: bool,
    pub(crate) folded: BTreeSet<usize>,
    pub(crate) max_cols: usize,
    pub(crate) blink_epoch: f64,
    pub(crate) last_click: Option<(f64, usize, u8)>,
    pub(crate) drag_anchor: Option<Range>,
    pub(crate) column_drag: Option<(usize, usize)>,
    pub(crate) hover: Option<(usize, f64, bool)>,
    pub(crate) alt_taps: (f64, u8, bool),
    pub(crate) ime_preedit: Option<String>,
    /// Lines copied without a selection paste as whole lines (IntelliJ).
    pub(crate) line_clipboard: Option<String>,
    pub(crate) select_stack: Vec<Range>,
    // --- decorations
    pub diagnostics: Vec<Diagnostic>,
    pub inlays: Vec<InlayHint>,
    /// Semantic colours per line (from the language server), char columns.
    pub semantic: Vec<Vec<Span>>,
    pub breakpoints: BTreeMap<usize, BreakpointKind>,
    pub gutter_icons: BTreeMap<usize, GutterIcon>,
    pub line_changes: BTreeMap<usize, LineChange>,
    pub search_matches: Vec<(usize, usize)>,
    pub current_match: Option<usize>,
    /// Line the debugger is stopped at.
    pub execution_line: Option<usize>,
    /// Ranges to highlight like "identifier under caret" (LSP document highlights).
    pub highlights: Vec<(usize, usize, bool)>,
    /// Full-row backgrounds (diff viewer: added, removed and changed lines).
    pub line_backgrounds: BTreeMap<usize, egui::Color32>,
    /// Text shown in an extra gutter column per line (Git blame "Annotate").
    pub annotations: Option<Vec<String>>,
    /// Line numbers to show instead of the buffer's (diff sides: `None` for filler rows).
    pub display_numbers: Option<Vec<Option<usize>>>,
    pub popups: Popups,
    pub(crate) edits: Vec<Edit>,
    pub(crate) hl_cache: Option<HlCache>,
    pub(crate) display: Option<Vec<usize>>,
}

pub(crate) struct HlCache {
    pub generation: u64,
    pub first: usize,
    pub lines: Vec<Vec<Span>>,
}

impl EditorState {
    pub fn new(id: u64, path: Option<PathBuf>, text: &str) -> Self {
        let lang = path.as_deref().map_or(Lang::Plain, Lang::from_path);
        let buffer = Buffer::new(text);
        let indent = detect_indent(&buffer);
        let mut s = EditorState {
            id,
            path,
            buffer,
            selection: Selection::point(0),
            syntax: Syntax::new(lang),
            indent,
            read_only: false,
            scroll: egui::Vec2::ZERO,
            scroll_to_caret: false,
            folded: BTreeSet::new(),
            max_cols: 0,
            blink_epoch: 0.0,
            last_click: None,
            drag_anchor: None,
            column_drag: None,
            hover: None,
            alt_taps: (0.0, 0, false),
            ime_preedit: None,
            line_clipboard: None,
            select_stack: Vec::new(),
            diagnostics: Vec::new(),
            inlays: Vec::new(),
            semantic: Vec::new(),
            breakpoints: BTreeMap::new(),
            gutter_icons: BTreeMap::new(),
            line_changes: BTreeMap::new(),
            search_matches: Vec::new(),
            current_match: None,
            execution_line: None,
            highlights: Vec::new(),
            line_backgrounds: BTreeMap::new(),
            annotations: None,
            display_numbers: None,
            popups: Popups::default(),
            edits: Vec::new(),
            hl_cache: None,
            display: None,
        };
        s.recompute_max_cols();
        s
    }

    pub fn lang(&self) -> Lang {
        self.syntax.lang
    }

    /// Parses now on the calling thread (small files and tests).
    pub fn parse_blocking(&mut self) {
        let rope = self.buffer.snapshot();
        self.syntax.parse_blocking(&rope, self.buffer.version());
    }

    /// Queues a background parse of the current text.
    pub fn request_parse(&mut self, pool: &ParserPool) {
        let rope = self.buffer.snapshot();
        self.syntax.request_parse(pool, self.id, rope, self.buffer.version());
    }

    /// Edits applied since the last call (for LSP `didChange`), oldest first.
    pub fn take_edits(&mut self) -> Vec<Edit> {
        std::mem::take(&mut self.edits)
    }

    pub fn has_pending_edits(&self) -> bool {
        !self.edits.is_empty()
    }

    pub fn caret(&self) -> usize {
        self.selection.primary().head
    }

    /// (line, column) of the primary caret, 0-based.
    pub fn caret_line_col(&self) -> (usize, usize) {
        self.buffer.line_col(self.caret())
    }

    pub fn set_selection(&mut self, sel: Selection) {
        self.selection = sel.clamp(self.buffer.len_chars());
        self.buffer.seal_undo();
        self.scroll_to_caret = true;
    }

    /// Moves the caret to (line, col) and scrolls it into view (centred when far away).
    pub fn goto(&mut self, line: usize, col: usize) {
        let pos = self.buffer.pos_at(line, col);
        self.unfold_line(line);
        self.set_selection(Selection::point(pos));
    }

    pub fn select_range(&mut self, start: usize, end: usize) {
        let line = self.buffer.line_col(start).0;
        self.unfold_line(line);
        self.set_selection(Selection::single(Range::new(start, end)));
    }

    /// Scroll offset in points (persisted with the session).
    pub fn scroll_offset(&self) -> egui::Vec2 {
        self.scroll
    }

    pub fn set_scroll_offset(&mut self, offset: egui::Vec2) {
        self.scroll = offset;
    }

    pub fn request_scroll_to_caret(&mut self) {
        self.scroll_to_caret = true;
    }

    /// Selected text of the primary range.
    pub fn selected_text(&self) -> String {
        let r = self.selection.primary();
        self.buffer.rope().slice(r.start()..r.end()).to_string()
    }

    /// Text of the word around the caret (for find, rename…).
    pub fn word_at_caret(&self) -> String {
        let (s, e) = commands::word_at(self.buffer.rope(), self.caret());
        self.buffer.rope().slice(s..e).to_string()
    }

    // ------------------------------------------------------------ editing

    /// Applies an edit produced by a command; returns whether anything changed.
    pub fn apply(&mut self, edited: Edited, kind: EditKind) -> bool {
        if self.read_only {
            return false;
        }
        if edited.changes.is_empty() {
            if edited.selection != self.selection {
                self.selection = edited.selection;
                self.buffer.seal_undo();
                self.scroll_to_caret = true;
            }
            return false;
        }
        let before = self.selection.clone();
        let edits = self.buffer.apply(&edited.changes, &before, &edited.selection, kind);
        self.after_change(&edited.changes, edits);
        self.selection = edited.selection.clamp(self.buffer.len_chars());
        true
    }

    /// Replaces `start..end` with `text` (completion, code actions, rename…) as one step.
    pub fn replace(&mut self, start: usize, end: usize, text: &str) {
        let cs = ChangeSet::single(rc_text::Change::new(start, end, text));
        let sel = self.selection.map(&cs);
        self.apply(Edited { changes: cs, selection: sel }, EditKind::Other);
    }

    /// Applies several replacements against the current text at once (LSP text edits).
    pub fn apply_text_edits(&mut self, edits: Vec<rc_text::Change>) {
        let cs = ChangeSet::new(edits);
        let sel = self.selection.map(&cs);
        self.apply(Edited { changes: cs, selection: sel }, EditKind::Other);
    }

    /// Replaces the whole text (reload from disk), keeping the caret's line and column.
    pub fn reload(&mut self, text: &str) {
        let (line, col) = self.caret_line_col();
        let sel = self.selection.clone();
        let old_len = self.buffer.len_chars();
        let normalized = text.replace("\r\n", "\n");
        let cs = ChangeSet::single(rc_text::Change::new(0, old_len, normalized));
        let edits = self.buffer.apply(&cs, &sel, &Selection::point(0), EditKind::Other);
        self.after_change(&cs, edits);
        self.buffer.mark_saved();
        let pos = self.buffer.pos_at(line, col);
        self.selection = Selection::point(pos);
    }

    pub fn undo(&mut self) {
        if let Some((edits, sel)) = self.buffer.undo() {
            self.after_raw_edits(edits);
            self.selection = sel;
            self.scroll_to_caret = true;
        }
    }

    pub fn redo(&mut self) {
        if let Some((edits, sel)) = self.buffer.redo() {
            self.after_raw_edits(edits);
            self.selection = sel;
            self.scroll_to_caret = true;
        }
    }

    fn after_change(&mut self, changes: &ChangeSet, edits: Vec<Edit>) {
        // Decorations anchored to offsets follow the text.
        let map = |p: usize| changes.map_pos(p, rc_text::Assoc::Before);
        for d in &mut self.diagnostics {
            d.start = map(d.start);
            d.end = map(d.end).max(d.start);
        }
        for h in &mut self.inlays {
            h.pos = map(h.pos);
        }
        self.search_matches.clear();
        self.current_match = None;
        self.highlights.clear();
        self.after_raw_edits(edits);
    }

    fn after_raw_edits(&mut self, edits: Vec<Edit>) {
        let version = self.buffer.version();
        self.syntax.edit(&edits, version);
        for e in &edits {
            let added = e.new_end_point.row as isize - e.old_end_point.row as isize;
            let row = e.start_point.row;
            shift_lines(&mut self.breakpoints, row, added);
            shift_lines(&mut self.gutter_icons, row, added);
            shift_line_set(&mut self.folded, row, added);
            if added != 0 && row < self.semantic.len() {
                if added > 0 {
                    let at = (row + 1).min(self.semantic.len());
                    self.semantic.splice(at..at, std::iter::repeat_n(Vec::new(), added as usize));
                } else {
                    let end = (row + 1 + (-added) as usize).min(self.semantic.len());
                    self.semantic.drain((row + 1).min(end)..end);
                }
            }
            if let Some(line) = self.semantic.get_mut(row) {
                // The edited line keeps its colours only left of the edit.
                let col = e.start_point.column;
                line.retain(|s| s.end <= col);
            }
            for line in e.start_point.row..=e.new_end_point.row {
                let n = self.buffer.line_len(line.min(self.buffer.len_lines() - 1));
                self.max_cols = self.max_cols.max(n);
            }
        }
        // Folds whose start line was edited away stay valid only if still foldable.
        self.edits.extend(edits);
        self.hl_cache = None;
        self.display = None;
        self.scroll_to_caret = true;
    }

    pub(crate) fn recompute_max_cols(&mut self) {
        let rope = self.buffer.rope();
        self.max_cols = (0..rope.len_lines()).map(|l| rope.line(l).len_chars()).max().unwrap_or(0);
    }

    // ------------------------------------------------------------ folding

    /// Folds the innermost foldable range around `line` (or starting on it).
    pub fn fold_at(&mut self, line: usize) {
        let best = self
            .syntax
            .folds()
            .iter()
            .filter(|f| f.start_line <= line && line <= f.end_line && !self.folded.contains(&f.start_line))
            .min_by_key(|f| f.end_line - f.start_line)
            .copied();
        if let Some(f) = best {
            self.folded.insert(f.start_line);
            self.display = None;
            // A caret inside the fold moves to its first line.
            let (cl, _) = self.caret_line_col();
            if cl > f.start_line && cl <= f.end_line {
                let pos = self.buffer.pos_at(f.start_line, usize::MAX);
                self.selection = Selection::point(pos);
            }
        }
    }

    /// Unfolds every fold hiding `line` or starting on it.
    pub fn unfold_line(&mut self, line: usize) {
        let starts: Vec<usize> = self
            .syntax
            .folds()
            .iter()
            .filter(|f| self.folded.contains(&f.start_line) && f.start_line <= line && line <= self.fold_end(f.end_line, f.kind))
            .map(|f| f.start_line)
            .collect();
        if !starts.is_empty() {
            for s in starts {
                self.folded.remove(&s);
            }
            self.display = None;
        }
    }

    pub fn toggle_fold(&mut self, line: usize) {
        if self.folded.contains(&line) {
            self.folded.remove(&line);
            self.display = None;
        } else if self.syntax.folds().iter().any(|f| f.start_line == line) {
            self.folded.insert(line);
            self.display = None;
        }
    }

    pub fn fold_all(&mut self) {
        self.folded = self.syntax.folds().iter().map(|f| f.start_line).collect();
        // Do not hide the caret.
        let (line, _) = self.caret_line_col();
        self.unfold_line(line);
        self.display = None;
    }

    pub fn unfold_all(&mut self) {
        self.folded.clear();
        self.display = None;
    }

    pub fn is_folded(&self, line: usize) -> bool {
        self.folded.contains(&line)
    }

    /// Last hidden line of a fold ending at `end_line`.
    pub(crate) fn fold_end(&self, end_line: usize, kind: rc_syntax::FoldKind) -> usize {
        // Brace blocks keep their closing line visible: `{...}` then `}` on its own line
        // would read oddly, so IntelliJ shows `{...}` and hides the `}` line too.
        let _ = kind;
        end_line
    }

    /// Display rows → buffer lines (identity when nothing is folded).
    pub(crate) fn display_rows(&mut self) -> Option<&[usize]> {
        if self.folded.is_empty() {
            return None;
        }
        if self.display.is_none() {
            let n = self.buffer.len_lines();
            let mut hidden_until: Option<usize> = None;
            let mut rows = Vec::with_capacity(n);
            let folds: BTreeMap<usize, usize> = self
                .syntax
                .folds()
                .iter()
                .filter(|f| self.folded.contains(&f.start_line))
                .map(|f| (f.start_line, self.fold_end(f.end_line, f.kind)))
                .collect();
            for line in 0..n {
                if let Some(h) = hidden_until {
                    if line <= h {
                        continue;
                    }
                    hidden_until = None;
                }
                rows.push(line);
                if let Some(&end) = folds.get(&line) {
                    hidden_until = Some(end);
                }
            }
            self.display = Some(rows);
        }
        self.display.as_deref()
    }

    /// The fold (start, end) collapsed at `line`, if any.
    pub(crate) fn collapsed_at(&self, line: usize) -> Option<rc_syntax::FoldRange> {
        if !self.folded.contains(&line) {
            return None;
        }
        self.syntax.folds().iter().find(|f| f.start_line == line).copied()
    }

    // ------------------------------------------------------------ decorations

    /// Replaces the semantic colours (LSP semantic tokens decoded to per-line spans).
    pub fn set_semantic(&mut self, lines: Vec<Vec<Span>>) {
        self.semantic = lines;
        self.hl_cache = None;
    }

    pub fn set_diagnostics(&mut self, mut diagnostics: Vec<Diagnostic>) {
        diagnostics.sort_by_key(|d| (d.start, std::cmp::Reverse(d.severity)));
        self.diagnostics = diagnostics;
    }

    /// Worst diagnostic covering `pos`.
    pub fn diagnostic_at(&self, pos: usize) -> Option<&Diagnostic> {
        self.diagnostics
            .iter()
            .filter(|d| d.start <= pos && pos <= d.end.max(d.start + 1) && d.severity != Severity::Hint)
            .min_by_key(|d| d.severity)
    }

    /// Next diagnostic (errors first, like F2) after/before the caret.
    pub fn next_diagnostic(&mut self, forward: bool) -> bool {
        let caret = self.caret();
        let worst = self.diagnostics.iter().filter(|d| d.severity <= Severity::Warning).map(|d| d.severity).min();
        let Some(worst) = worst else { return false };
        let candidates: Vec<usize> = self.diagnostics.iter().filter(|d| d.severity == worst).map(|d| d.start).collect();
        let target = if forward {
            candidates.iter().copied().find(|&p| p > caret).or_else(|| candidates.first().copied())
        } else {
            candidates.iter().rev().copied().find(|&p| p < caret).or_else(|| candidates.last().copied())
        };
        if let Some(p) = target {
            let line = self.buffer.line_col(p).0;
            self.unfold_line(line);
            self.set_selection(Selection::point(p));
            true
        } else {
            false
        }
    }

    pub fn toggle_breakpoint(&mut self, line: usize) {
        if self.breakpoints.remove(&line).is_none() {
            self.breakpoints.insert(line, BreakpointKind::Normal);
        }
    }

    /// Syntax colours for `first..first+count`, merged with semantic colours. Cached until
    /// the tree or the semantic tokens change.
    pub(crate) fn line_spans(&mut self, first: usize, count: usize) -> &[Vec<Span>] {
        let generation = self.syntax.generation();
        let valid = self
            .hl_cache
            .as_ref()
            .is_some_and(|c| c.generation == generation && c.first <= first && first + count <= c.first + c.lines.len());
        if !valid {
            // Highlight a margin around the viewport so small scrolls hit the cache.
            let margin = count / 2;
            let start = first.saturating_sub(margin);
            let total = (count + 2 * margin).min(self.buffer.len_lines() - start);
            let rope = self.buffer.rope();
            let mut lines = match self.syntax.tree() {
                Some(tree) => rc_syntax::highlight_lines(self.syntax.lang, tree, rope, start, total),
                None => rc_syntax::highlight_lexical(self.syntax.lang, rope, start, total).unwrap_or_else(|| vec![Vec::new(); total]),
            };
            for (i, spans) in lines.iter_mut().enumerate() {
                if let Some(sem) = self.semantic.get(start + i)
                    && !sem.is_empty()
                {
                    *spans = merge_spans(spans, sem);
                }
            }
            self.hl_cache = Some(HlCache { generation, first: start, lines });
        }
        let c = self.hl_cache.as_ref().expect("filled above");
        let from = first - c.first;
        &c.lines[from..(from + count).min(c.lines.len())]
    }
}

/// Semantic spans override syntax spans where they overlap.
fn merge_spans(syntax: &[Span], semantic: &[Span]) -> Vec<Span> {
    let mut out: Vec<Span> = Vec::new();
    for s in syntax {
        let mut start = s.start;
        for sem in semantic.iter().filter(|x| x.end > s.start && x.start < s.end) {
            if sem.start > start {
                out.push(Span { start, end: sem.start, hl: s.hl });
            }
            start = start.max(sem.end);
        }
        if start < s.end {
            out.push(Span { start, end: s.end, hl: s.hl });
        }
    }
    // Comments and strings keep their colour even if the server says otherwise.
    for sem in semantic {
        if sem.hl != Hl::Plain {
            out.push(*sem);
        }
    }
    out.sort_by_key(|s| s.start);
    out
}

fn shift_lines<V>(map: &mut BTreeMap<usize, V>, row: usize, added: isize) {
    if added == 0 {
        return;
    }
    let moved: Vec<(usize, V)> = {
        let keys: Vec<usize> = map.range(row + 1..).map(|(k, _)| *k).collect();
        keys.into_iter().filter_map(|k| map.remove(&k).map(|v| (k, v))).collect()
    };
    for (k, v) in moved {
        let nk = k as isize + added;
        if nk > row as isize {
            map.insert(nk as usize, v);
        }
    }
}

fn shift_line_set(set: &mut BTreeSet<usize>, row: usize, added: isize) {
    if added == 0 {
        return;
    }
    let moved: Vec<usize> = set.range(row + 1..).copied().collect();
    for k in &moved {
        set.remove(k);
    }
    for k in moved {
        let nk = k as isize + added;
        if nk > row as isize {
            set.insert(nk as usize);
        }
    }
}

/// Indentation of a file: tabs when most indented lines start with one, else spaces with the
/// most common step (2 or 4).
pub fn detect_indent(buffer: &Buffer) -> Indent {
    let rope = buffer.rope();
    let (mut tabs, mut spaces2, mut spaces4) = (0, 0, 0);
    for line in (0..rope.len_lines()).take(2000) {
        let l = rope.line(line);
        let mut chars = l.chars();
        match chars.next() {
            Some('\t') => tabs += 1,
            Some(' ') => {
                let n = 1 + chars.take_while(|c| *c == ' ').count();
                if n % 4 == 0 {
                    spaces4 += 1;
                } else if n % 2 == 0 {
                    spaces2 += 1;
                }
            }
            _ => {}
        }
    }
    if tabs > spaces2 + spaces4 {
        Indent { unit: "\t".into(), tab_width: 4 }
    } else if spaces2 > spaces4 * 2 {
        Indent { unit: "  ".into(), tab_width: 4 }
    } else {
        Indent::default()
    }
}
