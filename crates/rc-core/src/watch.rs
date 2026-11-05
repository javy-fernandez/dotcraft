//! File watching: changes under the workspace (outside build folders) reach the UI as
//! [`FsEvent`]s so open editors reload and the explorer refreshes.

use crate::inbox::Outbox;
use crate::workspace::HIDDEN_DIRS;
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

/// Keeps the watcher alive; dropping it stops watching.
pub struct FileWatcher {
    _inner: notify::RecommendedWatcher,
}

impl FileWatcher {
    pub fn new(root: &Path, out: Outbox<FsEvent>) -> notify::Result<FileWatcher> {
        let root_buf = root.to_path_buf();
        let mut w = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            let Ok(ev) = res else { return };
            for path in ev.paths {
                let rel = path.strip_prefix(&root_buf).unwrap_or(&path);
                if rel.components().any(|c| HIDDEN_DIRS.contains(&c.as_os_str().to_string_lossy().as_ref())) {
                    continue;
                }
                let e = match ev.kind {
                    EventKind::Create(_) => FsEvent::Created(path),
                    EventKind::Remove(_) => FsEvent::Removed(path),
                    EventKind::Modify(notify::event::ModifyKind::Name(_)) => {
                        if path.exists() { FsEvent::Created(path) } else { FsEvent::Removed(path) }
                    }
                    EventKind::Modify(_) => FsEvent::Changed(path),
                    _ => continue,
                };
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
            for path in ev.paths {
                out.send(if path.exists() { FsEvent::Changed(path) } else { FsEvent::Removed(path) });
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
        assert!(seen.iter().any(|e| e == &FsEvent::Changed(md.clone())), "{seen:?}");
    }
}
