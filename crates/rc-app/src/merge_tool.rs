//! The merge tool's state: the conflicted files of the operation in progress (merge, rebase,
//! cherry-pick, revert) and the three-way merge of one of them (ours, result, theirs). Git runs
//! on the shared runtime; results come back through an inbox. The UI is in `ui/merge.rs`.

use rc_core::{Inbox, Runtime, Waker};
use rc_kit::i18n::tr;
use rc_vcs::{ChunkKind, MergeModel, RepoState, Repository, ResultEdit, Side};
use std::collections::BTreeMap;

/// Conflicted paths, the operation in progress and the names of both sides.
type Conflicts = (Vec<String>, RepoState, (String, String));

enum MergeEvent {
    Files(Result<Conflicts, String>),
    /// The versions of a path: (base, ours, theirs), or why it cannot be merged as text.
    Loaded(String, Result<(String, String, String), String>),
    /// An operation finished: (what, error).
    Done(&'static str, Option<String>),
}

/// One conflicted file open in the three panes.
pub struct FileMerge {
    pub path: String,
    pub model: MergeModel,
    pub left: rc_editor::EditorState,
    pub result: rc_editor::EditorState,
    pub right: rc_editor::EditorState,
    /// Result buffer version the model last saw.
    synced: u64,
    dark: Option<bool>,
}

impl FileMerge {
    fn new(path: String, abs: std::path::PathBuf, (base, ours, theirs): (&str, &str, &str), next_id: &mut u64) -> FileMerge {
        let model = MergeModel::new(base, ours, theirs);
        let mut make = |text: &str, read_only: bool| {
            *next_id += 1;
            let mut e = rc_editor::EditorState::new(*next_id, Some(abs.clone()), text);
            e.read_only = read_only;
            e.parse_blocking();
            e
        };
        let left = make(ours, true);
        let result = make(&model.result_text(), false);
        let right = make(theirs, true);
        let synced = result.buffer.version();
        FileMerge { path, model, left, result, right, synced, dark: None }
    }

    /// Applies model edits to the result editor (one undo step each).
    fn apply(&mut self, edits: Vec<ResultEdit>) {
        for e in edits {
            let rope = self.result.buffer.rope();
            let line_char = |l: usize| if l >= rope.len_lines() { rope.len_chars() } else { rope.line_to_char(l) };
            let (start, end) = (line_char(e.lines.start), line_char(e.lines.end));
            self.result.replace(start, end, &e.text);
        }
        self.result.parse_blocking();
        self.synced = self.result.buffer.version();
        self.dark = None;
    }

    pub fn accept(&mut self, chunk: usize, side: Side) {
        let edit = self.model.accept(chunk, side);
        self.apply(edit.into_iter().collect());
    }

    pub fn ignore(&mut self, chunk: usize) {
        self.model.ignore(chunk);
        self.dark = None;
    }

    pub fn apply_non_conflicting(&mut self, side: Option<Side>) {
        let edits = self.model.apply_non_conflicting(side);
        self.apply(edits);
    }

    pub fn result_text(&self) -> String {
        self.result.buffer.rope().to_string()
    }

    /// Takes hand edits of the result into the model and refreshes the highlights.
    pub fn sync(&mut self, dark: bool) {
        if self.result.buffer.version() != self.synced {
            self.model.sync(&self.result_text());
            // Merge results are small files: a full parse keeps the highlighting in step.
            self.result.parse_blocking();
            self.synced = self.result.buffer.version();
            self.dark = None;
        }
        if self.dark != Some(dark) {
            self.dark = Some(dark);
            self.paint_backgrounds(dark);
        }
    }

    fn paint_backgrounds(&mut self, dark: bool) {
        let m = &self.model;
        let mut left = BTreeMap::new();
        let mut right = BTreeMap::new();
        let mut result = BTreeMap::new();
        for (i, c) in m.chunks.iter().enumerate() {
            let resolved = m.is_resolved(i);
            for (side, map) in [(Side::Ours, &mut left), (Side::Theirs, &mut right)] {
                if c.changed_by(side) {
                    let color = chunk_color(c, Some(side), resolved, dark);
                    c.side(side).for_each(|l| {
                        map.insert(l, color);
                    });
                }
            }
            if !resolved {
                let color = chunk_color(c, None, false, dark);
                m.result_range(i).for_each(|l| {
                    result.insert(l, color);
                });
            }
        }
        self.left.line_backgrounds = left;
        self.right.line_backgrounds = right;
        self.result.line_backgrounds = result;
    }

    /// The first unresolved chunk after (or before) the one at the result's top line.
    pub fn next_unresolved(&self, from_line: usize, forward: bool) -> Option<usize> {
        let m = &self.model;
        let mut open = (0..m.chunks.len()).filter(|i| !m.is_resolved(*i));
        if forward { open.find(|i| m.result_range(*i).start > from_line) } else { open.rev().find(|i| m.result_range(*i).start < from_line) }
    }
}

/// Highlight of a chunk (Rider's merge colours): red for conflicts, else green / grey / blue
/// for lines added / deleted / changed by a side; `side` None is the result pane.
pub fn chunk_color(c: &rc_vcs::Chunk, side: Option<Side>, resolved: bool, dark: bool) -> egui::Color32 {
    let s = rc_kit::theme::scheme(dark);
    let base = if c.kind == ChunkKind::Conflict {
        if dark { egui::Color32::from_rgb(0xD5, 0x5A, 0x50) } else { egui::Color32::from_rgb(0xF0, 0x80, 0x78) }
    } else {
        let changed = side.unwrap_or(if c.kind == ChunkKind::Theirs { Side::Theirs } else { Side::Ours });
        if c.side(changed).is_empty() {
            s.deleted_lines
        } else if c.base.is_empty() {
            s.added_lines
        } else {
            s.modified_lines
        }
    };
    let strength = if resolved { 0.18 } else if dark { 0.5 } else { 0.55 };
    base.gamma_multiply(strength)
}

pub struct MergeTool {
    pub repo: Repository,
    pub files: Vec<String>,
    pub state: RepoState,
    /// Names of ours and theirs.
    pub sides: (String, String),
    pub loading: bool,
    pub selected: Option<String>,
    pub open: Option<FileMerge>,
    /// A file that cannot be merged as text (binary or deleted on one side).
    pub unmergeable: Option<(String, String)>,
    pub busy: Option<&'static str>,
    pub error: Option<String>,
    /// The operation completed or was aborted: the tool closes.
    pub finished: bool,
    /// Apply was clicked once with unresolved changes left (the next click saves anyway).
    pub confirm_apply: bool,
    inbox: Inbox<MergeEvent>,
    next_id: u64,
}

impl MergeTool {
    pub fn new(repo: Repository, waker: Waker) -> MergeTool {
        let mut tool = MergeTool {
            repo,
            files: Vec::new(),
            state: RepoState::Clean,
            sides: Default::default(),
            loading: true,
            selected: None,
            open: None,
            unmergeable: None,
            busy: None,
            error: None,
            finished: false,
            confirm_apply: false,
            inbox: Inbox::new(waker),
            next_id: 1 << 41,
        };
        tool.reload();
        tool
    }

    /// Reads the conflicted files and the operation in progress again.
    pub fn reload(&mut self) {
        self.loading = true;
        let (repo, out) = (self.repo.clone(), self.inbox.sender());
        Runtime::spawn(async move {
            let r = async {
                let files = repo.conflicted_paths().await?;
                let state = repo.state().await;
                Ok::<_, rc_vcs::GitError>((files, state, repo.conflict_sides().await))
            }
            .await
            .map_err(|e| e.to_string());
            out.send(MergeEvent::Files(r));
        });
    }

    /// Opens the three-way merge of a conflicted path.
    pub fn open_file(&mut self, path: &str) {
        self.selected = Some(path.to_string());
        self.unmergeable = None;
        let (repo, out, path) = (self.repo.clone(), self.inbox.sender(), path.to_string());
        Runtime::spawn(async move {
            let r = async {
                let v = repo.conflict_versions(&path).await.map_err(|e| e.to_string())?;
                let (Some(ours), Some(theirs)) = (&v.ours, &v.theirs) else {
                    return Err(tr("The file was deleted on one side: accept yours or theirs").to_string());
                };
                if ours.binary || theirs.binary || v.base.as_ref().is_some_and(|b| b.binary) {
                    return Err(tr("Binary files cannot be merged here: accept yours or theirs").to_string());
                }
                let base = v.base.as_ref().map(|b| b.text().into_owned()).unwrap_or_default();
                Ok((base, ours.text().into_owned(), theirs.text().into_owned()))
            }
            .await;
            out.send(MergeEvent::Loaded(path, r));
        });
    }

    pub fn close_file(&mut self) {
        self.open = None;
        self.confirm_apply = false;
        self.unmergeable = None;
    }

    fn op<F>(&mut self, what: &'static str, f: F)
    where
        F: std::future::Future<Output = Result<(), rc_vcs::GitError>> + Send + 'static,
    {
        if self.busy.is_some() {
            return;
        }
        self.busy = Some(what);
        self.error = None;
        let out = self.inbox.sender();
        Runtime::spawn(async move {
            let r = f.await;
            out.send(MergeEvent::Done(what, r.err().map(|e| crate::vcs::error_text(&e))));
        });
    }

    /// Resolves whole files with one side (`checkout --ours/--theirs` and `git add`).
    pub fn accept_side(&mut self, paths: Vec<String>, side: Side) {
        let repo = self.repo.clone();
        self.op("accept", async move { repo.resolve_with(&paths, side == Side::Theirs).await });
    }

    /// Saves the result of the open file and marks it resolved.
    pub fn apply_result(&mut self) {
        let Some(f) = &self.open else { return };
        let (repo, path, text) = (self.repo.clone(), f.path.clone(), f.result_text());
        self.op("apply", async move { repo.resolve_with_text(&path, &text).await });
    }

    pub fn continue_operation(&mut self) {
        let repo = self.repo.clone();
        self.op("continue", async move { repo.continue_operation().await });
    }

    pub fn abort_operation(&mut self) {
        let repo = self.repo.clone();
        self.op("abort", async move { repo.abort_operation().await });
    }

    /// Drains events; returns notices to show.
    pub fn update(&mut self, dark: bool) -> Vec<String> {
        let mut notices = Vec::new();
        for ev in self.inbox.drain() {
            match ev {
                MergeEvent::Files(Ok((files, state, sides))) => {
                    self.loading = false;
                    self.files = files;
                    self.state = state;
                    self.sides = sides;
                    if self.selected.as_ref().is_none_or(|s| !self.files.contains(s)) {
                        self.selected = self.files.first().cloned();
                    }
                    if state == RepoState::Clean && self.files.is_empty() {
                        self.finished = true;
                    }
                }
                MergeEvent::Files(Err(e)) => {
                    self.loading = false;
                    self.error = Some(e);
                }
                MergeEvent::Loaded(path, Ok((base, ours, theirs))) => {
                    if self.selected.as_deref() == Some(path.as_str()) {
                        let abs = self.repo.abs_path(&path);
                        self.open = Some(FileMerge::new(path, abs, (&base, &ours, &theirs), &mut self.next_id));
                    }
                }
                MergeEvent::Loaded(path, Err(e)) => self.unmergeable = Some((path, e)),
                MergeEvent::Done(what, err) => {
                    self.busy = None;
                    match err {
                        Some(e) => self.error = Some(e),
                        None => {
                            match what {
                                "apply" => {
                                    self.confirm_apply = false;
                                    if let Some(f) = self.open.take() {
                                        notices.push(rc_kit::i18n::trf("Resolved {path}", &[("path", &f.path)]));
                                    }
                                }
                                "accept" => self.close_file(),
                                "continue" => notices.push(tr("Operation continued").to_string()),
                                "abort" => {
                                    self.close_file();
                                    notices.push(tr("Operation aborted").to_string());
                                }
                                _ => {}
                            }
                        }
                    }
                    self.reload();
                }
            }
        }
        if let Some(f) = &mut self.open {
            f.sync(dark);
        }
        notices
    }
}

/// Lines of `to` matching `line` of `from`, given matching anchor points (both increasing):
/// equal regions map one to one, chunks are stretched.
pub fn map_line(anchors: &[(usize, usize)], line: f32) -> f32 {
    let mut prev = (0usize, 0usize);
    for &(a, b) in anchors {
        if (a as f32) >= line {
            let span = (a - prev.0) as f32;
            let t = if span <= 0.0 { 0.0 } else { (line - prev.0 as f32) / span };
            return prev.1 as f32 + t * (b - prev.1) as f32;
        }
        prev = (a, b);
    }
    prev.1 as f32 + (line - prev.0 as f32)
}

/// Anchors from the result to one side: the start and end of every chunk.
pub fn anchors(model: &MergeModel, side: Side) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for (i, c) in model.chunks.iter().enumerate() {
        let (r, s) = (model.result_range(i), c.side(side));
        out.push((r.start, s.start));
        out.push((r.end, s.end));
    }
    out.push((model.result_lines().len(), model.lines(side).len()));
    // Keep both coordinates non-decreasing (hand edits can make ranges overlap).
    let mut fixed: Vec<(usize, usize)> = Vec::with_capacity(out.len());
    for (a, b) in out {
        let (pa, pb) = fixed.last().copied().unwrap_or((0, 0));
        fixed.push((a.max(pa), b.max(pb)));
    }
    fixed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_map_through_chunks() {
        // Result = base; ours inserts 2 lines after line 1.
        let m = MergeModel::new("a\nb\nc\n", "a\nx\ny\nb\nc\n", "a\nb\nc\n");
        let ours = anchors(&m, Side::Ours);
        assert_eq!(map_line(&ours, 0.0), 0.0);
        assert_eq!(map_line(&ours, 2.0), 4.0);
        let back: Vec<(usize, usize)> = ours.iter().map(|(a, b)| (*b, *a)).collect();
        assert_eq!(map_line(&back, 4.0), 2.0);
        assert_eq!(map_line(&back, 2.0), 1.0);
    }
}
