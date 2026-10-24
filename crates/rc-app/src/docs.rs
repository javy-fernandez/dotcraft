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
    /// The file changed on disk while it had unsaved edits.
    pub conflict: bool,
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
        Docs { list: Vec::new(), active: None, next_id: 1, pool, recent: Vec::new() }
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
        let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
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
        self.list.push(Doc { editor, path: path.clone(), mtime, bom, pinned: false, conflict: false });
        let i = self.list.len() - 1;
        self.activate(i);
        Ok(i)
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
        let Some(i) = self.index_of(path) else { return };
        let doc = &mut self.list[i];
        let mtime = std::fs::metadata(path).and_then(|m| m.modified()).ok();
        if mtime.is_some() && mtime == doc.mtime {
            return; // our own save
        }
        let Ok((text, bom, mtime)) = read_text(path) else { return };
        if doc.editor.buffer.rope().to_string() == text.replace("\r\n", "\n") {
            doc.mtime = mtime;
            return;
        }
        if doc.is_modified() {
            doc.conflict = true;
            return;
        }
        doc.editor.reload(&text);
        doc.bom = bom;
        doc.mtime = mtime;
        doc.editor.request_parse(&self.pool);
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
}
