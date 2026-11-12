//! Where DotCraft keeps its files: `~/Library/Application Support/DotCraft` on macOS,
//! `%APPDATA%\DotCraft` on Windows. `DOTCRAFT_HOME` overrides it (tests, portable installs).

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEMP_FILE: AtomicU64 = AtomicU64::new(0);

pub fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("DOTCRAFT_HOME").filter(|v| !v.is_empty()) {
        return PathBuf::from(dir);
    }
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("DotCraft")
}

/// Downloaded tools (the Roslyn language server, netcoredbg).
pub fn tools_dir() -> PathBuf {
    data_dir().join("tools")
}

pub fn sessions_dir() -> PathBuf {
    data_dir().join("sessions")
}

pub fn logs_dir() -> PathBuf {
    data_dir().join("logs")
}

/// Writes `bytes` atomically: a crash halfway never leaves a truncated file.
pub fn write_atomic(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::{ErrorKind, Write};

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file_name = path.file_name().ok_or_else(|| {
        std::io::Error::new(ErrorKind::InvalidInput, "destination has no file name")
    })?;
    let (tmp, mut file) = loop {
        let sequence = NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed);
        let mut name = file_name.to_os_string();
        name.push(format!(".dotcraft-{}-{sequence}.tmp", std::process::id()));
        let tmp = path.with_file_name(name);
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&tmp) {
            Ok(file) => break (tmp, file),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    };
    let result = (|| {
        // Replacing a file must retain its access controls, including private user secrets.
        #[cfg(unix)]
        if let Ok(metadata) = std::fs::metadata(path) {
            file.set_permissions(metadata.permissions())?;
        }
        file.write_all(bytes)?;
        drop(file);
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};

    #[cfg(unix)]
    #[test]
    fn atomic_replacement_preserves_private_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("secrets.json");
        std::fs::write(&file, b"private").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
        write_atomic(&file, b"still private").unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b"still private");
        assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
    }

    #[test]
    fn concurrent_writes_leave_a_complete_file_and_no_temporary_files() {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("session.json");
        let barrier = Arc::new(Barrier::new(12));
        let handles: Vec<_> = (0..12)
            .map(|index| {
                let destination = destination.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    let bytes = vec![b'A' + index; 64 * 1024];
                    barrier.wait();
                    write_atomic(&destination, &bytes).unwrap();
                    bytes
                })
            })
            .collect();
        let candidates: Vec<_> = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect();
        let saved = std::fs::read(&destination).unwrap();
        assert!(
            candidates.contains(&saved),
            "a concurrent write produced mixed content"
        );
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn failed_replacement_removes_its_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("session.json");
        std::fs::create_dir(&destination).unwrap();
        assert!(write_atomic(&destination, b"session").is_err());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}
