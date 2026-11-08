//! TODO tool window: the `TODO` and `FIXME` comments of the workspace (Rider's default
//! patterns), found with the Find in Files engine in the background.

use rc_core::{Inbox, Waker};
use rc_search::{FindEvent, FindOptions, Finder, QueryOptions};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub struct TodoItem {
    pub line: usize,
    pub col: usize,
    /// From the keyword to the end of the comment.
    pub text: String,
    pub fixme: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TodoFile {
    pub path: PathBuf,
    pub rel: String,
    pub items: Vec<TodoItem>,
}

pub struct Todos {
    pub files: Vec<TodoFile>,
    pub scanning: bool,
    /// Show only the active file's (Rider's "Current File" tab).
    pub current_file: bool,
    /// Time of the last scan (egui time), to rescan when the window opens again.
    pub scanned_at: Option<f64>,
    finder: Finder,
    inbox: Inbox<FindEvent>,
    pending: Vec<TodoFile>,
}

impl Todos {
    pub fn new(root: &Path, waker: Waker) -> Self {
        let inbox = Inbox::new(waker);
        Todos { files: Vec::new(), scanning: false, current_file: false, scanned_at: None, finder: Finder::new(root, inbox.sender()), inbox, pending: Vec::new() }
    }

    pub fn refresh(&mut self, now: f64) {
        let query = QueryOptions { pattern: r"\b(todo|fixme)\b".into(), regex: true, case_sensitive: false, whole_word: false };
        if self.finder.start(FindOptions { query, max_results: 5000, ..Default::default() }).is_ok() {
            self.scanning = true;
            self.pending.clear();
            self.scanned_at = Some(now);
        }
    }

    pub fn poll(&mut self) {
        let current = self.finder.current();
        for ev in self.inbox.drain() {
            if Some(ev.search()) != current {
                continue;
            }
            match ev {
                FindEvent::File { file, .. } => {
                    let items: Vec<TodoItem> = file
                        .lines
                        .iter()
                        .filter_map(|l| {
                            let (col, text) = l.ranges.iter().find_map(|r| todo_in_comment(&l.text, r.start as usize))?;
                            Some(TodoItem { line: l.line as usize, col, fixme: text.to_ascii_lowercase().starts_with("fixme"), text })
                        })
                        .collect();
                    if !items.is_empty() {
                        self.pending.push(TodoFile { path: file.path, rel: file.rel, items });
                    }
                }
                FindEvent::Done { .. } => {
                    let mut files = std::mem::take(&mut self.pending);
                    files.sort_by_key(|f| f.rel.to_lowercase());
                    self.files = files;
                    self.scanning = false;
                }
            }
        }
    }

    pub fn count(&self) -> usize {
        self.files.iter().map(|f| f.items.len()).sum()
    }
}

/// The TODO starting at char `start` of `line` when it sits in a comment: its column and text
/// (without the comment's closing `*/`, `-->` or `*@`).
pub fn todo_in_comment(line: &str, start: usize) -> Option<(usize, String)> {
    let byte = line.char_indices().nth(start).map_or(line.len(), |(b, _)| b);
    let before = &line[..byte];
    let trimmed = before.trim_start();
    let in_comment = ["//", "/*", "<!--", "@*", "#", "--", "'"].iter().any(|m| before.contains(m)) || trimmed.starts_with('*');
    if !in_comment {
        return None;
    }
    let mut text = line[byte..].trim_end().to_string();
    for close in ["*/", "-->", "*@"] {
        if let Some(i) = text.find(close) {
            text.truncate(i);
        }
    }
    Some((start, text.trim_end().to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_comments_count() {
        assert_eq!(todo_in_comment("    // TODO: validate input", 7), Some((7, "TODO: validate input".into())));
        assert_eq!(todo_in_comment("x = 1; /* FIXME later */", 10), Some((10, "FIXME later".into())));
        assert_eq!(todo_in_comment("     * todo something", 7), Some((7, "todo something".into())));
        assert_eq!(todo_in_comment("<!-- TODO: header -->", 5), Some((5, "TODO: header".into())));
        assert_eq!(todo_in_comment("var todo = new Todo();", 4), None);
    }

    #[test]
    fn scans_a_folder() {
        let dir = tempfile::tempdir().unwrap();
        let root = dunce::canonicalize(dir.path()).unwrap();
        std::fs::write(root.join("A.cs"), "class A {\n    // TODO: split this class\n    var todo = 1; // FIXME: rename\n}\n").unwrap();
        std::fs::write(root.join("B.cs"), "class B { }\n").unwrap();
        let mut t = Todos::new(&root, Waker::noop());
        t.refresh(0.0);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while t.scanning && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(20));
            t.poll();
        }
        assert_eq!(t.files.len(), 1);
        let items = &t.files[0].items;
        assert_eq!(items.len(), 2, "{items:?}");
        assert_eq!(items[0].text, "TODO: split this class");
        assert!(items[1].fixme && items[1].text == "FIXME: rename");
    }
}
