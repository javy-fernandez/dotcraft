//! Three-way merge of conflicted files: a diff3-style split of base / ours / theirs into
//! changed chunks, the editable result model behind the merge tool, and the commands that
//! finish or abort the merge, rebase, cherry-pick or revert in progress.

use crate::error::{GitError, Result};
use crate::repo::{RepoState, Repository};
use crate::textdiff;
use std::hash::Hash;
use std::ops::Range;

/// One side of a conflict: ours (`:2:`, the checked-out side) or theirs (`:3:`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Side {
    Ours,
    Theirs,
}

impl Side {
    pub fn other(self) -> Side {
        match self {
            Side::Ours => Side::Theirs,
            Side::Theirs => Side::Ours,
        }
    }
}

/// What changed the base in a chunk.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChunkKind {
    /// Only ours changed it.
    Ours,
    /// Only theirs changed it.
    Theirs,
    /// Both sides made the same change.
    Both,
    /// Both sides changed it differently.
    Conflict,
}

/// A changed region: line ranges of the base and both sides (empty for pure insertions or
/// deletions; `start` still says where the chunk sits).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Chunk {
    pub kind: ChunkKind,
    pub base: Range<usize>,
    pub ours: Range<usize>,
    pub theirs: Range<usize>,
}

impl Chunk {
    pub fn side(&self, side: Side) -> Range<usize> {
        match side {
            Side::Ours => self.ours.clone(),
            Side::Theirs => self.theirs.clone(),
        }
    }

    /// Whether `side` changed the base here.
    pub fn changed_by(&self, side: Side) -> bool {
        match self.kind {
            ChunkKind::Conflict | ChunkKind::Both => true,
            ChunkKind::Ours => side == Side::Ours,
            ChunkKind::Theirs => side == Side::Theirs,
        }
    }
}

/// The changed chunks of a three-way merge, in order (diff3): base lines matched on both
/// sides in step are stable; everything between two stable runs is one chunk, classified by
/// which sides differ from the base there.
pub fn diff3<T: Hash + Eq>(base: &[T], ours: &[T], theirs: &[T]) -> Vec<Chunk> {
    let (nb, no, nt) = (base.len(), ours.len(), theirs.len());
    let mut to_ours = vec![None; nb];
    let mut to_theirs = vec![None; nb];
    for (b, o) in textdiff::matches(base, ours, textdiff::DEFAULT_COST_LIMIT) {
        to_ours[b] = Some(o);
    }
    for (b, t) in textdiff::matches(base, theirs, textdiff::DEFAULT_COST_LIMIT) {
        to_theirs[b] = Some(t);
    }
    let mut out = Vec::new();
    let (mut i, mut j, mut k) = (0, 0, 0);
    loop {
        while i < nb && to_ours[i] == Some(j) && to_theirs[i] == Some(k) {
            (i, j, k) = (i + 1, j + 1, k + 1);
        }
        if i >= nb && j >= no && k >= nt {
            break;
        }
        // The next base line both sides keep ends the chunk (the matches only increase).
        let (m, oe, te) = match (i..nb).find(|&m| to_ours[m].is_some() && to_theirs[m].is_some()) {
            Some(m) => (m, to_ours[m].unwrap_or(no), to_theirs[m].unwrap_or(nt)),
            None => (nb, no, nt),
        };
        let (b, o, t) = (&base[i..m], &ours[j..oe], &theirs[k..te]);
        let kind = if o == t {
            (o != b).then_some(ChunkKind::Both)
        } else if o == b {
            Some(ChunkKind::Theirs)
        } else if t == b {
            Some(ChunkKind::Ours)
        } else {
            Some(ChunkKind::Conflict)
        };
        if let Some(kind) = kind {
            out.push(Chunk { kind, base: i..m, ours: j..oe, theirs: k..te });
        }
        (i, j, k) = (m, oe, te);
    }
    out
}

/// Lines with their terminators, so joining them gives the text back.
pub fn split_lines(text: &str) -> Vec<String> {
    text.split_inclusive('\n').map(str::to_string).collect()
}

/// How a chunk of the result was resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Resolution {
    Unresolved,
    /// Replaced by one side.
    Taken(Side),
    /// Both sides, the first one first.
    Both(Side),
    /// Left as in the base.
    Ignored,
    /// Edited by hand in the result.
    Edited,
}

/// A change of the result text: `lines` (in the result before the change) replaced by `text`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResultEdit {
    pub lines: Range<usize>,
    pub text: String,
}

/// The merge tool's model: the three versions, their chunks and the result, which starts as
/// the base. Accepting a side rewrites the chunk's lines of the result; hand edits are mapped
/// back onto the chunks with [`MergeModel::sync`].
#[derive(Clone, Debug)]
pub struct MergeModel {
    pub base: Vec<String>,
    pub ours: Vec<String>,
    pub theirs: Vec<String>,
    pub chunks: Vec<Chunk>,
    ranges: Vec<Range<usize>>,
    resolutions: Vec<Resolution>,
    result: Vec<String>,
}

impl MergeModel {
    pub fn new(base: &str, ours: &str, theirs: &str) -> MergeModel {
        let (base, ours, theirs) = (split_lines(base), split_lines(ours), split_lines(theirs));
        let chunks = diff3(&base, &ours, &theirs);
        let ranges = chunks.iter().map(|c| c.base.clone()).collect();
        let resolutions = vec![Resolution::Unresolved; chunks.len()];
        MergeModel { result: base.clone(), base, ours, theirs, chunks, ranges, resolutions }
    }

    pub fn lines(&self, side: Side) -> &[String] {
        match side {
            Side::Ours => &self.ours,
            Side::Theirs => &self.theirs,
        }
    }

    pub fn result_lines(&self) -> &[String] {
        &self.result
    }

    pub fn result_text(&self) -> String {
        self.result.concat()
    }

    /// The lines of chunk `i` in the result.
    pub fn result_range(&self, i: usize) -> Range<usize> {
        self.ranges[i].clone()
    }

    pub fn resolution(&self, i: usize) -> Resolution {
        self.resolutions[i]
    }

    pub fn is_resolved(&self, i: usize) -> bool {
        self.resolutions[i] != Resolution::Unresolved
    }

    /// Chunks not resolved yet.
    pub fn unresolved(&self) -> usize {
        self.resolutions.iter().filter(|r| **r == Resolution::Unresolved).count()
    }

    /// Conflicting chunks not resolved yet.
    pub fn unresolved_conflicts(&self) -> usize {
        self.chunks.iter().zip(&self.resolutions).filter(|(c, r)| c.kind == ChunkKind::Conflict && **r == Resolution::Unresolved).count()
    }

    /// Whether `side` of chunk `i` can still be applied: replaces an unresolved chunk, or is
    /// appended after the other side of a conflict.
    pub fn can_accept(&self, i: usize, side: Side) -> bool {
        let c = &self.chunks[i];
        match self.resolutions[i] {
            Resolution::Unresolved => c.changed_by(side),
            Resolution::Taken(taken) => c.kind == ChunkKind::Conflict && taken != side,
            _ => false,
        }
    }

    /// Applies `side` of chunk `i` (see [`MergeModel::can_accept`]).
    pub fn accept(&mut self, i: usize, side: Side) -> Option<ResultEdit> {
        if !self.can_accept(i, side) {
            return None;
        }
        let lines = self.lines(side)[self.chunks[i].side(side)].to_vec();
        match self.resolutions[i] {
            Resolution::Taken(first) => {
                let end = self.ranges[i].end;
                self.resolutions[i] = Resolution::Both(first);
                Some(self.replace(i, end..end, lines))
            }
            _ => {
                self.resolutions[i] = Resolution::Taken(side);
                let range = self.ranges[i].clone();
                Some(self.replace(i, range, lines))
            }
        }
    }

    /// Keeps the result of chunk `i` as it is (the base, unless edited) and marks it resolved.
    pub fn ignore(&mut self, i: usize) {
        if self.resolutions[i] == Resolution::Unresolved {
            self.resolutions[i] = Resolution::Ignored;
        }
    }

    /// Applies every unresolved change that does not conflict, from `side` only or from both.
    pub fn apply_non_conflicting(&mut self, side: Option<Side>) -> Vec<ResultEdit> {
        let mut edits = Vec::new();
        for i in 0..self.chunks.len() {
            let pick = match self.chunks[i].kind {
                ChunkKind::Conflict => None,
                ChunkKind::Both => Some(side.unwrap_or(Side::Ours)),
                ChunkKind::Ours => Some(Side::Ours).filter(|s| side.is_none_or(|w| w == *s)),
                ChunkKind::Theirs => Some(Side::Theirs).filter(|s| side.is_none_or(|w| w == *s)),
            };
            if let Some(s) = pick
                && self.resolutions[i] == Resolution::Unresolved
                && let Some(e) = self.accept(i, s)
            {
                edits.push(e);
            }
        }
        edits
    }

    /// Replaces `range` (inside chunk `i`'s result lines) and shifts the chunks after it.
    fn replace(&mut self, i: usize, range: Range<usize>, lines: Vec<String>) -> ResultEdit {
        let text = lines.concat();
        let delta = lines.len() as isize - range.len() as isize;
        let edit = ResultEdit { lines: range.clone(), text };
        self.result.splice(range.clone(), lines);
        self.ranges[i].end = (self.ranges[i].end as isize + delta) as usize;
        for r in self.ranges.iter_mut().skip(i + 1) {
            r.start = (r.start as isize + delta) as usize;
            r.end = (r.end as isize + delta) as usize;
        }
        edit
    }

    /// Takes a result edited by hand: chunks move with the lines around them, and chunks whose
    /// lines changed count as resolved by hand.
    pub fn sync(&mut self, text: &str) {
        let new = split_lines(text);
        if new == self.result {
            return;
        }
        let changes = textdiff::diff(&self.result, &new);
        for (range, resolution) in self.ranges.iter_mut().zip(self.resolutions.iter_mut()) {
            let (moved, touched) = remap(range.clone(), &changes);
            *range = moved;
            if touched {
                *resolution = Resolution::Edited;
            }
        }
        self.result = new;
    }
}

/// Where a line range ends up after `changes`, and whether a change touched it.
fn remap(r: Range<usize>, changes: &[textdiff::Change]) -> (Range<usize>, bool) {
    let (s, e) = (r.start, r.end);
    let empty = s == e;
    let mut delta = 0isize;
    let mut start: Option<usize> = None;
    let mut end: Option<usize> = None;
    for ch in changes {
        let (a, b) = (ch.old.start, ch.old.end);
        let insertion = a == b;
        let before = b <= s && !(empty && insertion && a == s);
        let after = a >= e && !(empty && insertion && a == e);
        let d = ch.new.len() as isize - ch.old.len() as isize;
        if after && !before {
            break;
        }
        if before {
            delta += d;
            continue;
        }
        let shifted = (s as isize + delta) as usize;
        start = Some(start.unwrap_or(shifted).min(ch.new.start));
        delta += d;
        end = Some(ch.new.end);
    }
    let shifted_end = (e as isize + delta) as usize;
    match start {
        Some(start) => (start..shifted_end.max(end.unwrap_or(0)), true),
        None => ((s as isize + delta) as usize..shifted_end, false),
    }
}

impl Repository {
    /// Writes the merged text of a conflicted path and marks it resolved (`git add`).
    pub async fn resolve_with_text(&self, path: &str, text: &str) -> Result<()> {
        let conflicted = self.conflicted_paths().await?;
        if !conflicted.iter().any(|p| p == path) {
            return Err(GitError::NotFound(format!("conflicted path {path}")));
        }
        std::fs::write(self.abs_path(path), text)?;
        self.cmd().args(["--literal-pathspecs", "add", "--"]).arg(path).output().await.map(drop)
    }

    /// Finishes the merge, rebase, cherry-pick or revert in progress once every conflict is
    /// resolved (the default messages are kept). A rebase may stop again on the next commit.
    pub async fn continue_operation(&self) -> Result<()> {
        let conflicted = self.conflicted_paths().await?;
        if !conflicted.is_empty() {
            return Err(GitError::Conflicts(conflicted.join("\n")));
        }
        let command = match self.state().await {
            RepoState::Merging => "merge",
            RepoState::Rebasing => "rebase",
            RepoState::CherryPicking => "cherry-pick",
            RepoState::Reverting => "revert",
            RepoState::Clean | RepoState::Bisecting => return Ok(()),
        };
        self.cmd().args([command, "--continue"]).output().await.map(drop)
    }

    /// Aborts the merge, rebase, cherry-pick or revert in progress.
    pub async fn abort_operation(&self) -> Result<()> {
        let command = match self.state().await {
            RepoState::Merging => "merge",
            RepoState::Rebasing => "rebase",
            RepoState::CherryPicking => "cherry-pick",
            RepoState::Reverting => "revert",
            RepoState::Clean | RepoState::Bisecting => return Ok(()),
        };
        self.cmd().args([command, "--abort"]).output().await.map(drop)
    }

    /// Short names of ours and theirs for the operation in progress: the current branch and
    /// the merged branch, the rebase target and the rebased branch, or the picked commit.
    pub async fn conflict_sides(&self) -> (String, String) {
        let state = self.state().await;
        let read = |name: &str| std::fs::read_to_string(self.git_dir().join(name)).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        let current = match self.current_branch().await {
            Ok(Some(b)) => b,
            _ => self.short_name("HEAD").await,
        };
        match state {
            RepoState::Rebasing => {
                let onto = match read("rebase-merge/onto").or_else(|| read("rebase-apply/onto")) {
                    Some(hash) => self.short_name(&hash).await,
                    None => "HEAD".to_string(),
                };
                let branch = read("rebase-merge/head-name").or_else(|| read("rebase-apply/head-name")).map(|h| h.trim_start_matches("refs/heads/").to_string());
                let theirs = match branch {
                    Some(b) if b != "detached HEAD" => b,
                    _ => self.short_name("REBASE_HEAD").await,
                };
                (onto, theirs)
            }
            RepoState::Merging => (current, self.short_name("MERGE_HEAD").await),
            RepoState::CherryPicking => (current, self.short_name("CHERRY_PICK_HEAD").await),
            RepoState::Reverting => (current, self.short_name("REVERT_HEAD").await),
            _ => (current, String::new()),
        }
    }

    /// A branch name for `rev` when one points at it, otherwise its short hash.
    async fn short_name(&self, rev: &str) -> String {
        let named = self.cmd().args(["name-rev", "--name-only", "--no-undefined", "--exclude=refs/tags/*", "--refs=refs/heads/*", "--refs=refs/remotes/*", rev]).text().await.ok();
        if let Some(n) = named.map(|n| n.trim().to_string()).filter(|n| !n.is_empty() && !n.contains('~') && !n.contains('^')) {
            return n.trim_start_matches("remotes/").to_string();
        }
        self.cmd().args(["rev-parse", "--short", "--verify", "--quiet", rev]).text().await.map(|s| s.trim().to_string()).unwrap_or_else(|_| rev.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(s: &str) -> Vec<String> {
        split_lines(s)
    }

    fn chunks(base: &str, ours: &str, theirs: &str) -> Vec<Chunk> {
        diff3(&lines(base), &lines(ours), &lines(theirs))
    }

    #[test]
    fn identical_versions_have_no_chunks() {
        assert!(chunks("a\nb\n", "a\nb\n", "a\nb\n").is_empty());
        assert!(chunks("", "", "").is_empty());
    }

    #[test]
    fn one_sided_changes_are_not_conflicts() {
        let c = chunks("a\nb\nc\nd\n", "a\nB\nc\nd\n", "a\nb\nc\nD\n");
        assert_eq!(c, vec![Chunk { kind: ChunkKind::Ours, base: 1..2, ours: 1..2, theirs: 1..2 }, Chunk { kind: ChunkKind::Theirs, base: 3..4, ours: 3..4, theirs: 3..4 }]);
    }

    #[test]
    fn insertions_and_deletions_on_each_side() {
        // Ours inserts after a, theirs deletes c.
        let c = chunks("a\nb\nc\nd\n", "a\nx\nb\nc\nd\n", "a\nb\nd\n");
        assert_eq!(c, vec![Chunk { kind: ChunkKind::Ours, base: 1..1, ours: 1..2, theirs: 1..1 }, Chunk { kind: ChunkKind::Theirs, base: 2..3, ours: 3..4, theirs: 2..2 }]);
        // Insertions at the end, and on both sides identically.
        let c = chunks("a\n", "a\nz\n", "a\nz\n");
        assert_eq!(c, vec![Chunk { kind: ChunkKind::Both, base: 1..1, ours: 1..2, theirs: 1..2 }]);
    }

    #[test]
    fn different_changes_of_the_same_lines_conflict() {
        let c = chunks("a\nb\nc\n", "a\nours\nc\n", "a\ntheirs\nc\n");
        assert_eq!(c, vec![Chunk { kind: ChunkKind::Conflict, base: 1..2, ours: 1..2, theirs: 1..2 }]);
        // Adjacent changes conflict (as in git).
        let c = chunks("1\n2\n3\n4\n", "1\nX\n3\n4\n", "1\n2\nY\n4\n");
        assert_eq!(c, vec![Chunk { kind: ChunkKind::Conflict, base: 1..3, ours: 1..3, theirs: 1..3 }]);
        // Insertions at the same place with different text.
        let c = chunks("a\nb\n", "a\no\nb\n", "a\nt\nb\n");
        assert_eq!(c, vec![Chunk { kind: ChunkKind::Conflict, base: 1..1, ours: 1..2, theirs: 1..2 }]);
        // Both delete the whole file vs one edits it.
        let c = chunks("a\n", "", "b\n");
        assert_eq!(c, vec![Chunk { kind: ChunkKind::Conflict, base: 0..1, ours: 0..0, theirs: 0..1 }]);
    }

    #[test]
    fn missing_final_newline_is_a_change() {
        let c = chunks("a\nb", "a\nb\n", "a\nb");
        assert_eq!(c, vec![Chunk { kind: ChunkKind::Ours, base: 1..2, ours: 1..2, theirs: 1..2 }]);
    }

    /// Accepting every changed side of a non-conflicting merge gives what git merges.
    #[test]
    fn non_conflicting_changes_merge_cleanly() {
        let base = "using System;\n\nclass A\n{\n    int x = 1;\n    void F() { }\n}\n";
        let ours = "using System;\nusing System.Linq;\n\nclass A\n{\n    int x = 1;\n    void F() { }\n}\n";
        let theirs = "using System;\n\nclass A\n{\n    int x = 2;\n    void F() { }\n    void G() { }\n}\n";
        let mut m = MergeModel::new(base, ours, theirs);
        assert_eq!(m.result_text(), base);
        assert_eq!(m.unresolved(), 3);
        assert_eq!(m.unresolved_conflicts(), 0);
        let edits = m.apply_non_conflicting(None);
        assert_eq!(edits.len(), 3);
        assert_eq!(m.unresolved(), 0);
        assert_eq!(m.result_text(), "using System;\nusing System.Linq;\n\nclass A\n{\n    int x = 2;\n    void F() { }\n    void G() { }\n}\n");
        // Replaying the edits on the base gives the same text.
        let mut text = lines(base);
        for e in edits {
            text.splice(e.lines, lines(&e.text));
        }
        assert_eq!(text.concat(), m.result_text());
    }

    #[test]
    fn non_conflicting_changes_of_one_side_only() {
        let mut m = MergeModel::new("a\nb\nc\n", "A\nb\nc\n", "a\nb\nC\n");
        m.apply_non_conflicting(Some(Side::Theirs));
        assert_eq!(m.result_text(), "a\nb\nC\n");
        assert_eq!(m.unresolved(), 1);
        assert!(m.can_accept(0, Side::Ours));
        assert!(!m.can_accept(0, Side::Theirs));
    }

    #[test]
    fn conflicts_take_one_side_then_append_the_other() {
        let mut m = MergeModel::new("a\nb\nc\nd\n", "a\nours\nc\nD\n", "a\ntheirs1\ntheirs2\nc\nd\n");
        assert_eq!(m.chunks.iter().map(|c| c.kind).collect::<Vec<_>>(), [ChunkKind::Conflict, ChunkKind::Ours]);
        assert!(m.apply_non_conflicting(None).len() == 1);
        assert_eq!(m.unresolved_conflicts(), 1);
        let e = m.accept(0, Side::Theirs).unwrap();
        assert_eq!(e, ResultEdit { lines: 1..2, text: "theirs1\ntheirs2\n".into() });
        assert_eq!(m.result_text(), "a\ntheirs1\ntheirs2\nc\nD\n");
        assert_eq!(m.result_range(0), 1..3);
        assert_eq!(m.result_range(1), 4..5);
        assert!(m.can_accept(0, Side::Ours));
        let e = m.accept(0, Side::Ours).unwrap();
        assert_eq!(e, ResultEdit { lines: 3..3, text: "ours\n".into() });
        assert_eq!(m.resolution(0), Resolution::Both(Side::Theirs));
        assert_eq!(m.result_text(), "a\ntheirs1\ntheirs2\nours\nc\nD\n");
        assert_eq!(m.result_range(1), 5..6);
        assert!(m.accept(0, Side::Ours).is_none());
        assert_eq!(m.unresolved(), 0);
    }

    #[test]
    fn ignoring_keeps_the_base() {
        let mut m = MergeModel::new("a\nb\n", "a\nx\n", "a\ny\n");
        m.ignore(0);
        assert_eq!(m.resolution(0), Resolution::Ignored);
        assert_eq!(m.result_text(), "a\nb\n");
        assert!(!m.can_accept(0, Side::Ours));
    }

    #[test]
    fn hand_edits_move_chunks_and_resolve_the_edited_ones() {
        let mut m = MergeModel::new("1\n2\n3\n4\n5\n6\n", "1\nA\n3\n4\n5\nB\n", "1\nX\n3\n4\n5\nY\n");
        assert_eq!((m.result_range(0), m.result_range(1)), (1..2, 5..6));
        // Two lines inserted above both chunks.
        m.sync("0\n0\n1\n2\n3\n4\n5\n6\n");
        assert_eq!((m.result_range(0), m.result_range(1)), (3..4, 7..8));
        assert_eq!(m.unresolved(), 2);
        // Editing the first chunk's line resolves it by hand; the second only moves.
        m.sync("0\n0\n1\nmine\nmine too\n3\n4\n5\n6\n");
        assert_eq!(m.resolution(0), Resolution::Edited);
        assert_eq!(m.result_range(0), 3..5);
        assert_eq!(m.result_range(1), 8..9);
        assert_eq!(m.resolution(1), Resolution::Unresolved);
        // Deleting a line between them moves only the second.
        m.sync("0\n0\n1\nmine\nmine too\n3\n5\n6\n");
        assert_eq!(m.result_range(1), 7..8);
        assert_eq!(m.resolution(1), Resolution::Unresolved);
        // Accepting after hand edits still targets the right lines.
        let e = m.accept(1, Side::Theirs).unwrap();
        assert_eq!(e.lines, 7..8);
        assert_eq!(m.result_text(), "0\n0\n1\nmine\nmine too\n3\n5\nY\n");
    }

    #[test]
    fn typing_into_an_empty_chunk_resolves_it() {
        let mut m = MergeModel::new("a\nb\n", "a\no\nb\n", "a\nt\nb\n");
        assert_eq!(m.result_range(0), 1..1);
        m.sync("a\nmanual\nb\n");
        assert_eq!(m.result_range(0), 1..2);
        assert_eq!(m.resolution(0), Resolution::Edited);
    }

    #[test]
    fn remap_is_stable_without_overlap() {
        let ch = |a: Range<usize>, b: Range<usize>| textdiff::Change { old: a, new: b };
        assert_eq!(remap(2..4, &[ch(0..0, 0..3)]), (5..7, false));
        assert_eq!(remap(2..4, &[ch(5..6, 5..5)]), (2..4, false));
        assert_eq!(remap(2..4, &[ch(1..3, 1..2)]), (1..3, true));
        assert_eq!(remap(2..2, &[ch(2..2, 2..4)]), (2..4, true));
    }
}
