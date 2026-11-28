//! Git blame in the editor: the annotation column ("Annotate with Git Blame") and Code
//! Vision's author labels. Each file is blamed once per text version and HEAD (the editor's
//! text through `git blame --contents -`, so the lines match what it shows); edits, saves,
//! commits and checkouts make the next request blame it again.

use super::{Vcs, VcsEvent, format_date, short_author};
use crate::docs::Docs;
use rc_core::Runtime;
use rc_kit::i18n::{tr, trf};
use rc_vcs::{Blame, BlameCommit};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Edits settle this long before the file is blamed again.
const SETTLE: Duration = Duration::from_millis(600);

/// What a blame was computed from: the document, its text version and the HEAD generation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct BlameKey {
    pub doc: u64,
    pub version: u64,
    pub generation: u64,
}

#[derive(Default)]
pub struct BlameCache {
    /// The newest blame of each file (an error when git could not blame it).
    entries: HashMap<PathBuf, (BlameKey, Result<Arc<Blame>, String>)>,
    /// Files being blamed, with the key of the request.
    pending: HashMap<PathBuf, BlameKey>,
    /// Bumped when HEAD may have moved (commit, checkout, reset, pull…).
    generation: u64,
    /// Text version of each document and when it was first seen (edits settle first).
    seen: HashMap<u64, (u64, Instant)>,
    /// The blame shown in the annotation column, with its key.
    annotated: Option<(BlameKey, Arc<Blame>)>,
    /// Author labels set in each document: (key, number of labels, lines of the text).
    applied: HashMap<u64, (BlameKey, usize, usize)>,
    /// A wake-up is scheduled for this time (to blame once edits settle).
    wake_at: Option<Instant>,
    /// Full commit messages (`None` while loading).
    messages: HashMap<String, Option<String>>,
    /// Code Vision's author labels are on (set by the app from the settings).
    pub author_lens: bool,
}

/// One author of a range of lines.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RangeAuthor {
    pub name: String,
    /// Time of the author's newest change in the range (Unix seconds).
    pub time: i64,
    pub lines: usize,
    /// The lines are changes not committed yet (shown as "You").
    pub uncommitted: bool,
}

/// A declaration that gets an author label: the label goes on the line of its name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Declaration {
    /// Char offset of the declaration's name (where Roslyn puts its Code Vision).
    pub position: usize,
    pub line: usize,
    /// First and last line of the declaration, with its attributes and doc comment.
    pub start_line: usize,
    pub end_line: usize,
}

/// The authors of lines `start..=end` (0-based) of `blame`, the newest change first.
/// Uncommitted lines count as the current user's, newer than any commit. Authors are told
/// apart by e-mail (by name when there is none).
pub fn range_authors(blame: &Blame, start: usize, end: usize) -> Vec<RangeAuthor> {
    let mut by_key: HashMap<String, RangeAuthor> = HashMap::new();
    let end = end.min(blame.lines.len().saturating_sub(1));
    for line in blame.lines.get(start..=end).unwrap_or_default() {
        let commit = &blame.commits[line.commit];
        let uncommitted = commit.is_uncommitted();
        let key = if uncommitted { String::new() } else if commit.author_email.is_empty() { format!("name:{}", commit.author) } else { commit.author_email.to_lowercase() };
        let time = if uncommitted { i64::MAX } else { commit.author_time };
        let entry = by_key.entry(key).or_insert_with(|| RangeAuthor { name: commit.author.clone(), time, lines: 0, uncommitted });
        entry.lines += 1;
        if time > entry.time {
            entry.time = time;
            entry.name = commit.author.clone();
        }
    }
    let mut authors: Vec<RangeAuthor> = by_key.into_values().collect();
    authors.sort_by(|a, b| b.time.cmp(&a.time).then(b.lines.cmp(&a.lines)).then_with(|| a.name.cmp(&b.name)));
    authors
}

/// The label ("Ana Pérez +1") and tooltip (one author per line) of `authors`.
pub fn author_label(authors: &[RangeAuthor]) -> Option<(String, String)> {
    let name = |a: &RangeAuthor| if a.uncommitted { tr("You").to_string() } else { a.name.clone() };
    let first = authors.first()?;
    let title = match authors.len() {
        1 => name(first),
        n => format!("{} +{}", name(first), n - 1),
    };
    let tooltip = authors
        .iter()
        .map(|a| {
            let when = if a.uncommitted { tr("Not committed yet").to_string() } else { format_date(a.time) };
            let lines = if a.lines == 1 { tr("1 line").to_string() } else { trf("{count} lines", &[("count", &a.lines)]) };
            format!("{}  ·  {when}  ·  {lines}", name(a))
        })
        .collect::<Vec<_>>()
        .join("\n");
    Some((title, tooltip))
}

/// Declarations of a C# syntax tree that get an author label: types, delegates and members
/// with a body or accessors (the ones Roslyn counts usages of), not fields or locals.
pub fn declarations(rope: &rc_text::Rope, tree: &rc_syntax::tree_sitter::Tree) -> Vec<Declaration> {
    fn walk(rope: &rc_text::Rope, node: rc_syntax::tree_sitter::Node, out: &mut Vec<Declaration>) {
        let named = matches!(
            node.kind(),
            "class_declaration"
                | "struct_declaration"
                | "interface_declaration"
                | "record_declaration"
                | "record_struct_declaration"
                | "enum_declaration"
                | "delegate_declaration"
                | "method_declaration"
                | "constructor_declaration"
                | "property_declaration"
                | "event_declaration"
        );
        let name = if named {
            node.child_by_field_name("name")
        } else if node.kind() == "indexer_declaration" {
            let mut cursor = node.walk();
            node.children(&mut cursor).find(|child| child.kind() == "this")
        } else {
            None
        };
        if let Some(name) = name {
            let mut start_line = node.start_position().row;
            // Doc comments (and other comments) right above belong to the declaration.
            let mut previous = node.prev_sibling();
            while let Some(comment) = previous.filter(|p| p.kind() == "comment" && p.end_position().row + 1 >= start_line) {
                start_line = comment.start_position().row;
                previous = comment.prev_sibling();
            }
            let position = rope.byte_to_char(name.start_byte().min(rope.len_bytes()));
            out.push(Declaration { position, line: rope.char_to_line(position), start_line, end_line: node.end_position().row });
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            walk(rope, child, out);
        }
    }
    let mut out = Vec::new();
    walk(rope, tree.root_node(), &mut out);
    out
}

/// The author labels of `declarations` (none when nothing in the file is committed yet).
pub fn author_lenses(blame: &Blame, declarations: &[Declaration]) -> Vec<rc_editor::CodeVision> {
    if blame.commits.iter().all(BlameCommit::is_uncommitted) {
        return Vec::new();
    }
    declarations
        .iter()
        .filter_map(|d| {
            let (title, tooltip) = author_label(&range_authors(blame, d.start_line, d.end_line))?;
            Some(rc_editor::CodeVision { line: d.line, position: d.position, title, kind: rc_editor::CodeVisionKind::Author, tooltip: Some(tooltip) })
        })
        .collect()
}

/// The annotation column's text for each line.
fn annotation_texts(blame: &Blame) -> Vec<String> {
    blame
        .lines
        .iter()
        .map(|l| {
            let c = &blame.commits[l.commit];
            if c.is_uncommitted() { tr("Not committed yet").to_string() } else { format!("{}  {}", format_date(c.author_time), short_author(&c.author)) }
        })
        .collect()
}

/// `2026-10-24 14:05` in the author's time zone (`tz` like `+0200`).
pub fn format_time(secs: i64, tz: &str) -> String {
    let sign = if tz.starts_with('-') { -1 } else { 1 };
    let digits = tz.trim_start_matches(['+', '-']);
    let offset = match (digits.get(..2).and_then(|h| h.parse::<i64>().ok()), digits.get(2..4).and_then(|m| m.parse::<i64>().ok())) {
        (Some(h), Some(m)) => sign * (h * 3600 + m * 60),
        _ => 0,
    };
    let local = secs + offset;
    let minutes = local.rem_euclid(86_400) / 60;
    format!("{} {:02}:{:02}", format_date(local), minutes / 60, minutes % 60)
}

impl Vcs {
    /// Turns the annotation column of `path` on or off.
    pub fn toggle_blame(&mut self, path: &Path, docs: &mut Docs) {
        if self.blame_for.as_deref() == Some(path) {
            self.close_blame(docs);
            return;
        }
        let Some(repo) = &self.repo else { return };
        if repo.rel_path(path).is_none() {
            return;
        }
        self.close_blame(docs);
        self.blame_for = Some(path.to_path_buf());
        if let Some(i) = docs.index_of(path) {
            self.request_blame(docs, i, true);
        }
    }

    /// Closes the annotation column.
    pub fn close_blame(&mut self, docs: &mut Docs) {
        if let Some(path) = self.blame_for.take()
            && let Some(i) = docs.index_of(&path)
        {
            docs.list[i].editor.annotations = None;
        }
        self.blames.annotated = None;
    }

    /// Whether `path` is a file of the repository (blame and its menus apply).
    pub fn in_repository(&self, path: &Path) -> bool {
        self.repo.as_ref().is_some_and(|r| r.rel_path(path).is_some())
    }

    /// The commit of an annotated line of `path`.
    pub fn annotation_commit(&self, path: &Path, line: usize) -> Option<&BlameCommit> {
        if self.blame_for.as_deref() != Some(path) {
            return None;
        }
        self.blames.annotated.as_ref()?.1.commit_of(line)
    }

    /// The full message of `hash`: from the loaded log, or fetched in the background (`None`
    /// until it arrives).
    pub fn commit_message(&mut self, hash: &str) -> Option<String> {
        if let Some(c) = self.log.iter().find(|c| c.hash == hash) {
            return Some(if c.body.trim().is_empty() { c.subject.clone() } else { format!("{}\n\n{}", c.subject, c.body.trim()) });
        }
        if let Some(message) = self.blames.messages.get(hash) {
            return message.clone();
        }
        let repo = self.repo.clone()?;
        self.blames.messages.insert(hash.to_string(), None);
        let out = self.inbox.sender();
        let hash = hash.to_string();
        Runtime::spawn(async move {
            let message = repo.commit_message(&hash).await.unwrap_or_default();
            out.send(VcsEvent::Message(hash, message));
        });
        None
    }

    /// Selects `hash` in the Git log. False when the commit is not in the loaded history.
    pub fn show_commit(&mut self, hash: &str) -> bool {
        if !self.log.iter().any(|c| c.hash == hash) {
            return false;
        }
        self.select_commit(hash);
        self.scroll_to_commit = true;
        (self.tools.visible, self.show_stashes) = (false, false);
        true
    }

    /// HEAD may have moved: blames are computed again on the next request.
    pub(super) fn blame_head_changed(&mut self) {
        self.blames.generation += 1;
    }

    pub(super) fn message_loaded(&mut self, hash: String, message: String) {
        self.blames.messages.insert(hash, Some(message.trim_end().to_string()));
    }

    pub(super) fn blamed(&mut self, path: PathBuf, key: BlameKey, result: Result<Blame, String>) {
        if self.blames.pending.get(&path) == Some(&key) {
            self.blames.pending.remove(&path);
        }
        self.blames.entries.insert(path, (key, result.map(Arc::new)));
    }

    fn key_of(&self, docs: &Docs, i: usize) -> BlameKey {
        let editor = &docs.list[i].editor;
        BlameKey { doc: editor.id, version: editor.buffer.version(), generation: self.blames.generation }
    }

    /// Blames document `i` unless its blame is current or already requested. Unless `now`,
    /// a file blamed before waits until its edits settle.
    fn request_blame(&mut self, docs: &Docs, i: usize, now: bool) {
        let Some(repo) = self.repo.clone() else { return };
        let doc = &docs.list[i];
        let key = self.key_of(docs, i);
        let entry = self.blames.entries.get(&doc.path).map(|(k, _)| *k);
        if entry == Some(key) || self.blames.pending.contains_key(&doc.path) {
            return;
        }
        let since = self.blames.seen.entry(key.doc).or_insert((key.version, Instant::now()));
        if since.0 != key.version {
            *since = (key.version, Instant::now());
        }
        if !now && entry.is_some() && since.1.elapsed() < SETTLE {
            let at = since.1 + SETTLE;
            if self.blames.wake_at.is_none_or(|t| t < Instant::now()) {
                self.blames.wake_at = Some(at);
                let waker = self.waker.clone();
                Runtime::spawn(async move {
                    tokio::time::sleep_until(at.into()).await;
                    waker.wake();
                });
            }
            return;
        }
        let Some(rel) = repo.rel_path(&doc.path) else { return };
        let text = doc.editor.buffer.rope().to_string();
        let path = doc.path.clone();
        self.blames.pending.insert(path.clone(), key);
        let out = self.inbox.sender();
        Runtime::spawn(async move {
            let result = repo.blame(&rel, Some(&text), None, None).await.map_err(|e| e.to_string());
            out.send(VcsEvent::Blamed(path, key, result));
        });
    }

    /// Keeps the annotation column and the author labels of the open files up to date.
    pub(super) fn update_blame(&mut self, docs: &mut Docs, notices: &mut Vec<String>) {
        // The annotated file was closed: so are its annotations.
        if let Some(path) = &self.blame_for
            && docs.index_of(path).is_none()
        {
            self.blame_for = None;
            self.blames.annotated = None;
        }
        if self.repo.is_none() {
            return;
        }
        if let Some(i) = self.blame_for.as_deref().and_then(|p| docs.index_of(p)) {
            self.request_blame(docs, i, false);
            let key = self.key_of(docs, i);
            match self.blames.entries.get(&docs.list[i].path) {
                Some((k, Ok(blame))) if k.doc == key.doc && self.blames.annotated.as_ref().is_none_or(|(shown, _)| shown != k) => {
                    docs.list[i].editor.annotations = Some(annotation_texts(blame));
                    self.blames.annotated = Some((*k, blame.clone()));
                }
                // Git could not blame the file (not committed yet…): say why and close.
                Some((k, Err(e))) if *k == key => {
                    notices.push(e.clone());
                    self.close_blame(docs);
                }
                _ => {}
            }
        }
        self.update_author_lenses(docs);
    }

    fn update_author_lenses(&mut self, docs: &mut Docs) {
        let wanted = if self.blames.author_lens { docs.active.filter(|&i| crate::insight::is_csharp(&docs.list[i]) && self.in_repository(&docs.list[i].path)) } else { None };
        // Labels of other documents (or all, when they are off) go away.
        let stale: Vec<u64> = self.blames.applied.keys().copied().filter(|id| wanted.is_none_or(|i| docs.list[i].editor.id != *id)).collect();
        for id in stale {
            self.blames.applied.remove(&id);
            if let Some(doc) = docs.list.iter_mut().find(|d| d.editor.id == id) {
                doc.editor.code_vision.retain(|v| v.kind != rc_editor::CodeVisionKind::Author);
            }
        }
        let Some(i) = wanted else { return };
        self.request_blame(docs, i, false);
        let key = self.key_of(docs, i);
        let doc = &mut docs.list[i];
        let shown = doc.editor.code_vision.iter().filter(|v| v.kind == rc_editor::CodeVisionKind::Author).count();
        if let Some((applied, count, lines)) = self.blames.applied.get(&key.doc).copied() {
            if applied == key && shown == count {
                return;
            }
            // After an edit the labels stay until they are computed again, unless lines moved.
            if applied.version != key.version && shown > 0 && doc.editor.buffer.len_lines() != lines {
                doc.editor.code_vision.retain(|v| v.kind != rc_editor::CodeVisionKind::Author);
            }
        }
        let Some((k, result)) = self.blames.entries.get(&doc.path) else { return };
        if *k != key || !doc.editor.syntax.is_current() {
            return;
        }
        let Some(tree) = doc.editor.syntax.tree() else { return };
        let lenses = match result {
            Ok(blame) => author_lenses(blame, &declarations(doc.editor.buffer.rope(), tree)),
            Err(_) => Vec::new(),
        };
        doc.editor.code_vision.retain(|v| v.kind != rc_editor::CodeVisionKind::Author);
        let count = lenses.len();
        doc.editor.code_vision.extend(lenses);
        self.blames.applied.insert(key.doc, (key, count, doc.editor.buffer.len_lines()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blame(lines: &[(&str, &str, i64)]) -> Blame {
        let mut b = Blame::default();
        for (i, (author, email, time)) in lines.iter().enumerate() {
            let hash = if author.is_empty() { "0".repeat(40) } else { format!("{:040x}", time) };
            let commit = match b.commits.iter().position(|c| c.hash == hash) {
                Some(c) => c,
                None => {
                    let author = if author.is_empty() { "Not Committed Yet" } else { author };
                    b.commits.push(BlameCommit { hash, author: author.to_string(), author_email: email.to_string(), author_time: *time, ..Default::default() });
                    b.commits.len() - 1
                }
            };
            b.lines.push(rc_vcs::BlameLine { commit, orig_line: i as u32 + 1 });
        }
        b
    }

    #[test]
    fn range_authors_puts_the_newest_change_first_and_counts_lines() {
        let b = blame(&[("Ana", "ana@x", 100), ("Juan Darío Penagos", "juan@x", 300), ("Ana", "ana@x", 200), ("Ana", "ana@x", 100), ("Luis", "luis@x", 50)]);
        let authors = range_authors(&b, 0, 3);
        assert_eq!(authors.iter().map(|a| (a.name.as_str(), a.lines, a.time)).collect::<Vec<_>>(), vec![("Juan Darío Penagos", 1, 300), ("Ana", 3, 200)]);
        let (title, tooltip) = author_label(&authors).unwrap();
        assert_eq!(title, "Juan Darío Penagos +1");
        assert_eq!(tooltip.lines().count(), 2);
        assert!(tooltip.starts_with("Juan Darío Penagos"));
        // One line, one author; ranges past the end are clipped.
        assert_eq!(author_label(&range_authors(&b, 4, 99)).unwrap().0, "Luis");
        assert!(range_authors(&b, 9, 12).is_empty());
        assert!(author_label(&[]).is_none());
    }

    #[test]
    fn range_authors_merges_names_of_one_email_and_counts_uncommitted_lines_as_yours() {
        let b = blame(&[("Ana", "Ana@X", 100), ("Ana López", "ana@x", 200), ("", "not.committed.yet", 0)]);
        let authors = range_authors(&b, 0, 2);
        assert_eq!(authors.len(), 2);
        assert!(authors[0].uncommitted);
        assert_eq!((authors[1].name.as_str(), authors[1].lines), ("Ana López", 2));
        assert_eq!(author_label(&authors).unwrap().0, format!("{} +1", tr("You")));
    }

    #[test]
    fn declarations_cover_their_lines_doc_comments_and_members() {
        let text = "namespace N;\n\n/// <summary>Prices.</summary>\n[Serializable]\npublic class Price\n{\n    private int field;\n\n    // Total.\n    public int Total()\n    {\n        return 1;\n    }\n\n    public int Count { get; set; }\n}\n";
        let rope = rc_text::Rope::from_str(text);
        let tree = rc_syntax::parse_now(rc_syntax::Lang::CSharp, &rope, None).unwrap();
        let found: Vec<(usize, usize, usize)> = declarations(&rope, &tree).iter().map(|d| (d.line, d.start_line, d.end_line)).collect();
        // The class (from its doc comment), Total (with its comment) and Count; not the field.
        assert_eq!(found, vec![(4, 2, 15), (9, 8, 12), (14, 14, 14)]);
        let class = declarations(&rope, &tree)[0];
        assert_eq!(rope.slice(class.position..class.position + 5).to_string(), "Price");
    }

    #[test]
    fn author_lenses_map_declarations_to_their_authors() {
        let text = "class A\n{\n    void M()\n    {\n    }\n}\n";
        let rope = rc_text::Rope::from_str(text);
        let tree = rc_syntax::parse_now(rc_syntax::Lang::CSharp, &rope, None).unwrap();
        let b = blame(&[("Ana", "ana@x", 100), ("Ana", "ana@x", 100), ("Luis", "luis@x", 300), ("Ana", "ana@x", 100), ("Ana", "ana@x", 100), ("Ana", "ana@x", 100)]);
        let lenses = author_lenses(&b, &declarations(&rope, &tree));
        assert_eq!(lenses.iter().map(|l| (l.line, l.title.as_str())).collect::<Vec<_>>(), vec![(0, "Luis +1"), (2, "Luis +1")]);
        assert!(lenses.iter().all(|l| l.kind == rc_editor::CodeVisionKind::Author && l.tooltip.is_some()));
        // A file nothing of which is committed gets no labels.
        let fresh = blame(&[("", "", 0); 6]);
        assert!(author_lenses(&fresh, &declarations(&rope, &tree)).is_empty());
    }

    #[test]
    fn times_follow_the_author_time_zone() {
        assert_eq!(format_time(0, "+0200"), "1970-01-01 02:00");
        assert_eq!(format_time(3600, "-0130"), "1969-12-31 23:30");
        assert_eq!(format_time(90, ""), "1970-01-01 00:01");
    }
}
