//! The workspace file index: every file under the root (honouring `.gitignore`, `.ignore` and
//! global git excludes, skipping build and tool folders), walked in parallel on a background
//! thread and then patched from file-system events. Readers take an immutable
//! [`IndexSnapshot`] (an `Arc`), so searching never holds a lock.

use ignore::{WalkBuilder, WalkState};
use parking_lot::Mutex;
use rc_core::Waker;
use rc_core::watch::FsEvent;
use rc_core::workspace::{HIDDEN_DIRS, is_hidden};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;

/// What a file is, for icons and ranking.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FileKind {
    /// `.cs`
    CSharp,
    /// `.csproj`, `.fsproj`, `.vbproj`, `.proj`
    Project,
    /// `.sln`, `.slnx`, `.slnf`
    Solution,
    Other,
}

impl FileKind {
    /// The kind of a file from its name.
    pub fn from_name(name: &str) -> FileKind {
        let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
        match ext.as_str() {
            "cs" => FileKind::CSharp,
            "csproj" | "fsproj" | "vbproj" | "proj" => FileKind::Project,
            "sln" | "slnx" | "slnf" => FileKind::Solution,
            _ => FileKind::Other,
        }
    }
}

/// One indexed file. Cloning is cheap (the path is shared).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileEntry {
    rel: Arc<str>,
    name_start: u32,
    kind: FileKind,
}

impl FileEntry {
    /// An entry for a root-relative path with `/` separators.
    pub fn new(rel: &str) -> FileEntry {
        let name_start = rel.rfind('/').map_or(0, |i| i + 1) as u32;
        FileEntry { rel: rel.into(), name_start, kind: FileKind::from_name(&rel[name_start as usize..]) }
    }

    /// Path relative to the workspace root, `/`-separated.
    pub fn rel(&self) -> &str {
        &self.rel
    }

    /// The file name.
    pub fn name(&self) -> &str {
        &self.rel[self.name_start as usize..]
    }

    /// The directory part of [`rel`](Self::rel), with its trailing `/` (empty at the root).
    pub fn dir(&self) -> &str {
        &self.rel[..self.name_start as usize]
    }

    pub fn kind(&self) -> FileKind {
        self.kind
    }
}

/// An immutable view of the index: files sorted by relative path.
#[derive(Clone, Debug, Default)]
pub struct IndexSnapshot {
    root: PathBuf,
    files: Vec<FileEntry>,
}

impl IndexSnapshot {
    /// Builds a snapshot from relative paths without touching the disk (tests, benches).
    pub fn from_rel_paths<'a>(root: &Path, rels: impl IntoIterator<Item = &'a str>) -> IndexSnapshot {
        let mut files: Vec<FileEntry> = rels.into_iter().map(FileEntry::new).collect();
        sort_dedup(&mut files);
        IndexSnapshot { root: root.to_path_buf(), files }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// All files, sorted by relative path.
    pub fn files(&self) -> &[FileEntry] {
        &self.files
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// The entry for a relative path, if indexed.
    pub fn get(&self, rel: &str) -> Option<&FileEntry> {
        self.files.binary_search_by(|e| e.rel().cmp(rel)).ok().map(|i| &self.files[i])
    }

    /// Absolute path of an entry.
    pub fn abs_path(&self, entry: &FileEntry) -> PathBuf {
        self.root.join(entry.rel())
    }

    /// The `/`-separated path of `path` relative to the root (`None` outside it).
    pub fn relative(&self, path: &Path) -> Option<String> {
        relative_to(&self.root, path)
    }
}

/// Root-relative, `/`-separated form of `path` (also accepts the canonical form of the root).
pub(crate) fn relative_to(root: &Path, path: &Path) -> Option<String> {
    let rel = match path.strip_prefix(root) {
        Ok(r) => r.to_path_buf(),
        Err(_) => {
            let canon = root.canonicalize().ok()?;
            path.strip_prefix(&canon).ok()?.to_path_buf()
        }
    };
    let mut out = String::new();
    for c in rel.components() {
        match c {
            Component::Normal(s) => {
                if !out.is_empty() {
                    out.push('/');
                }
                out.push_str(&s.to_string_lossy());
            }
            Component::CurDir => {}
            _ => return None,
        }
    }
    Some(out)
}

fn sort_dedup(files: &mut Vec<FileEntry>) {
    files.sort_unstable_by(|a, b| a.rel().cmp(b.rel()));
    files.dedup_by(|a, b| a.rel == b.rel);
}

/// The walker shared by the index and Find in Files: ignore files are honoured (even outside
/// a git repository), symlinks are followed (loops are detected and skipped), build and tool
/// folders are skipped.
pub(crate) fn walk_builder(start: &Path) -> WalkBuilder {
    let mut b = WalkBuilder::new(start);
    b.hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .ignore(true)
        .parents(true)
        .require_git(false)
        .follow_links(true)
        .filter_entry(|e| {
            let is_dir = e.file_type().is_some_and(|t| t.is_dir());
            e.depth() == 0 || !is_hidden(&e.file_name().to_string_lossy(), is_dir)
        });
    b
}

/// Number of walker threads for background work (leaves a core for the UI).
pub(crate) fn walker_threads() -> usize {
    std::thread::available_parallelism().map_or(2, |n| n.get().saturating_sub(1).clamp(1, 8))
}

/// Walks `start` in parallel and returns the files below it relative to `root`; `None` when
/// cancelled.
fn walk_files(root: &Path, start: &Path, cancel: &AtomicBool) -> Option<Vec<FileEntry>> {
    struct Collector<'a> {
        local: Vec<FileEntry>,
        out: &'a Mutex<Vec<FileEntry>>,
    }
    impl Drop for Collector<'_> {
        fn drop(&mut self) {
            self.out.lock().append(&mut self.local);
        }
    }

    let out = Mutex::new(Vec::new());
    walk_builder(start).threads(walker_threads()).build_parallel().run(|| {
        let mut c = Collector { local: Vec::new(), out: &out };
        Box::new(move |res| {
            if cancel.load(Ordering::Relaxed) {
                return WalkState::Quit;
            }
            if let Ok(e) = res
                && e.file_type().is_some_and(|t| t.is_file())
                && let Some(rel) = relative_to(root, e.path())
                && !rel.is_empty()
            {
                c.local.push(FileEntry::new(&rel));
            }
            WalkState::Continue
        })
    });
    if cancel.load(Ordering::Relaxed) {
        return None;
    }
    let mut files = out.into_inner();
    sort_dedup(&mut files);
    Some(files)
}

struct Shared {
    root: PathBuf,
    snapshot: Mutex<Arc<IndexSnapshot>>,
    /// `Some` while a scan runs: events wait here and are replayed on the scan's result.
    pending: Mutex<Option<Vec<FsEvent>>>,
    ready: AtomicBool,
}

/// The live file index of a workspace. Clones share the same index.
#[derive(Clone)]
pub struct FileIndex {
    shared: Arc<Shared>,
}

/// A running scan; dropping it (or [`cancel`](Self::cancel)) stops the walk.
pub struct ScanHandle {
    cancel: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl ScanHandle {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn is_finished(&self) -> bool {
        self.thread.as_ref().is_none_or(|t| t.is_finished())
    }

    /// Blocks until the scan ends (tests, benches).
    pub fn wait(mut self) {
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for ScanHandle {
    fn drop(&mut self) {
        if self.thread.is_some() {
            self.cancel();
        }
    }
}

impl FileIndex {
    /// An empty index for `root`; call [`start_scan`](Self::start_scan) to fill it.
    pub fn new(root: &Path) -> FileIndex {
        let root = root.to_path_buf();
        let snapshot = Arc::new(IndexSnapshot { root: root.clone(), files: Vec::new() });
        FileIndex {
            shared: Arc::new(Shared {
                root,
                snapshot: Mutex::new(snapshot),
                pending: Mutex::new(None),
                ready: AtomicBool::new(false),
            }),
        }
    }

    /// Scans `root` on the calling thread (tests, benches, CLI tools).
    pub fn scan_blocking(root: &Path) -> FileIndex {
        let index = FileIndex::new(root);
        if let Some(files) = walk_files(root, root, &AtomicBool::new(false)) {
            *index.shared.snapshot.lock() = Arc::new(IndexSnapshot { root: root.to_path_buf(), files });
        }
        index.shared.ready.store(true, Ordering::Release);
        index
    }

    /// Walks the workspace on a background thread (parallel walk) and wakes the UI when the
    /// snapshot is replaced. Events applied meanwhile are replayed on the new snapshot.
    pub fn start_scan(&self, waker: Waker) -> ScanHandle {
        let cancel = Arc::new(AtomicBool::new(false));
        *self.shared.pending.lock() = Some(Vec::new());
        let shared = self.shared.clone();
        let flag = cancel.clone();
        let thread = std::thread::Builder::new()
            .name("rc-search-index".into())
            .spawn(move || {
                let files = walk_files(&shared.root, &shared.root, &flag);
                let mut pending = shared.pending.lock();
                let Some(files) = files else {
                    // Cancelled: keep the previous snapshot and apply what arrived meanwhile.
                    let events = pending.take().unwrap_or_default();
                    apply_events(&shared, &events);
                    return;
                };
                *shared.snapshot.lock() = Arc::new(IndexSnapshot { root: shared.root.clone(), files });
                let events = pending.take().unwrap_or_default();
                apply_events(&shared, &events);
                shared.ready.store(true, Ordering::Release);
                drop(pending);
                waker.wake();
            })
            .expect("spawn index thread");
        ScanHandle { cancel, thread: Some(thread) }
    }

    pub fn root(&self) -> &Path {
        &self.shared.root
    }

    /// True once a scan has completed.
    pub fn is_ready(&self) -> bool {
        self.shared.ready.load(Ordering::Acquire)
    }

    /// The current snapshot (cheap: an `Arc` clone under a short lock).
    pub fn snapshot(&self) -> Arc<IndexSnapshot> {
        self.shared.snapshot.lock().clone()
    }

    /// Applies watcher events (absolute paths): created files and folders are added when not
    /// ignored, removed ones dropped (a folder takes its files with it), a rename is a remove
    /// plus a create. Touches the disk for created paths; call off the UI thread for big
    /// batches.
    pub fn apply(&self, events: &[FsEvent]) {
        let mut pending = self.shared.pending.lock();
        if let Some(queue) = pending.as_mut() {
            queue.extend_from_slice(events);
            return;
        }
        apply_events(&self.shared, events);
    }

    /// Records a rename (both paths absolute).
    pub fn rename(&self, from: &Path, to: &Path) {
        self.apply(&[FsEvent::Removed(from.to_path_buf()), FsEvent::Created(to.to_path_buf())]);
    }
}

fn apply_events(shared: &Shared, events: &[FsEvent]) {
    if events.is_empty() {
        return;
    }
    let current = shared.snapshot.lock().clone();
    let mut files = current.files.clone();
    let mut changed = false;
    for ev in events {
        let Some(rel) = relative_to(&shared.root, ev.path()) else { continue };
        if rel.is_empty() || rel.split('/').any(|c| HIDDEN_DIRS.contains(&c)) {
            continue;
        }
        // Rebuilt from the root as given, so a canonical event path compares equal below.
        let path = &shared.root.join(&rel);
        let exists = std::fs::symlink_metadata(path).is_ok();
        if matches!(ev, FsEvent::Removed(_)) || !exists {
            changed |= remove_path(&mut files, &rel);
            continue;
        }
        if path.is_dir() {
            // A modified folder only means its listing changed; its files report themselves.
            if matches!(ev, FsEvent::Created(_))
                && is_included(&shared.root, path, &files)
                && let Some(mut added) = walk_files(&shared.root, path, &AtomicBool::new(false))
            {
                files.append(&mut added);
                sort_dedup(&mut files);
                changed = true;
            }
        } else if let Err(at) = files.binary_search_by(|e| e.rel().cmp(&rel))
            && is_included(&shared.root, path, &files)
        {
            files.insert(at, FileEntry::new(&rel));
            changed = true;
        }
    }
    if changed {
        *shared.snapshot.lock() = Arc::new(IndexSnapshot { root: shared.root.clone(), files });
    }
}

/// Removes `rel` and, if it was a folder, everything below it.
fn remove_path(files: &mut Vec<FileEntry>, rel: &str) -> bool {
    let before = files.len();
    if let Ok(i) = files.binary_search_by(|e| e.rel().cmp(rel)) {
        files.remove(i);
    }
    let prefix = format!("{rel}/");
    let start = files.partition_point(|e| e.rel() < prefix.as_str());
    let end = start + files[start..].iter().take_while(|e| e.rel().starts_with(&prefix)).count();
    files.drain(start..end);
    files.len() != before
}

/// Whether the walker would reach `path`: each level from the deepest folder that already has
/// indexed files down to `path` must survive the ignore rules of its parent folder.
fn is_included(root: &Path, path: &Path, files: &[FileEntry]) -> bool {
    let mut levels = Vec::new();
    let mut cur = path;
    while cur != root {
        let Some(parent) = cur.parent() else { return false };
        levels.push(cur);
        if parent == root {
            break;
        }
        if let Some(rel) = relative_to(root, parent) {
            let prefix = format!("{rel}/");
            let i = files.partition_point(|e| e.rel() < prefix.as_str());
            if files.get(i).is_some_and(|e| e.rel().starts_with(&prefix)) {
                break;
            }
        }
        cur = parent;
    }
    levels.iter().rev().all(|level| {
        let Some(parent) = level.parent() else { return false };
        let mut b = walk_builder(parent);
        b.max_depth(Some(1)).follow_links(false);
        b.build().flatten().any(|e| e.depth() == 1 && e.path() == *level)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_split_name_and_kind() {
        let e = FileEntry::new("src/Shop.Core/OrderService.cs");
        assert_eq!(e.name(), "OrderService.cs");
        assert_eq!(e.dir(), "src/Shop.Core/");
        assert_eq!(e.kind(), FileKind::CSharp);
        assert_eq!(FileEntry::new("Shop.slnx").kind(), FileKind::Solution);
        assert_eq!(FileEntry::new("Shop.slnx").dir(), "");
        assert_eq!(FileEntry::new("a/App.CSPROJ").kind(), FileKind::Project);
        assert_eq!(FileEntry::new("README.md").kind(), FileKind::Other);
    }

    #[test]
    fn removing_a_folder_drops_its_files_only() {
        let mut files: Vec<FileEntry> = ["a/b.cs", "a/b/c.cs", "a/b/d/e.cs", "a/bc.cs"].into_iter().map(FileEntry::new).collect();
        sort_dedup(&mut files);
        assert!(remove_path(&mut files, "a/b"));
        let left: Vec<&str> = files.iter().map(|e| e.rel()).collect();
        assert_eq!(left, ["a/b.cs", "a/bc.cs"]);
        assert!(!remove_path(&mut files, "zzz"));
    }

    #[test]
    fn relative_paths_use_slashes() {
        let snap = IndexSnapshot::from_rel_paths(Path::new("/ws"), ["b.cs", "a/x.cs", "b.cs"]);
        assert_eq!(snap.len(), 2);
        assert_eq!(snap.files()[0].rel(), "a/x.cs");
        assert_eq!(snap.relative(Path::new("/ws/a/x.cs")).as_deref(), Some("a/x.cs"));
        assert_eq!(snap.relative(Path::new("/other/x.cs")), None);
        assert!(snap.get("b.cs").is_some());
    }
}
