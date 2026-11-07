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
    detected_indent: rc_text::commands::Indent,
    style_revision: u64,
    resolved_style: rc_core::editorconfig::Indentation,
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

    fn apply_style(&mut self) {
        let base = &self.detected_indent;
        let style = &self.resolved_style;
        let tabs = style.tabs.unwrap_or(base.uses_tabs());
        let tab_width = style.tab_width.unwrap_or(base.tab_width).max(1);
        let size = style.size.unwrap_or_else(|| if tabs || style.size_uses_tab_width { tab_width } else { base.width() });
        self.editor.indent = rc_text::commands::Indent {
            unit: if tabs { format!("{}{}", "\t".repeat(size / tab_width), " ".repeat(size % tab_width)) } else { " ".repeat(size) },
            tab_width,
            prefer_tabs: tabs,
        };
    }

    fn reload(&mut self, text: &str) {
        self.editor.reload(text);
        self.detected_indent = rc_editor::state::detect_indent(&self.editor.buffer);
        // Keep the current configuration effective while its refresh is in flight.
        self.apply_style();
    }
}

pub struct Docs {
    pub list: Vec<Doc>,
    pub active: Option<usize>,
    next_id: u64,
    pub pool: ParserPool,
    /// Recently opened files, most recent first (⌘E).
    pub recent: Vec<PathBuf>,
    styles: StyleWorker,
}

/// A single background reader per document collection; no filesystem work on the UI thread.
struct StyleWorker {
    requests: std::sync::mpsc::Sender<(u64, u64, PathBuf)>,
    results: rc_core::Inbox<(u64, u64, rc_core::editorconfig::Indentation)>,
    cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl StyleWorker {
    fn new(waker: rc_core::Waker) -> Self {
        let (requests, queue) = std::sync::mpsc::channel::<(u64, u64, PathBuf)>();
        let results = rc_core::Inbox::new(waker);
        let out = results.sender();
        let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stop = cancelled.clone();
        std::thread::spawn(move || {
            while let Ok((id, revision, path)) = queue.recv() {
                if let Some(style) = rc_core::editorconfig::for_file(&path, &stop) {
                    if !out.send((id, revision, style)) { break; }
                } else { break; }
            }
        });
        Self { requests, results, cancelled }
    }
}

impl Drop for StyleWorker {
    fn drop(&mut self) {
        self.cancelled.store(true, std::sync::atomic::Ordering::Relaxed);
    }
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
        Self::with_waker(pool, rc_core::Waker::noop())
    }

    pub fn with_waker(pool: ParserPool, waker: rc_core::Waker) -> Self {
        Docs { list: Vec::new(), active: None, next_id: 1, pool, recent: Vec::new(), styles: StyleWorker::new(waker) }
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
        let detected_indent = editor.indent.clone();
        let _ = self.styles.requests.send((id, 0, path.clone()));
        self.list.push(Doc { detected_indent, style_revision: 0, resolved_style: Default::default(), editor, path: path.clone(), mtime, bom, pinned: false, preview: false, conflict: false, md_view });
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
                self.list[o] = doc;
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
        self.active = match self.active {
            _ if self.list.is_empty() => None,
            Some(a) if a > i => Some(a - 1),
            Some(a) if a == i => Some(i.saturating_sub(1).min(self.list.len() - 1)),
            other => other,
        };
        Some(doc)
    }

    pub fn save(&mut self, i: usize) -> Result<(), String> {
        let doc = &mut self.list[i];
        let mut bytes = Vec::new();
        if doc.bom {
            bytes.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
        }
        bytes.extend_from_slice(doc.editor.buffer.to_disk_string().as_bytes());
        rc_core::paths::write_atomic(&doc.path, &bytes).map_err(|e| trf("Could not save {file}: {error}", &[("file", &doc.name()), ("error", &e)]))?;
        doc.editor.buffer.mark_saved();
        doc.mtime = std::fs::metadata(&doc.path).and_then(|m| m.modified()).ok();
        doc.conflict = false;
        Ok(())
    }

    /// Saves every modified document; returns the errors.
    pub fn save_all(&mut self) -> Vec<String> {
        let modified: Vec<usize> = (0..self.list.len()).filter(|&i| self.list[i].is_modified()).collect();
        modified.into_iter().filter_map(|i| self.save(i).err()).collect()
    }

    /// A file changed on disk: unmodified documents reload, modified ones are flagged.
    pub fn on_disk_change(&mut self, path: &Path) {
        if path.file_name().is_some_and(|name| name == ".editorconfig") {
            self.refresh_styles();
        }
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
        doc.reload(&text);
        doc.bom = bom;
        doc.mtime = mtime;
        doc.editor.request_parse(&self.pool);
        Self::request_style(&self.styles, doc);
    }

    /// Checks every open document against the disk (the window got the focus back: catches
    /// changes the watcher could not see).
    pub fn check_disk_changes(&mut self) {
        self.refresh_styles();
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
        doc.reload(&text);
        doc.bom = bom;
        doc.mtime = mtime;
        doc.conflict = false;
        doc.editor.request_parse(&self.pool);
        Self::request_style(&self.styles, doc);
        Ok(())
    }

    /// Re-read settings after config changes or focus restoration. Stale results are ignored.
    fn refresh_styles(&mut self) {
        for doc in &mut self.list {
            Self::request_style(&self.styles, doc);
        }
    }

    fn request_style(worker: &StyleWorker, doc: &mut Doc) {
        doc.style_revision = doc.style_revision.wrapping_add(1);
        let _ = worker.requests.send((doc.editor.id, doc.style_revision, doc.path.clone()));
    }

    /// Open documents follow file/folder renames, including the rules for their new paths.
    pub fn follow_rename(&mut self, old: &Path, new: &Path) {
        for doc in &mut self.list {
            if let Ok(rest) = doc.path.strip_prefix(old) {
                doc.path = new.join(rest);
                doc.editor.path = Some(doc.path.clone());
                doc.resolved_style = Default::default();
                doc.detected_indent = rc_editor::state::detect_indent(&doc.editor.buffer);
                doc.apply_style();
                Self::request_style(&self.styles, doc);
            }
        }
    }

    fn poll_styles(&mut self) -> bool {
        let styles = self.styles.results.drain();
        let any = !styles.is_empty();
        for (id, revision, style) in styles {
            let Some(doc) = self.list.iter_mut().find(|d| d.editor.id == id && d.style_revision == revision) else { continue };
            doc.resolved_style = style;
            doc.apply_style();
        }
        any
    }

    /// Takes finished background parses; returns whether any arrived (repaint).
    pub fn poll_parses(&mut self) -> bool {
        let results = self.pool.poll();
        let any = !results.is_empty() | self.poll_styles();
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
    fn editorconfig_tab_sized_spaces_use_the_editor_tab_width() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("App.cs");
        std::fs::write(&path, "class App {\n  int value;\n}\n").unwrap();
        std::fs::write(dir.path().join(".editorconfig"), "root=true\n[*.cs]\nindent_style=space\nindent_size=tab\n").unwrap();
        let (wake, frames) = std::sync::mpsc::channel();
        let mut docs = Docs::with_waker(ParserPool::new(Arc::new(|| {})), rc_core::Waker::new(move || { let _ = wake.send(()); }));
        docs.open(&path).unwrap();
        frames.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        docs.poll_parses();
        let editor = &mut docs.list[0].editor;
        assert_eq!(editor.indent.tab_width, 4);
        assert_eq!(editor.indent.unit, "    ");
        assert!(!editor.indent.uses_tabs());
        assert!(!editor.buffer.is_modified());
        let edit = rc_text::commands::insert_tab(editor.buffer.rope(), &editor.selection, &editor.indent);
        editor.apply(edit, rc_text::EditKind::Other);
        assert!(editor.buffer.rope().to_string().starts_with("    class"));
        editor.undo();
        assert!(!editor.buffer.is_modified());
    }

    #[test]
    fn editorconfig_mixed_indent_sizes_drive_newline_tab_and_undo() {
        for (size, width, expected) in [(4, 3, "\t "), (5, 2, "\t\t "), (6, 3, "\t\t"), (2, 4, "  ")] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("App.cs");
            std::fs::write(&path, "class App {}\n").unwrap();
            std::fs::write(dir.path().join(".editorconfig"), format!("root=true\n[*.cs]\nindent_style=tab\nindent_size={size}\ntab_width={width}\n")).unwrap();
            let (wake, frames) = std::sync::mpsc::channel();
            let waker = rc_core::Waker::new(move || { let _ = wake.send(()); });
            let mut docs = Docs::with_waker(ParserPool::new(Arc::new(|| {})), waker);
            docs.open(&path).unwrap();
            frames.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
            docs.poll_parses();
            let editor = &mut docs.list[0].editor;
            assert_eq!(editor.indent.unit, expected);
            assert_eq!(editor.indent.width(), size);
            assert_eq!(editor.indent.tab_width, width);
            assert!(editor.indent.prefer_tabs);
            let edit = rc_text::commands::insert_tab(editor.buffer.rope(), &editor.selection, &editor.indent);
            editor.apply(edit, rc_text::EditKind::Other);
            assert!(editor.buffer.rope().to_string().starts_with(expected));
            editor.undo();
            assert_eq!(editor.buffer.rope().to_string(), "class App {}\n");
            assert!(!editor.buffer.is_modified());
            editor.selection = rc_text::Selection::point(11);
            let edit = rc_text::commands::newline(editor.buffer.rope(), &editor.selection, &editor.indent);
            editor.apply(edit, rc_text::EditKind::Other);
            assert_eq!(editor.buffer.rope().to_string(), format!("class App {{\n{expected}\n}}\n"));
            editor.undo();
            assert_eq!(editor.buffer.rope().to_string(), "class App {}\n");
        }
    }

    #[test]
    fn editorconfig_drives_real_csharp_edits_and_refreshes_without_dirtying_documents() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("PriceCalculator.cs");
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/Shop/src/Shop.Core/PriceCalculator.cs");
        std::fs::copy(fixture, &file).unwrap();
        let config = dir.path().join(".editorconfig");
        std::fs::write(&config, "root=true\n[*.cs]\nindent_style=space\nindent_size=2\n").unwrap();
        let (wake, frames) = std::sync::mpsc::channel();
        let waker = rc_core::Waker::new(move || { let _ = wake.send(()); });
        let mut docs = Docs::with_waker(ParserPool::new(Arc::new(|| {})), waker);
        docs.open(&file).unwrap();
        frames.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        docs.poll_parses();
        let editor = &mut docs.list[0].editor;
        assert_eq!(editor.indent.unit, "  ");
        assert_eq!(editor.indent.tab_width, 2);
        assert!(!editor.buffer.is_modified());
        let edit = rc_text::commands::insert_tab(editor.buffer.rope(), &editor.selection, &editor.indent);
        editor.apply(edit, rc_text::EditKind::Other);
        assert!(editor.buffer.to_disk_string().starts_with("  "));
        editor.undo();
        std::fs::write(&config, "root=true\n[*.cs]\nindent_style=tab\ntab_width=6\n").unwrap();
        docs.on_disk_change(&config);
        frames.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        docs.poll_parses();
        assert_eq!(docs.list[0].editor.indent.unit, "\t");
        assert_eq!(docs.list[0].editor.indent.tab_width, 6);
        std::fs::remove_file(&config).unwrap();
        docs.on_disk_change(&config);
        frames.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        docs.poll_parses();
        assert_eq!(docs.list[0].editor.indent, docs.list[0].detected_indent);
    }

    #[test]
    fn editorconfig_survives_reload_and_renames_and_restores_the_new_disk_baseline() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("App.cs");
        let renamed = dir.path().join("App.json");
        let config = dir.path().join(".editorconfig");
        std::fs::write(&file, "class App {\n    int value;\n}\n").unwrap();
        std::fs::write(&config, "root=true\n[*.cs]\nindent_style=space\nindent_size=2\n[*.json]\nindent_style=tab\ntab_width=3\n").unwrap();
        let (wake, frames) = std::sync::mpsc::channel();
        let waker = rc_core::Waker::new(move || { let _ = wake.send(()); });
        let mut docs = Docs::with_waker(ParserPool::new(Arc::new(|| {})), waker);
        let settle = |docs: &mut Docs| {
            frames.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
            docs.poll_parses();
        };
        docs.open(&file).unwrap();
        settle(&mut docs);
        assert_eq!(docs.list[0].editor.indent.unit, "  ");

        // Automatic reload changes the detected baseline to tabs, but configured spaces win.
        std::fs::write(&file, "class App {\n\tint changed;\n}\n").unwrap();
        let later = SystemTime::now() + std::time::Duration::from_secs(5);
        std::fs::File::options().write(true).open(&file).unwrap().set_modified(later).unwrap();
        docs.on_disk_change(&file);
        assert_eq!(docs.list[0].detected_indent.unit, "\t");
        assert_eq!(docs.list[0].editor.indent.unit, "  ");
        settle(&mut docs);
        assert_eq!(docs.list[0].editor.indent.unit, "  ");
        std::fs::remove_file(&config).unwrap();
        docs.on_disk_change(&config);
        settle(&mut docs);
        assert_eq!(docs.list[0].editor.indent.unit, "\t");

        std::fs::write(&config, "root=true\n[*.cs]\nindent_style=space\nindent_size=2\n[*.json]\nindent_style=tab\ntab_width=3\n").unwrap();
        docs.on_disk_change(&config);
        settle(&mut docs);
        // Explicit reload changes the baseline back to four spaces without dropping rules.
        std::fs::write(&file, "class App {\n    int explicit;\n}\n").unwrap();
        docs.reload_from_disk(0).unwrap();
        assert_eq!(docs.list[0].detected_indent.unit, "    ");
        assert_eq!(docs.list[0].editor.indent.unit, "  ");
        settle(&mut docs);
        assert_eq!(docs.list[0].editor.indent.unit, "  ");

        std::fs::rename(&file, &renamed).unwrap();
        docs.follow_rename(&file, &renamed);
        settle(&mut docs);
        assert_eq!(docs.list[0].path, renamed);
        assert_eq!(docs.list[0].editor.indent.unit, "\t");
        assert_eq!(docs.list[0].editor.indent.tab_width, 3);
        std::fs::remove_file(&config).unwrap();
        docs.on_disk_change(&config);
        settle(&mut docs);
        assert_eq!(docs.list[0].editor.indent.unit, "    ");
        assert!(!docs.list[0].is_modified());
    }

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
