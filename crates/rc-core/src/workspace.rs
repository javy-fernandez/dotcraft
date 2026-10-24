//! The open workspace: a folder, or a solution file and its folder. Directory listings for the
//! explorer are read lazily and skip build output and tool folders.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkspaceKind {
    Folder,
    /// A `.sln`, `.slnx` or `.slnf` file.
    Solution(PathBuf),
    /// A lone project file.
    Project(PathBuf),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Workspace {
    /// Folder that contains everything (the solution's folder).
    pub root: PathBuf,
    pub kind: WorkspaceKind,
}

impl Workspace {
    /// Opens `path`: a folder (using the single solution inside it, if there is exactly one),
    /// or a solution / project file.
    pub fn open(path: &Path) -> std::io::Result<Workspace> {
        let path = path.canonicalize()?;
        if path.is_dir() {
            let solutions: Vec<PathBuf> = std::fs::read_dir(&path)?
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_file() && is_solution(p))
                .collect();
            let kind = match solutions.as_slice() {
                [only] => WorkspaceKind::Solution(only.clone()),
                _ => WorkspaceKind::Folder,
            };
            return Ok(Workspace { root: path, kind });
        }
        let root = path.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."));
        let kind = if is_solution(&path) {
            WorkspaceKind::Solution(path)
        } else if is_project(&path) {
            WorkspaceKind::Project(path)
        } else {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "not a folder, solution or project"));
        };
        Ok(Workspace { root, kind })
    }

    /// Display name: the solution / project name or the folder name.
    pub fn name(&self) -> String {
        let p = match &self.kind {
            WorkspaceKind::Folder => &self.root,
            WorkspaceKind::Solution(p) | WorkspaceKind::Project(p) => p,
        };
        p.file_stem().or_else(|| p.file_name()).map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into_owned())
    }

    /// The path the session and the recent list are keyed by.
    pub fn key_path(&self) -> &Path {
        match &self.kind {
            WorkspaceKind::Folder => &self.root,
            WorkspaceKind::Solution(p) | WorkspaceKind::Project(p) => p,
        }
    }

    pub fn solution(&self) -> Option<&Path> {
        match &self.kind {
            WorkspaceKind::Solution(p) => Some(p),
            _ => None,
        }
    }
}

pub fn is_solution(p: &Path) -> bool {
    matches!(ext(p).as_str(), "sln" | "slnx" | "slnf")
}

pub fn is_project(p: &Path) -> bool {
    matches!(ext(p).as_str(), "csproj" | "fsproj" | "vbproj")
}

fn ext(p: &Path) -> String {
    p.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase()
}

/// Folders never shown in the explorer.
pub const HIDDEN_DIRS: &[&str] = &["bin", "obj", ".git", ".vs", ".idea", "node_modules", ".dotcraft", "TestResults"];
const HIDDEN_FILES: &[&str] = &[".DS_Store", "Thumbs.db"];

pub fn is_hidden(name: &str, is_dir: bool) -> bool {
    if is_dir { HIDDEN_DIRS.contains(&name) } else { HIDDEN_FILES.contains(&name) || name.ends_with(".user") || name.ends_with(".suo") }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileNode {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
}

/// Children of `dir`: folders first, then files, each sorted case-insensitively.
pub fn list_dir(dir: &Path) -> Vec<FileNode> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<FileNode> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let is_dir = e.file_type().ok()?.is_dir();
            (!is_hidden(&name, is_dir)).then(|| FileNode { path: e.path(), name, is_dir })
        })
        .collect();
    out.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_with_one_solution_opens_the_solution() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("App.sln"), "").unwrap();
        std::fs::create_dir(dir.path().join("bin")).unwrap();
        std::fs::create_dir(dir.path().join("Src")).unwrap();
        std::fs::write(dir.path().join("a.cs"), "").unwrap();
        let ws = Workspace::open(dir.path()).unwrap();
        assert!(matches!(ws.kind, WorkspaceKind::Solution(_)));
        assert_eq!(ws.name(), "App");
        let names: Vec<String> = list_dir(dir.path()).into_iter().map(|n| n.name).collect();
        assert_eq!(names, vec!["Src", "a.cs", "App.sln"]);
    }
}
