//! Local History (IntelliJ): a copy of a file each time DotCraft saves it or another app changes
//! it, kept for a few days outside the project, to compare and restore without Git.

use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Revisions kept per file.
const MAX_REVISIONS: usize = 100;
/// Older revisions are dropped.
const MAX_AGE: Duration = Duration::from_secs(5 * 24 * 3600);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Revision {
    pub time: SystemTime,
    /// Why it was recorded ("Saved", "External change", "Before restore"…).
    pub label: String,
    pub file: PathBuf,
}

impl Revision {
    pub fn read(&self) -> std::io::Result<String> {
        std::fs::read_to_string(&self.file)
    }
}

fn root() -> PathBuf {
    crate::paths::data_dir().join("history")
}

/// The folder of `path`'s revisions (by a hash of its canonical path).
fn dir_of(root: &Path, path: &Path) -> PathBuf {
    let canon = dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let hash = canon.to_string_lossy().bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3));
    root.join(format!("{hash:016x}"))
}

/// Records `text` as a revision of `path`, unless it equals the last one. Errors are ignored by
/// callers: history is a convenience, never a reason to fail a save.
pub fn record(path: &Path, text: &str, label: &str) -> std::io::Result<()> {
    record_in(&root(), path, text, label)
}

fn record_in(root: &Path, path: &Path, text: &str, label: &str) -> std::io::Result<()> {
    let dir = dir_of(root, path);
    if let Some(last) = revisions_in(root, path).first()
        && last.read().is_ok_and(|t| t == text)
    {
        return Ok(());
    }
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("path.txt"), path.to_string_lossy().as_bytes())?;
    let millis = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis();
    // Labels are short ASCII words; keep them safe for a file name.
    let label: String = label.chars().map(|c| if c.is_ascii_alphanumeric() || c == ' ' { c } else { '_' }).collect();
    // Exclusive creation keeps simultaneous saves in the same millisecond separate.
    let mut n = 0;
    loop {
        let name = if n == 0 {
            format!("{millis}-{label}.txt")
        } else {
            format!("{millis}{n:03}-{label}.txt")
        };
        let path = dir.join(name);
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                if let Err(error) = file.write_all(text.as_bytes()) {
                    drop(file);
                    let _ = std::fs::remove_file(&path);
                    return Err(error);
                }
                break;
            }
            Err(error) if error.kind() == ErrorKind::AlreadyExists => n += 1,
            Err(error) => return Err(error),
        }
    }
    prune(root, path);
    Ok(())
}

/// Revisions of `path`, newest first.
pub fn revisions(path: &Path) -> Vec<Revision> {
    revisions_in(&root(), path)
}

fn revisions_in(root: &Path, path: &Path) -> Vec<Revision> {
    let Ok(entries) = std::fs::read_dir(dir_of(root, path)) else { return Vec::new() };
    let mut out: Vec<((u64, u32), Revision)> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let stem = name.strip_suffix(".txt")?;
            let (stamp, label) = stem.split_once('-')?;
            // Milliseconds (13 digits), then a counter for saves in the same millisecond.
            let (ms, n) = stamp.split_at(stamp.len().min(13));
            let millis: u64 = ms.parse().ok()?;
            let n: u32 = if n.is_empty() { 0 } else { n.parse().ok()? };
            let time = UNIX_EPOCH + Duration::from_millis(millis);
            Some(((millis, n), Revision { time, label: label.to_string(), file: e.path() }))
        })
        .collect();
    out.sort_by_key(|r| std::cmp::Reverse(r.0));
    out.into_iter().map(|(_, r)| r).collect()
}

fn prune(root: &Path, path: &Path) {
    let now = SystemTime::now();
    for (i, r) in revisions_in(root, path).into_iter().enumerate() {
        let old = now.duration_since(r.time).is_ok_and(|age| age > MAX_AGE);
        if i >= MAX_REVISIONS || old {
            let _ = std::fs::remove_file(&r.file);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::sync::{Arc, Barrier};

    #[test]
    fn records_lists_and_skips_duplicates() {
        let home = tempfile::tempdir().unwrap();
        let root = home.path().join("history");
        let file = home.path().join("A.cs");
        std::fs::write(&file, "one").unwrap();
        record_in(&root, &file, "one", "Saved").unwrap();
        record_in(&root, &file, "one", "Saved").unwrap();
        record_in(&root, &file, "two", "External change").unwrap();
        let revs = revisions_in(&root, &file);
        assert_eq!(revs.len(), 2);
        assert_eq!(revs[0].read().unwrap(), "two");
        assert_eq!(revs[0].label, "External change");
        assert_eq!(revs[1].read().unwrap(), "one");
        assert!(revisions_in(&root, &home.path().join("B.cs")).is_empty());
    }

    #[test]
    fn concurrent_dotnet_source_revisions_remain_distinct_and_readable() {
        let home = tempfile::tempdir().unwrap();
        let root = home.path().join("history");
        std::fs::write(home.path().join("App.csproj"), "<Project Sdk=\"Microsoft.NET.Sdk\" />\n").unwrap();
        let source = home.path().join("Program.cs");
        std::fs::write(&source, "class Program {}\n").unwrap();
        let barrier = Arc::new(Barrier::new(24));
        let workers: Vec<_> = (0..24)
            .map(|index| {
                let root = root.clone();
                let source = source.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    let text = format!("class Revision{index} {{}}\n");
                    barrier.wait();
                    record_in(&root, &source, &text, "Saved").unwrap();
                    text
                })
            })
            .collect();
        let expected: BTreeSet<_> = workers.into_iter().map(|worker| worker.join().unwrap()).collect();
        let revisions = revisions_in(&root, &source);
        assert_eq!(revisions.len(), expected.len());
        let actual: BTreeSet<_> = revisions.iter().map(|revision| revision.read().unwrap()).collect();
        assert_eq!(actual, expected);
    }

    #[test]
    fn old_revisions_are_pruned_after_the_retention_limit() {
        let home = tempfile::tempdir().unwrap();
        let root = home.path().join("history");
        let source = home.path().join("Program.cs");
        std::fs::write(&source, "class Program {}\n").unwrap();
        for index in 0..=MAX_REVISIONS {
            record_in(&root, &source, &format!("revision {index}"), "Saved").unwrap();
        }
        let revisions = revisions_in(&root, &source);
        assert_eq!(revisions.len(), MAX_REVISIONS);
        assert_eq!(revisions[0].read().unwrap(), format!("revision {MAX_REVISIONS}"));
        assert!(!revisions.iter().any(|revision| revision.read().unwrap() == "revision 0"));
    }
}
