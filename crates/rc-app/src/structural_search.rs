//! Background C# structural search and conflict-checked replacement.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use rc_core::Runtime;
use rc_core::inbox::{Inbox, Waker};
use rc_kit::i18n::trf;
use rc_search::structural::{Constraint, Pattern};
use rc_search::{FileKind, IndexSnapshot};

#[derive(Clone)]
pub struct Hit {
    pub path: PathBuf,
    pub start: usize,
    pub end: usize,
    pub line: usize,
    pub before: String,
    pub after: String,
    pub selected: bool,
    original: Arc<String>,
}

enum Event {
    Found(u64, Result<Vec<Hit>, String>),
    Applied(u64, Result<Vec<PathBuf>, String>),
}

pub struct StructuralSearch {
    pub visible: bool,
    pub query: String,
    pub replacement: String,
    pub constraints: BTreeMap<String, Constraint>,
    pub hits: Vec<Hit>,
    pub busy: bool,
    pub applying: bool,
    pub error: Option<String>,
    pub summary: Option<String>,
    generation: u64,
    cancel: Arc<AtomicBool>,
    inbox: Inbox<Event>,
}

impl StructuralSearch {
    pub fn new(waker: Waker) -> Self {
        Self {
            visible: false,
            query: String::new(),
            replacement: String::new(),
            constraints: BTreeMap::new(),
            hits: Vec::new(),
            busy: false,
            applying: false,
            error: None,
            summary: None,
            generation: 0,
            cancel: Arc::new(AtomicBool::new(false)),
            inbox: Inbox::new(waker),
        }
    }

    pub fn placeholder_names(&self) -> Vec<String> {
        let mut names = Vec::new();
        let bytes = self.query.as_bytes();
        let mut index = 0;
        while index < bytes.len() {
            if bytes[index] == b'$' {
                if let Some(end) = bytes[index + 1..].iter().position(|byte| *byte == b'$') {
                    let name = &self.query[index + 1..index + 1 + end];
                    if !name.is_empty() && !names.iter().any(|existing| existing == name) {
                        names.push(name.to_string());
                    }
                    index += end + 2;
                    continue;
                }
            }
            index += 1;
        }
        names
    }

    pub fn search(&mut self, snapshot: IndexSnapshot) {
        self.cancel.store(true, Ordering::Relaxed);
        self.cancel = Arc::new(AtomicBool::new(false));
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.hits.clear();
        self.busy = false;
        self.error = None;
        self.summary = None;
        let pattern = match Pattern::compile(&self.query, &self.constraints) {
            Ok(pattern) => pattern,
            Err(error) => {
                self.error = Some(error);
                return;
            }
        };
        if let Err(error) = pattern.replace(
            &self.replacement,
            &pattern
                .names()
                .iter()
                .map(|name| (name.clone(), String::new()))
                .collect(),
        ) {
            self.error = Some(error);
            return;
        }
        self.busy = true;
        self.applying = false;
        let cancel = self.cancel.clone();
        let sender = self.inbox.sender();
        let replacement = self.replacement.clone();
        Runtime::handle().spawn_blocking(move || {
            let mut hits = Vec::new();
            for file in snapshot.files() {
                if cancel.load(Ordering::Relaxed) {
                    return;
                }
                if file.kind() != FileKind::CSharp {
                    continue;
                }
                let path = snapshot.abs_path(file);
                let Ok(metadata) = std::fs::metadata(&path) else {
                    continue;
                };
                if metadata.len() > 2 * 1024 * 1024 {
                    continue;
                }
                let Ok(source) = std::fs::read_to_string(&path) else {
                    continue;
                };
                let original = Arc::new(source);
                for found in pattern.find(&original, 1000usize.saturating_sub(hits.len())) {
                    let Ok(after) = pattern.replace(&replacement, &found.captures) else {
                        continue;
                    };
                    let before = original[found.start..found.end].to_string();
                    let line = original[..found.start]
                        .bytes()
                        .filter(|byte| *byte == b'\n')
                        .count()
                        + 1;
                    hits.push(Hit {
                        path: path.clone(),
                        start: found.start,
                        end: found.end,
                        line,
                        before,
                        after,
                        selected: true,
                        original: original.clone(),
                    });
                }
                if hits.len() >= 1000 {
                    break;
                }
            }
            sender.send(Event::Found(generation, Ok(hits)));
        });
    }

    pub fn cancel(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        self.generation = self.generation.wrapping_add(1);
        self.busy = false;
        self.applying = false;
    }

    pub fn invalidate(&mut self) {
        self.cancel();
        self.hits.clear();
        self.error = None;
        self.summary = None;
    }

    pub fn apply(&mut self) {
        if self.busy || self.applying {
            return;
        }
        let selected = self
            .hits
            .iter()
            .filter(|hit| hit.selected)
            .cloned()
            .collect::<Vec<_>>();
        if selected.is_empty() {
            return;
        }
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.applying = true;
        self.error = None;
        let sender = self.inbox.sender();
        Runtime::handle()
            .spawn_blocking(move || sender.send(Event::Applied(generation, apply_hits(&selected))));
    }

    pub fn update(&mut self) -> Vec<PathBuf> {
        let mut changed = Vec::new();
        for event in self.inbox.drain() {
            match event {
                Event::Found(generation, result) if generation == self.generation => {
                    self.busy = false;
                    match result {
                        Ok(hits) => {
                            self.summary =
                                Some(trf("Found {count} matches", &[("count", &hits.len())]));
                            self.hits = hits;
                        }
                        Err(error) => self.error = Some(error),
                    }
                }
                Event::Applied(generation, result) if generation == self.generation => {
                    self.applying = false;
                    match result {
                        Ok(paths) => {
                            self.summary =
                                Some(trf("Updated {count} files", &[("count", &paths.len())]));
                            self.hits.clear();
                            changed = paths;
                        }
                        Err(error) => self.error = Some(error),
                    }
                }
                _ => {}
            }
        }
        changed
    }
}

fn apply_hits(hits: &[Hit]) -> Result<Vec<PathBuf>, String> {
    let mut paths = BTreeSet::new();
    for hit in hits {
        paths.insert(hit.path.clone());
    }
    let mut changes = Vec::new();
    for path in paths {
        let file_hits = hits
            .iter()
            .filter(|hit| hit.path == path)
            .collect::<Vec<_>>();
        let original = &file_hits[0].original;
        let current = std::fs::read_to_string(&path)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        if current != **original {
            return Err(format!("{} changed on disk; search again", path.display()));
        }
        let mut edits = file_hits;
        edits.sort_by_key(|hit| std::cmp::Reverse(hit.start));
        let mut updated = current;
        let mut boundary = updated.len();
        for hit in edits {
            if hit.end > boundary {
                return Err(format!("Overlapping matches in {}", path.display()));
            }
            boundary = hit.start;
            updated.replace_range(hit.start..hit.end, &hit.after);
        }
        changes.push((path, updated));
    }
    let mut written = Vec::new();
    for (path, text) in changes {
        rc_core::paths::write_atomic(&path, text.as_bytes())
            .map_err(|error| format!("{}: {error}", path.display()))?;
        written.push(path);
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacement_rechecks_the_source_before_writing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("A.cs");
        let original = Arc::new("class A { void M() { Foo(x); } }".to_string());
        std::fs::write(&path, &**original).unwrap();
        let hit = Hit {
            path: path.clone(),
            start: 21,
            end: 27,
            line: 1,
            before: "Foo(x)".into(),
            after: "Bar(x)".into(),
            selected: true,
            original,
        };
        std::fs::write(&path, "class A { void M() { Foo(y); } }").unwrap();
        assert!(apply_hits(&[hit]).is_err());
        assert!(std::fs::read_to_string(path).unwrap().contains("Foo(y)"));
    }

    #[test]
    fn replacement_applies_multiple_matches_from_end_of_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("A.cs");
        let source = "class A { void M() { Foo(x); Foo(y); } }".to_string();
        std::fs::write(&path, &source).unwrap();
        let pattern = Pattern::compile("Foo($value$)", &BTreeMap::new()).unwrap();
        let original = Arc::new(source.clone());
        let hits = pattern
            .find(&source, 10)
            .into_iter()
            .map(|found| Hit {
                path: path.clone(),
                start: found.start,
                end: found.end,
                line: 1,
                before: source[found.start..found.end].into(),
                after: pattern.replace("Bar($value$)", &found.captures).unwrap(),
                selected: true,
                original: original.clone(),
            })
            .collect::<Vec<_>>();
        assert_eq!(hits.len(), 2);
        apply_hits(&hits).unwrap();
        let updated = std::fs::read_to_string(path).unwrap();
        assert!(updated.contains("Bar(x); Bar(y)"), "{updated}");
    }
}
