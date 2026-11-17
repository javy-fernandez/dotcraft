//! Structural Search and Replace: C# syntax-tree search over the solution on a worker, results
//! in the Find tool window, replacements as one undoable edit per open document and an atomic
//! write per closed file.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use rc_core::Runtime;
use rc_core::inbox::{Inbox, Waker};
use rc_kit::i18n::{tr, trf};
use rc_search::structural::{Constraint, Pattern, PatternError};
use rc_search::{FileKind, IndexSnapshot};

/// The most matches one search lists.
const MAX_HITS: usize = 1000;
/// Bigger files are generated code; parsing them would stall the search.
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Clone)]
pub struct Hit {
    pub path: PathBuf,
    /// Path relative to the workspace root, for display.
    pub rel: String,
    /// Byte range in `original`.
    pub start: usize,
    pub end: usize,
    /// Zero-based line and character column of the start.
    pub line: usize,
    pub col: usize,
    pub before: String,
    pub after: String,
    pub selected: bool,
    /// The text that was searched: the open document's buffer or the file on disk.
    original: Arc<String>,
}

impl Hit {
    /// The range in characters, as the editor counts.
    fn char_range(&self) -> (usize, usize) {
        let start = self.original[..self.start].chars().count();
        (start, start + self.original[self.start..self.end].chars().count())
    }
}

struct Found {
    hits: Vec<Hit>,
    files: usize,
    truncated: bool,
}

/// What a replacement changed: files (written or edited) and matches, and the first error.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Replaced {
    pub files: usize,
    pub matches: usize,
    pub error: Option<String>,
}

enum Event {
    Found(u64, Found),
    Written(u64, Replaced, Vec<OpenEdits>),
}

pub struct StructuralSearch {
    /// The pattern dialog is open.
    pub dialog: bool,
    pub focus: bool,
    /// The results own the Find tool window (until Find in Files or Find Usages takes it).
    pub shown: bool,
    pub query: String,
    pub replacement: String,
    pub constraints: BTreeMap<String, Constraint>,
    pub hits: Vec<Hit>,
    /// Files with at least one hit.
    pub files: usize,
    pub truncated: bool,
    /// The search had a replacement template: the hits show and apply it.
    pub replacing: bool,
    pub busy: bool,
    pub applying: bool,
    pub error: Option<String>,
    /// Set when a replacement finished and the search should run again.
    pub rerun: bool,
    generation: u64,
    /// Bumped when the workspace closes: replacements finishing later are not reported.
    epoch: u64,
    cancel: Arc<AtomicBool>,
    inbox: Inbox<Event>,
}

impl StructuralSearch {
    pub fn new(waker: Waker) -> Self {
        Self {
            dialog: false,
            focus: false,
            shown: false,
            query: String::new(),
            replacement: String::new(),
            constraints: BTreeMap::new(),
            hits: Vec::new(),
            files: 0,
            truncated: false,
            replacing: false,
            busy: false,
            applying: false,
            error: None,
            rerun: false,
            generation: 0,
            epoch: 0,
            cancel: Arc::new(AtomicBool::new(false)),
            inbox: Inbox::new(waker),
        }
    }

    pub fn placeholder_names(&self) -> Vec<String> {
        rc_search::structural::placeholder_names(&self.query)
    }

    /// Compiles the pattern and checks the replacement against it.
    pub fn compile(&self) -> Result<Pattern, String> {
        let pattern = Pattern::compile(&self.query, &self.constraints).map_err(error_text)?;
        let empty = pattern.names().iter().map(|name| (name.clone(), String::new())).collect();
        pattern.replace(&self.replacement, &empty).map_err(error_text)?;
        Ok(pattern)
    }

    /// Searches every C# file of `snapshot`; `open` holds the text of the open documents,
    /// searched instead of the file on disk.
    pub fn search(&mut self, snapshot: Arc<IndexSnapshot>, open: HashMap<PathBuf, String>) {
        self.stop();
        self.hits.clear();
        self.files = 0;
        self.truncated = false;
        self.error = None;
        self.rerun = false;
        let pattern = match self.compile() {
            Ok(pattern) => pattern,
            Err(error) => {
                self.error = Some(error);
                return;
            }
        };
        self.busy = true;
        let generation = self.generation;
        let cancel = self.cancel.clone();
        let sender = self.inbox.sender();
        let replacement = self.replacement.clone();
        self.replacing = !replacement.is_empty();
        Runtime::handle().spawn_blocking(move || {
            let mut found = Found { hits: Vec::new(), files: 0, truncated: false };
            for file in snapshot.files() {
                if cancel.load(Ordering::Relaxed) {
                    return;
                }
                if file.kind() != FileKind::CSharp {
                    continue;
                }
                let path = snapshot.abs_path(file);
                let source = match open.get(&path) {
                    Some(text) => text.clone(),
                    None => {
                        if std::fs::metadata(&path).map_or(true, |meta| meta.len() > MAX_FILE_BYTES) {
                            continue;
                        }
                        let Ok(text) = std::fs::read_to_string(&path) else { continue };
                        text
                    }
                };
                let original = Arc::new(source);
                let matches = pattern.find(&original, MAX_HITS + 1 - found.hits.len());
                if !matches.is_empty() {
                    found.files += 1;
                }
                for hit in matches {
                    if found.hits.len() == MAX_HITS {
                        found.truncated = true;
                        break;
                    }
                    let Ok(after) = pattern.replace(&replacement, &hit.captures) else { continue };
                    let head = &original[..hit.start];
                    let line_start = head.rfind('\n').map_or(0, |i| i + 1);
                    found.hits.push(Hit {
                        path: path.clone(),
                        rel: file.rel().to_string(),
                        start: hit.start,
                        end: hit.end,
                        line: head.matches('\n').count(),
                        col: head[line_start..].chars().count(),
                        before: original[hit.start..hit.end].to_string(),
                        after,
                        selected: true,
                        original: original.clone(),
                    });
                }
                if found.truncated {
                    break;
                }
            }
            sender.send(Event::Found(generation, found));
        });
    }

    /// Stops the running search (its results are dropped when they arrive).
    pub fn stop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        self.cancel = Arc::new(AtomicBool::new(false));
        self.generation = self.generation.wrapping_add(1);
        self.busy = false;
    }

    /// Forgets everything (the workspace closed). A replacement already writing finishes its
    /// files unreported, leaving the (now closed) editors alone.
    pub fn reset(&mut self) {
        self.stop();
        self.epoch = self.epoch.wrapping_add(1);
        self.hits.clear();
        self.files = 0;
        self.truncated = false;
        self.applying = false;
        self.error = None;
        self.rerun = false;
        self.dialog = false;
        self.shown = false;
    }

    pub fn selected(&self) -> usize {
        self.hits.iter().filter(|hit| hit.selected).count()
    }

    /// Replaces the selected matches: closed files are checked against what was searched and
    /// written atomically on a worker, then each open document gets one undoable edit (left
    /// unsaved, like Replace in Files). Nothing changes when a file changed since the search or
    /// matches overlap. Returns the outcome when no file had to be written.
    pub fn apply(&mut self, docs: &mut crate::docs::Docs) -> Result<Option<Replaced>, String> {
        if self.busy || self.applying {
            return Ok(None);
        }
        let mut by_file: BTreeMap<PathBuf, Vec<&Hit>> = BTreeMap::new();
        for hit in self.hits.iter().filter(|hit| hit.selected) {
            by_file.entry(hit.path.clone()).or_default().push(hit);
        }
        let mut open = Vec::new();
        let mut closed = Vec::new();
        for (path, mut hits) in by_file {
            hits.sort_by_key(|hit| hit.start);
            if hits.windows(2).any(|pair| pair[0].end > pair[1].start) {
                return Err(trf("Overlapping matches in {path}", &[("path", &path.display())]));
            }
            let original = hits[0].original.clone();
            if hits.iter().any(|hit| !Arc::ptr_eq(&hit.original, &original)) {
                return Err(changed(&path));
            }
            match docs.index_of(&path) {
                Some(index) => {
                    if *docs.list[index].editor.buffer.rope() != original.as_str() {
                        return Err(changed(&path));
                    }
                    let changes = hits
                        .iter()
                        .map(|hit| {
                            let (start, end) = hit.char_range();
                            rc_text::Change::new(start, end, hit.after.clone())
                        })
                        .collect();
                    open.push(OpenEdits { path, original, changes });
                }
                None => closed.push((path, hits.iter().map(|hit| (hit.start, hit.end, hit.after.clone())).collect(), original)),
            }
        }
        if closed.is_empty() {
            self.rerun = true;
            return Ok(Some(edit_documents(open, docs, Replaced::default())));
        }
        // The editors change only once the files are written, so a refused write leaves no
        // half-done replacement behind.
        self.applying = true;
        let epoch = self.epoch;
        let sender = self.inbox.sender();
        Runtime::handle().spawn_blocking(move || sender.send(Event::Written(epoch, write_files(closed), open)));
        Ok(None)
    }

    /// Takes finished searches and writes; returns a finished replacement's outcome once.
    pub fn update(&mut self, docs: &mut crate::docs::Docs) -> Option<Replaced> {
        let mut replaced = None;
        for event in self.inbox.drain() {
            match event {
                Event::Found(generation, found) if generation == self.generation => {
                    self.busy = false;
                    self.hits = found.hits;
                    self.files = found.files;
                    self.truncated = found.truncated;
                }
                Event::Written(epoch, written, open) if epoch == self.epoch => {
                    self.applying = false;
                    self.rerun = true;
                    replaced = Some(if written.error.is_some() { written } else { edit_documents(open, docs, written) });
                }
                _ => {}
            }
        }
        replaced
    }
}

/// The edits of one open document, in characters.
struct OpenEdits {
    path: PathBuf,
    original: Arc<String>,
    changes: Vec<rc_text::Change>,
}

/// Applies each document's edits as one undo step, unless it changed since the search.
fn edit_documents(open: Vec<OpenEdits>, docs: &mut crate::docs::Docs, mut replaced: Replaced) -> Replaced {
    for edits in open {
        let Some(index) = docs.index_of(&edits.path) else {
            replaced.error.get_or_insert_with(|| changed(&edits.path));
            continue;
        };
        let doc = &mut docs.list[index];
        if *doc.editor.buffer.rope() != edits.original.as_str() {
            replaced.error.get_or_insert_with(|| changed(&edits.path));
            continue;
        }
        replaced.files += 1;
        replaced.matches += edits.changes.len();
        doc.editor.apply_text_edits(edits.changes);
        doc.editor.request_parse(&docs.pool);
    }
    replaced
}

fn error_text(error: PatternError) -> String {
    match error {
        PatternError::Empty => tr("The pattern is empty").to_string(),
        PatternError::StrayDollar => tr("Write placeholders as $name$").to_string(),
        PatternError::NotCSharp => tr("The pattern is not valid C# code").to_string(),
        PatternError::OnlyPlaceholders => tr("The pattern needs code around its placeholders").to_string(),
        PatternError::Regex { name, error } => trf("Invalid regex for ${name}$: {error}", &[("name", &name), ("error", &error)]),
        PatternError::UnknownPlaceholder(name) => trf("The replacement uses ${name}$, which the pattern does not have", &[("name", &name)]),
    }
}

fn changed(path: &std::path::Path) -> String {
    trf("{path} changed since the search; search again", &[("path", &path.display())])
}

type FileEdits = (PathBuf, Vec<(usize, usize, String)>, Arc<String>);

/// Checks every file against the searched text, then writes them; edits are sorted by start.
fn write_files(files: Vec<FileEdits>) -> Replaced {
    let mut updated = Vec::new();
    for (path, edits, original) in &files {
        match std::fs::read_to_string(path) {
            Ok(current) if current == **original => {}
            Ok(_) => return Replaced { error: Some(changed(path)), ..Default::default() },
            Err(error) => return Replaced { error: Some(format!("{}: {error}", path.display())), ..Default::default() },
        }
        let mut text = String::with_capacity(original.len());
        let mut at = 0;
        for (start, end, after) in edits {
            text.push_str(&original[at..*start]);
            text.push_str(after);
            at = *end;
        }
        text.push_str(&original[at..]);
        updated.push((path, text, edits.len()));
    }
    let mut result = Replaced::default();
    for (path, text, count) in updated {
        if let Err(error) = rc_core::paths::write_atomic(path, text.as_bytes()) {
            result.error = Some(format!("{}: {error}", path.display()));
            break;
        }
        result.files += 1;
        result.matches += count;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn searched(path: &std::path::Path, source: &str, query: &str, replacement: &str) -> Vec<Hit> {
        let pattern = Pattern::compile(query, &BTreeMap::new()).unwrap();
        let original = Arc::new(source.to_string());
        pattern
            .find(source, 10)
            .into_iter()
            .map(|found| Hit {
                path: path.to_path_buf(),
                rel: "A.cs".into(),
                start: found.start,
                end: found.end,
                line: 0,
                col: 0,
                before: source[found.start..found.end].into(),
                after: pattern.replace(replacement, &found.captures).unwrap(),
                selected: true,
                original: original.clone(),
            })
            .collect()
    }

    fn edits(hits: &[Hit]) -> FileEdits {
        (hits[0].path.clone(), hits.iter().map(|hit| (hit.start, hit.end, hit.after.clone())).collect(), hits[0].original.clone())
    }

    #[test]
    fn replacement_rechecks_every_file_before_writing_any() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("A.cs");
        let b = dir.path().join("B.cs");
        let source = "class A { void M() { Foo(x); } }";
        std::fs::write(&a, source).unwrap();
        std::fs::write(&b, source).unwrap();
        let first = searched(&a, source, "Foo($v$)", "Bar($v$)");
        let second = searched(&b, source, "Foo($v$)", "Bar($v$)");
        std::fs::write(&b, "class A { void M() { Foo(y); } }").unwrap();
        let result = write_files(vec![edits(&first), edits(&second)]);
        assert!(result.error.is_some_and(|error| error.contains("B.cs")));
        assert_eq!(std::fs::read_to_string(&a).unwrap(), source, "nothing is written when one file changed");
    }

    #[test]
    fn replacement_keeps_the_text_around_matches() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("A.cs");
        let source = "class A {\r\n    void M() {\r\n        Foo(x);   // keep\r\n        Foo( y );\r\n    }\r\n}\r\n";
        std::fs::write(&path, source).unwrap();
        let hits = searched(&path, source, "Foo($value$)", "Bar($value$)");
        assert_eq!(hits.len(), 2);
        let result = write_files(vec![edits(&hits)]);
        assert_eq!((result.files, result.matches, result.error), (1, 2, None));
        assert_eq!(std::fs::read_to_string(path).unwrap(), source.replace("Foo(x)", "Bar(x)").replace("Foo( y )", "Bar(y)"));
    }

    #[test]
    fn open_documents_get_one_undoable_edit_with_character_offsets() {
        let dir = tempfile::tempdir().unwrap();
        // Canonical, as the workspace index and the editor see it (/var is /private/var on macOS).
        let path = dunce::canonicalize(dir.path()).unwrap().join("A.cs");
        let source = "class A { void M() { var s = \"ñandú\"; Foo(s); Foo(1); } }";
        std::fs::write(&path, source).unwrap();
        let mut docs = crate::docs::Docs::new(rc_syntax::ParserPool::new(Arc::new(|| {})));
        docs.open(&path).unwrap();
        let mut search = StructuralSearch::new(Waker::noop());
        search.hits = searched(&path, source, "Foo($v$)", "Bar($v$)");
        let replaced = search.apply(&mut docs).unwrap();
        assert_eq!(replaced, Some(Replaced { files: 1, matches: 2, error: None }));
        let editor = &mut docs.list[0].editor;
        assert_eq!(editor.buffer.rope().to_string(), source.replace("Foo(", "Bar("));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), source, "open documents are not written");
        editor.undo();
        assert_eq!(editor.buffer.rope().to_string(), source, "one undo restores both matches");
    }

    #[test]
    fn edited_open_documents_and_overlaps_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dunce::canonicalize(dir.path()).unwrap().join("A.cs");
        let source = "class A { void M() { Foo(x); } }";
        std::fs::write(&path, source).unwrap();
        let mut docs = crate::docs::Docs::new(rc_syntax::ParserPool::new(Arc::new(|| {})));
        docs.open(&path).unwrap();
        docs.list[0].editor.replace(0, 0, "// edited\n");
        let mut search = StructuralSearch::new(Waker::noop());
        search.hits = searched(&path, source, "Foo($v$)", "Bar($v$)");
        assert!(search.apply(&mut docs).is_err());
        let mut overlapping = searched(&path, source, "Foo($v$)", "Bar($v$)");
        overlapping.push(overlapping[0].clone());
        search.hits = overlapping;
        assert!(search.apply(&mut docs).unwrap_err().contains("Overlapping"));
    }
}
