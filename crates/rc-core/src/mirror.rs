//! Local copies of a remote machine's files. A remote folder is edited through files under a
//! mirror folder here: remote `/home/dev/App/Program.cs` is `<root>/home/dev/App/Program.cs`,
//! so editor tabs, the language server and the debugger see ordinary local paths, and the
//! remote side gets them back with [`Mirror::remote`].

use std::path::{Component, Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mirror {
    pub root: PathBuf,
}

impl Mirror {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Mirror { root: root.into() }
    }

    /// The local copy of an absolute POSIX path of the remote machine.
    pub fn local(&self, remote: &str) -> Option<PathBuf> {
        let rest = remote.strip_prefix('/')?;
        let mut local = self.root.clone();
        for part in rest.split('/').filter(|p| !p.is_empty() && *p != ".") {
            if part == ".." || part.contains('\\') || part.contains(':') && cfg!(windows) {
                return None;
            }
            local.push(part);
        }
        Some(local)
    }

    /// The remote path of a local copy; `None` for files outside the mirror.
    pub fn remote(&self, local: &Path) -> Option<String> {
        let rest = local.strip_prefix(&self.root).ok()?;
        let mut remote = String::new();
        for part in rest.components() {
            match part {
                Component::Normal(name) => {
                    remote.push('/');
                    remote.push_str(&name.to_string_lossy());
                }
                Component::CurDir => {}
                _ => return None,
            }
        }
        Some(if remote.is_empty() { "/".into() } else { remote })
    }

    pub fn contains(&self, local: &Path) -> bool {
        local.starts_with(&self.root)
    }
}

/// A folder name for a host or distribution: letters, digits and `.-_@` kept, the rest `_`.
pub fn folder_name(host: &str) -> String {
    let name: String = host.trim().chars().map(|c| if c.is_ascii_alphanumeric() || ".-_@".contains(c) { c } else { '_' }).collect();
    if name.is_empty() || name.chars().all(|c| c == '.') { "_".into() } else { name }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_paths_map_both_ways() {
        let m = Mirror::new("/tmp/mirror");
        let local = m.local("/home/dev/src/My App/Program.cs").unwrap();
        assert_eq!(local, Path::new("/tmp/mirror/home/dev/src/My App/Program.cs"));
        assert_eq!(m.remote(&local).as_deref(), Some("/home/dev/src/My App/Program.cs"));
        assert_eq!(m.local("/").unwrap(), Path::new("/tmp/mirror"));
        assert_eq!(m.remote(Path::new("/tmp/mirror")).as_deref(), Some("/"));
        assert_eq!(m.local("relative/x"), None);
        assert_eq!(m.local("/home/../etc/passwd"), None);
        assert_eq!(m.remote(Path::new("/tmp/other/x")), None);
        assert!(m.contains(&local));
        assert_eq!(folder_name("dev@build-box"), "dev@build-box");
        assert_eq!(folder_name("Ubuntu 24.04"), "Ubuntu_24.04");
        assert_eq!(folder_name(".."), "_");
    }
}
