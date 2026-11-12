//! File watching: changes under the workspace (outside build folders) reach the UI as
//! [`FsEvent`]s so open editors reload and the explorer refreshes.

use crate::inbox::Outbox;
use crate::workspace::HIDDEN_DIRS;
use notify::event::{ModifyKind, RenameMode};
use notify::{EventKind, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FsEvent {
    Changed(PathBuf),
    Created(PathBuf),
    Removed(PathBuf),
}

impl FsEvent {
    pub fn path(&self) -> &Path {
        match self {
            FsEvent::Changed(p) | FsEvent::Created(p) | FsEvent::Removed(p) => p,
        }
    }
}

/// Preserve both sides of a rename. Checking whether a path exists is insufficient here:
/// an atomic save can replace the destination before the watcher callback runs.
fn classify_event(event: notify::Event) -> Vec<FsEvent> {
    let kind = event.kind;
    event
        .paths
        .into_iter()
        .enumerate()
        .filter_map(|(index, path)| {
            let change = match kind {
                EventKind::Create(_) => FsEvent::Created(path),
                EventKind::Remove(_) => FsEvent::Removed(path),
                EventKind::Modify(ModifyKind::Name(RenameMode::From)) => FsEvent::Removed(path),
                EventKind::Modify(ModifyKind::Name(RenameMode::To)) => FsEvent::Created(path),
                EventKind::Modify(ModifyKind::Name(RenameMode::Both)) if index == 0 => {
                    FsEvent::Removed(path)
                }
                EventKind::Modify(ModifyKind::Name(RenameMode::Both)) if index == 1 => {
                    FsEvent::Created(path)
                }
                EventKind::Modify(ModifyKind::Name(_)) => {
                    if path.exists() {
                        FsEvent::Created(path)
                    } else {
                        FsEvent::Removed(path)
                    }
                }
                EventKind::Modify(_) => FsEvent::Changed(path),
                _ => return None,
            };
            Some(change)
        })
        .collect()
}

/// Keeps the watcher alive; dropping it stops watching.
pub struct FileWatcher {
    _inner: notify::RecommendedWatcher,
}

impl FileWatcher {
    pub fn new(root: &Path, out: Outbox<FsEvent>) -> notify::Result<FileWatcher> {
        let root_buf = root.to_path_buf();
        let mut w = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            let Ok(ev) = res else { return };
            for e in classify_event(ev) {
                let rel = e.path().strip_prefix(&root_buf).unwrap_or(e.path());
                if rel
                    .components()
                    .any(|c| HIDDEN_DIRS.contains(&c.as_os_str().to_string_lossy().as_ref()))
                {
                    continue;
                }
                out.send(e);
            }
        })?;
        w.watch(root, RecursiveMode::Recursive)?;
        Ok(FileWatcher { _inner: w })
    }

    /// Watches the files directly inside `dirs` (open documents outside the workspace).
    pub fn dirs(dirs: &[PathBuf], out: Outbox<FsEvent>) -> notify::Result<FileWatcher> {
        let mut w = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            let Ok(ev) = res else { return };
            for event in classify_event(ev) {
                out.send(event);
            }
        })?;
        for d in dirs {
            w.watch(d, RecursiveMode::NonRecursive)?;
        }
        Ok(FileWatcher { _inner: w })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inbox::{Inbox, Waker};
    use std::time::{Duration, Instant};

    use notify::event::RenameMode;

    fn await_events(inbox: &Inbox<FsEvent>, expected: &[FsEvent]) -> Vec<FsEvent> {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut seen = Vec::new();
        while Instant::now() < deadline && !expected.iter().all(|event| seen.contains(event)) {
            seen.extend(inbox.drain());
            std::thread::sleep(Duration::from_millis(25));
        }
        assert!(
            expected.iter().all(|event| seen.contains(event)),
            "expected {expected:?}, observed {seen:?}"
        );
        seen
    }

    #[test]
    fn rename_classification_preserves_source_and_destination() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("Old.cs");
        let new = dir.path().join("New.cs");
        std::fs::write(&old, "class Old {}\n").unwrap();
        std::fs::write(&new, "class New {}\n").unwrap();
        let both = notify::Event::new(EventKind::Modify(ModifyKind::Name(RenameMode::Both)))
            .add_path(old.clone())
            .add_path(new.clone());
        assert_eq!(
            classify_event(both),
            vec![FsEvent::Removed(old.clone()), FsEvent::Created(new.clone())]
        );
        let from = notify::Event::new(EventKind::Modify(ModifyKind::Name(RenameMode::From)))
            .add_path(old.clone());
        let to = notify::Event::new(EventKind::Modify(ModifyKind::Name(RenameMode::To)))
            .add_path(new.clone());
        assert_eq!(classify_event(from), vec![FsEvent::Removed(old)]);
        assert_eq!(classify_event(to), vec![FsEvent::Created(new)]);
    }

    #[test]
    fn workspace_watcher_reports_dotnet_renames_and_atomic_replacement() {
        let dir = tempfile::tempdir().unwrap();
        let root = dunce::canonicalize(dir.path()).unwrap();
        let project = root.join("App.csproj");
        std::fs::write(&project, "<Project Sdk=\"Microsoft.NET.Sdk\" />\n").unwrap();
        let source = root.join("Program.cs");
        let renamed = root.join("Main.cs");
        let inbox = Inbox::new(Waker::noop());
        let _watcher = FileWatcher::new(&root, inbox.sender()).unwrap();
        std::fs::write(&source, "class Program {}\n").unwrap();
        await_events(&inbox, &[FsEvent::Created(source.clone())]);
        std::fs::rename(&source, &renamed).unwrap();
        await_events(
            &inbox,
            &[FsEvent::Removed(source), FsEvent::Created(renamed.clone())],
        );

        #[cfg(unix)]
        {
            let replacement = root.join("Main.cs.tmp");
            std::fs::write(&replacement, "class Main {}\n").unwrap();
            await_events(&inbox, &[FsEvent::Created(replacement.clone())]);
            std::fs::rename(&replacement, &renamed).unwrap();
            await_events(
                &inbox,
                &[FsEvent::Removed(replacement), FsEvent::Created(renamed)],
            );
        }
    }

    #[test]
    fn dirs_reports_changes_to_files_in_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        let root = dunce::canonicalize(dir.path()).unwrap();
        let md = root.join("notes.md");
        std::fs::write(&md, "# One\n").unwrap();
        let inbox = Inbox::new(Waker::noop());
        let _w = FileWatcher::dirs(std::slice::from_ref(&root), inbox.sender()).unwrap();
        std::thread::sleep(Duration::from_millis(300));
        std::fs::write(&md, "# Two\n").unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut seen = Vec::new();
        while Instant::now() < deadline && !seen.iter().any(|e: &FsEvent| e.path() == md) {
            seen.extend(inbox.drain());
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(
            seen.iter().any(|e| e == &FsEvent::Changed(md.clone())),
            "{seen:?}"
        );
    }
}
