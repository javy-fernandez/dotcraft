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
}
