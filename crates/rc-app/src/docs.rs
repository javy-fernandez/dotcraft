//! Open documents: one editor per file, opened from disk, saved back with the file's line
//! endings and BOM, reloaded when changed outside, and parsed in the background.

use rc_editor::EditorState;
use rc_kit::i18n::trf;
use rc_syntax::ParserPool;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Files bigger than this open read-only (no syntax tree).
const LARGE_FILE: u64 = 8 * 1024 * 1024;
/// Files bigger than this do not open at all.
const HUGE_FILE: u64 = 64 * 1024 * 1024;

pub struct Doc {
    pub editor: EditorState,
    pub path: PathBuf,
    pub mtime: Option<SystemTime>,
    pub bom: bool,
    pub pinned: bool,
    /// Opened with a single click and not edited yet: the next single-click open replaces it
    /// (title in italics, like Rider and VS Code).
    pub preview: bool,
    /// The file changed on disk while it had unsaved edits.
    pub conflict: bool,
    /// Markdown files: editor, preview or both side by side.
    pub md_view: Option<MdView>,
}

/// How a Markdown file is shown (Rider's editor / editor and preview / preview switch).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MdView {
    Editor,
    Split,
    Preview,
}

impl Doc {
    pub fn name(&self) -> String {
        self.path.file_name().map_or_else(|| self.path.display().to_string(), |n| n.to_string_lossy().into_owned())
    }

    pub fn is_modified(&self) -> bool {
        self.editor.buffer.is_modified()
    }
}

pub struct Docs {
    pub list: Vec<Doc>,
    pub active: Option<usize>,
    next_id: u64,
    pub pool: ParserPool,
    /// Recently opened files, most recent first (⌘E).
    pub recent: Vec<PathBuf>,
    /// Bookmarks of the files that aren't open (open ones live in their editor).
    pub bookmarks: std::collections::BTreeMap<PathBuf, std::collections::BTreeMap<usize, Option<char>>>,
    /// Record Local History on saves and outside changes (off in unit tests: no home folder).
    pub history: bool,
}

/// A bookmark anywhere in the workspace.
#[derive(Clone, Debug, PartialEq)]
pub struct Bookmark {
    pub path: PathBuf,
    pub line: usize,
    pub mnemonic: Option<char>,
}

/// Reads a text file: (text without BOM, had BOM). Binary files are refused.
pub fn read_text(path: &Path) -> Result<(String, bool, Option<SystemTime>), String> {
    let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
    if meta.len() > HUGE_FILE {
        return Err(trf("The file is too large to open ({size} MB)", &[("size", &(meta.len() / 1024 / 1024))]));
    }
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let (bytes, bom) = match bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        Some(rest) => (rest.to_vec(), true),
        None => (bytes, false),
    };
    if bytes.iter().take(8000).any(|&b| b == 0) {
        return Err(rc_kit::i18n::tr("This is a binary file and cannot be edited").to_string());
    }
    let text = match String::from_utf8(bytes) {
        Ok(t) => t,
        Err(e) => String::from_utf8_lossy(e.as_bytes()).into_owned(),
    };
    Ok((text, bom, meta.modified().ok()))
}

impl Docs {
    pub fn new(pool: ParserPool) -> Self {
        Docs { list: Vec::new(), active: None, next_id: 1, pool, recent: Vec::new(), bookmarks: Default::default(), history: false }
    }

    /// Every bookmark: open files' from their editors, the others' from the store; by file and line.
    pub fn all_bookmarks(&self) -> Vec<Bookmark> {
        let mut out: Vec<Bookmark> = self.list.iter().flat_map(|d| d.editor.bookmarks.iter().map(|(l, m)| Bookmark { path: d.path.clone(), line: *l, mnemonic: *m })).collect();
        out.extend(self.bookmarks.iter().flat_map(|(p, b)| b.iter().map(|(l, m)| Bookmark { path: p.clone(), line: *l, mnemonic: *m })));
        out.sort_by(|a, b| a.path.cmp(&b.path).then(a.line.cmp(&b.line)));
        out
    }

    /// Sets or removes the bookmark of `path`:`line`; a mnemonic moves from wherever it was.
    pub fn set_bookmark(&mut self, path: &Path, line: usize, mark: Option<Option<char>>) {
        if let Some(Some(c)) = mark {
            // The old line keeps a plain bookmark.
            let maps = self.list.iter_mut().map(|d| &mut d.editor.bookmarks).chain(self.bookmarks.values_mut());
            for map in maps {
                for m in map.values_mut().filter(|m| **m == Some(c)) {
                    *m = None;
                }
            }
        }
        let map = match self.index_of(path) {
            Some(i) => &mut self.list[i].editor.bookmarks,
            None => self.bookmarks.entry(path.to_path_buf()).or_default(),
        };
        match mark {
            Some(m) => {
                map.insert(line, m);
            }
            None => {
                map.remove(&line);
            }
        }
        self.bookmarks.retain(|_, b| !b.is_empty());
    }

    /// Keeps a closing document's bookmarks.
    fn stash_bookmarks(&mut self, doc: &Doc) {
        if !doc.editor.bookmarks.is_empty() {
            self.bookmarks.insert(doc.path.clone(), doc.editor.bookmarks.clone());
        }
    }

    pub fn index_of(&self, path: &Path) -> Option<usize> {
        self.list.iter().position(|d| d.path == path)
    }

    pub fn active_doc(&self) -> Option<&Doc> {
        self.active.and_then(|i| self.list.get(i))
    }

    pub fn active_doc_mut(&mut self) -> Option<&mut Doc> {
        self.active.and_then(|i| self.list.get_mut(i))
    }

    /// Opens `path` (or switches to it) and makes it active.
    pub fn open(&mut self, path: &Path) -> Result<usize, String> {
        let path = dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        if let Some(i) = self.index_of(&path) {
            self.activate(i);
            return Ok(i);
        }
        let (text, bom, mtime) = read_text(&path)?;
        let id = self.next_id;
        self.next_id += 1;
        let mut editor = EditorState::new(id, Some(path.clone()), &text);
        let large = std::fs::metadata(&path).map(|m| m.len() > LARGE_FILE).unwrap_or(false);
        if large {
            editor.read_only = true;
        } else if text.len() < 64 * 1024 {
            // Small files get their colours on the first frame.
            editor.parse_blocking();
        } else {
            editor.request_parse(&self.pool);
        }
        // Sources Roslyn decompiled from an assembly (⌘+click on a framework type) are read-only.
        if path.components().any(|c| c.as_os_str() == "MetadataAsSource") {
            editor.read_only = true;
        }
        let md_view = path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("md") || e.eq_ignore_ascii_case("markdown")).then_some(MdView::Split);
        if let Some(marks) = self.bookmarks.remove(&path) {
            let lines = editor.buffer.len_lines();
            editor.bookmarks = marks.into_iter().filter(|(l, _)| *l < lines).collect();
        }
        self.list.push(Doc { editor, path: path.clone(), mtime, bom, pinned: false, preview: false, conflict: false, md_view });
        let i = self.list.len() - 1;
        self.activate(i);
        Ok(i)
    }

    /// Single-click open: shows `path` in the preview tab, replacing the previous preview.
    pub fn open_preview(&mut self, path: &Path) -> Result<usize, String> {
        let canon = dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        if let Some(i) = self.index_of(&canon) {
            self.activate(i);
            return Ok(i);
        }
        let old = self.list.iter().position(|d| d.preview && !d.is_modified());
        let i = self.open(path)?;
        self.list[i].preview = true;
        match old {
            Some(o) => {
                let doc = self.list.remove(i);
                let old = std::mem::replace(&mut self.list[o], doc);
                self.stash_bookmarks(&old);
                self.activate(o);
                Ok(o)
            }
            None => Ok(i),
        }
    }

    /// Moves tab `from` to `to` (drag and drop), keeping the active document.
    pub fn move_tab(&mut self, from: usize, to: usize) {
        if from >= self.list.len() || to >= self.list.len() || from == to {
            return;
        }
        let active = self.active.map(|a| self.list[a].editor.id);
        let doc = self.list.remove(from);
        self.list.insert(to, doc);
        self.active = active.and_then(|id| self.list.iter().position(|d| d.editor.id == id));
    }

    pub fn activate(&mut self, i: usize) {
        if i < self.list.len() {
            self.active = Some(i);
            let p = self.list[i].path.clone();
            self.recent.retain(|r| r != &p);
            self.recent.insert(0, p);
            self.recent.truncate(50);
        }
    }

    /// Closes tab `i`; the tab to its left (or the new first) becomes active, like Rider.
    pub fn close(&mut self, i: usize) -> Option<Doc> {
        if i >= self.list.len() {
            return None;
        }
        let doc = self.list.remove(i);
        self.stash_bookmarks(&doc);
        self.active = match self.active {
            _ if self.list.is_empty() => None,
            Some(a) if a > i => Some(a - 1),
            Some(a) if a == i => Some(i.saturating_sub(1).min(self.list.len() - 1)),
            other => other,
        };
        Some(doc)
    }

    pub fn save(&mut self, i: usize) -> Result<(), String> {
        let history = self.history;
        let doc = &mut self.list[i];
        // The first revision is the file as it was before DotCraft touched it.
        if history && rc_core::local_history::revisions(&doc.path).is_empty()
            && let Ok((old, _, _)) = read_text(&doc.path)
        {
            let _ = rc_core::local_history::record(&doc.path, &old, "Original");
        }
        let mut bytes = Vec::new();
        if doc.bom {
            bytes.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
        }
        bytes.extend_from_slice(doc.editor.buffer.to_disk_string().as_bytes());
        rc_core::paths::write_atomic(&doc.path, &bytes).map_err(|e| trf("Could not save {file}: {error}", &[("file", &doc.name()), ("error", &e)]))?;
        doc.editor.buffer.mark_saved();
        doc.mtime = std::fs::metadata(&doc.path).and_then(|m| m.modified()).ok();
        doc.conflict = false;
        if history {
            let _ = rc_core::local_history::record(&doc.path, &doc.editor.buffer.rope().to_string(), "Saved");
        }
        Ok(())
    }

    /// Saves every modified document; returns the errors.
    pub fn save_all(&mut self) -> Vec<String> {
        let modified: Vec<usize> = (0..self.list.len()).filter(|&i| self.list[i].is_modified()).collect();
        modified.into_iter().filter_map(|i| self.save(i).err()).collect()
    }

    /// A file changed on disk: unmodified documents reload, modified ones are flagged.
    pub fn on_disk_change(&mut self, path: &Path) {
        let Some(i) = self.index_of(path) else { return };
        let doc = &mut self.list[i];
        let mtime = std::fs::metadata(path).and_then(|m| m.modified()).ok();
        if mtime.is_some() && mtime == doc.mtime {
            return; // our own save
        }
        let Ok((text, bom, mtime)) = read_text(path) else { return };
        if *doc.editor.buffer.rope() == text.replace("\r\n", "\n") {
            doc.mtime = mtime;
            return;
        }
        if doc.is_modified() {
            doc.conflict = true;
            return;
        }
        if self.history {
            let _ = rc_core::local_history::record(path, &text.replace("\r\n", "\n"), "External change");
        }
        doc.editor.reload(&text);
        doc.bom = bom;
        doc.mtime = mtime;
        doc.editor.request_parse(&self.pool);
    }

    /// Checks every open document against the disk (the window got the focus back: catches
    /// changes the watcher could not see).
    pub fn check_disk_changes(&mut self) {
        let paths: Vec<PathBuf> = self.list.iter().map(|d| d.path.clone()).collect();
        for p in paths {
            self.on_disk_change(&p);
        }
    }

    /// File › Reload from Disk: an unmodified document loads the file again; one with unsaved
    /// edits shows the conflict banner so the user picks which version to keep.
    pub fn reload_from_disk(&mut self, i: usize) -> Result<(), String> {
        let doc = &mut self.list[i];
        let (text, bom, mtime) = read_text(&doc.path)?;
        if doc.is_modified() {
            doc.conflict = true;
            return Ok(());
        }
        doc.editor.reload(&text);
        doc.bom = bom;
        doc.mtime = mtime;
        doc.conflict = false;
        doc.editor.request_parse(&self.pool);
        Ok(())
    }

    /// Takes finished background parses; returns whether any arrived (repaint).
    pub fn poll_parses(&mut self) -> bool {
        let results = self.pool.poll();
        let any = !results.is_empty();
        for r in results {
            if let Some(doc) = self.list.iter_mut().find(|d| d.editor.id == r.doc) {
                doc.editor.syntax.accept(r);
            }
        }
        any
    }

    /// Queues reparses for documents edited this frame.
    pub fn reparse_edited(&mut self) {
        for d in &mut self.list {
            if d.editor.syntax.lang != rc_syntax::Lang::Plain && !d.editor.syntax.is_current() && !d.editor.read_only {
                let pending = d.editor.syntax.tree().is_none() || d.editor.buffer.version() > 0;
                if pending {
                    d.editor.request_parse(&self.pool);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn open_save_keeps_bom_and_crlf_and_closing_activates_the_left_tab() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("A.cs");
        let b = dir.path().join("B.cs");
        std::fs::write(&a, b"\xEF\xBB\xBFclass A {}\r\nclass A2 {}\r\n").unwrap();
        std::fs::write(&b, "class B {}\n").unwrap();
        let mut docs = Docs::new(ParserPool::new(Arc::new(|| {})));
        let ia = docs.open(&a).unwrap();
        let ib = docs.open(&b).unwrap();
        assert_eq!((ia, ib, docs.active), (0, 1, Some(1)));
        assert_eq!(docs.open(&a).unwrap(), 0);
        let d = &mut docs.list[0];
        d.editor.replace(0, 0, "// x\n");
        assert!(d.is_modified());
        docs.save(0).unwrap();
        assert_eq!(std::fs::read(&a).unwrap(), b"\xEF\xBB\xBF// x\r\nclass A {}\r\nclass A2 {}\r\n");
        assert!(!docs.list[0].is_modified());
        docs.active = Some(1);
        docs.close(1);
        assert_eq!(docs.active, Some(0));
    }

    #[test]
    fn binary_files_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x.dll");
        std::fs::write(&p, [0u8, 1, 2, 3]).unwrap();
        assert!(read_text(&p).is_err());
    }

    #[test]
    fn preview_tabs_are_replaced_until_kept_and_tabs_reorder() {
        let dir = tempfile::tempdir().unwrap();
        for f in ["a.cs", "b.cs", "c.cs", "d.cs"] {
            std::fs::write(dir.path().join(f), "class X {}\n").unwrap();
        }
        let mut docs = Docs::new(ParserPool::new(Arc::new(|| {})));
        docs.open(&dir.path().join("a.cs")).unwrap();
        docs.open_preview(&dir.path().join("b.cs")).unwrap();
        assert!(docs.list[1].preview);
        // The next single-click open replaces the preview in place.
        docs.open_preview(&dir.path().join("c.cs")).unwrap();
        assert_eq!(docs.list.iter().map(|d| d.name()).collect::<Vec<_>>(), ["a.cs", "c.cs"]);
        assert_eq!(docs.active, Some(1));
        // Kept (edited or double-clicked): the next preview opens a new tab.
        docs.list[1].preview = false;
        docs.open_preview(&dir.path().join("d.cs")).unwrap();
        assert_eq!(docs.list.len(), 3);
        docs.move_tab(2, 0);
        assert_eq!(docs.list.iter().map(|d| d.name()).collect::<Vec<_>>(), ["d.cs", "a.cs", "c.cs"]);
        assert_eq!(docs.active_doc().unwrap().name(), "d.cs");
    }

    #[test]
    fn bookmarks_survive_closing_and_mnemonics_are_unique() {
        let dir = tempfile::tempdir().unwrap();
        let root = dunce::canonicalize(dir.path()).unwrap();
        let (a, b) = (root.join("A.cs"), root.join("B.cs"));
        std::fs::write(&a, "one\ntwo\nthree\n").unwrap();
        std::fs::write(&b, "x\ny\n").unwrap();
        let mut docs = Docs::new(ParserPool::new(Arc::new(|| {})));
        docs.open(&a).unwrap();
        docs.set_bookmark(&a, 1, Some(Some('1')));
        docs.set_bookmark(&b, 0, Some(None));
        // Closed: kept aside, and back when the file opens again.
        docs.close(0);
        assert_eq!(docs.all_bookmarks().len(), 2);
        let i = docs.open(&a).unwrap();
        assert_eq!(docs.list[i].editor.bookmarks.get(&1), Some(&Some('1')));
        // A mnemonic moves from where it was.
        docs.set_bookmark(&b, 1, Some(Some('1')));
        let marks = docs.all_bookmarks();
        assert_eq!(marks.iter().filter(|m| m.mnemonic == Some('1')).count(), 1);
        assert!(marks.iter().any(|m| m.path == b && m.line == 1 && m.mnemonic == Some('1')));
        assert!(marks.iter().any(|m| m.path == a && m.line == 1 && m.mnemonic.is_none()));
        // Edits above move the bookmark with its line.
        docs.list[i].editor.replace(0, 0, "zero\n");
        assert!(docs.list[i].editor.bookmarks.contains_key(&2));
    }

    #[test]
    fn reload_from_disk_and_focus_check_pick_up_outside_changes() {
        let dir = tempfile::tempdir().unwrap();
        let md = dir.path().join("notes.md");
        std::fs::write(&md, "# One\n").unwrap();
        let mut docs = Docs::new(ParserPool::new(Arc::new(|| {})));
        docs.open(&md).unwrap();
        // Another app rewrites the file (with a different modification time).
        std::fs::write(&md, "# Two\n").unwrap();
        let later = SystemTime::now() + std::time::Duration::from_secs(5);
        std::fs::File::options().write(true).open(&md).unwrap().set_modified(later).unwrap();
        docs.check_disk_changes();
        assert_eq!(docs.list[0].editor.buffer.rope().to_string(), "# Two\n");
        // Explicit reload, even with the same modification time.
        std::fs::write(&md, "# Three\n").unwrap();
        docs.reload_from_disk(0).unwrap();
        assert_eq!(docs.list[0].editor.buffer.rope().to_string(), "# Three\n");
        assert!(!docs.list[0].is_modified());
        // Unsaved edits are not thrown away: the conflict banner asks.
        docs.list[0].editor.replace(0, 0, "x");
        docs.reload_from_disk(0).unwrap();
        assert!(docs.list[0].conflict);
        assert_eq!(docs.list[0].editor.buffer.rope().to_string(), "x# Three\n");
    }
}
