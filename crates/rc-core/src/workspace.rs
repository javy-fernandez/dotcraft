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
    /// Session and recent-entry key; folder-opened projects retain the folder key.
    key: PathBuf,
    /// The Godot manifest found while opening the workspace, if present.
    godot_project: Option<PathBuf>,
}

impl Workspace {
    /// Opens `path`: a folder (using its single solution or standalone project), or a
    /// solution / project file. Ambiguous folders remain plain folders.
    pub fn open(path: &Path) -> std::io::Result<Workspace> {
        let path = dunce::canonicalize(path)?;
        if path.is_dir() {
            let files: Vec<PathBuf> = std::fs::read_dir(&path)?
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_file())
                .collect();
            let solutions: Vec<_> = files.iter().filter(|p| is_solution(p)).collect();
            let kind = match solutions.as_slice() {
                [only] => WorkspaceKind::Solution((*only).clone()),
                [] => {
                    let projects: Vec<_> = files.iter().filter(|p| is_project(p)).collect();
                    match projects.as_slice() {
                        [only] => WorkspaceKind::Project((*only).clone()),
                        _ => WorkspaceKind::Folder,
                    }
                }
                _ => WorkspaceKind::Folder,
            };
            let key = match &kind {
                WorkspaceKind::Solution(solution) => solution.clone(),
                _ => path.clone(),
            };
            let godot_project = path.join("project.godot");
            let godot_project = godot_project.is_file().then_some(godot_project);
            return Ok(Workspace { root: path, kind, key, godot_project });
        }
        let root = path.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."));
        let is_godot_manifest = path.file_name().is_some_and(|name| name == "project.godot");
        let key = if is_godot_manifest { root.clone() } else { path.clone() };
        let kind = if is_solution(&path) {
            WorkspaceKind::Solution(path)
        } else if is_project(&path) {
            WorkspaceKind::Project(path)
        } else if is_godot_manifest {
            WorkspaceKind::Folder
        } else {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "not a folder, solution or project"));
        };
        let godot_project = root.join("project.godot");
        let godot_project = godot_project.is_file().then_some(godot_project);
        Ok(Workspace { root, kind, key, godot_project })
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
        &self.key
    }

    pub fn solution(&self) -> Option<&Path> {
        match &self.kind {
            WorkspaceKind::Solution(p) => Some(p),
            _ => None,
        }
    }

    /// Godot manifest present at the workspace root when it was opened.
    pub fn godot_project(&self) -> Option<&Path> {
        self.godot_project.as_deref()
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
pub const HIDDEN_DIRS: &[&str] = &["bin", "obj", ".git", ".vs", ".idea", ".godot", "node_modules", ".dotcraft", "TestResults"];
const HIDDEN_FILES: &[&str] = &[".DS_Store", "Thumbs.db"];

pub fn is_hidden(name: &str, is_dir: bool) -> bool {
    if name == ".git" { return true; }
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
    list_dir_with(dir, false)
}

/// Like [`list_dir`]; with `all` (the explorer's Show All Files) it also lists what
/// [`is_hidden`] hides, except `.git` and the OS's own files.
pub fn list_dir_with(dir: &Path, all: bool) -> Vec<FileNode> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let canonical_dir = dunce::canonicalize(dir).ok();
    let mut out: Vec<FileNode> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let file_type = e.file_type().ok()?;
            let is_dir = file_type.is_dir() || (file_type.is_symlink() && canonical_dir.as_ref().is_some_and(|base| {
                dunce::canonicalize(e.path()).ok().is_some_and(|target| target != *base && target.starts_with(base) && target.is_dir())
            }));
            let shown = if all { name != ".git" && !HIDDEN_FILES.contains(&name.as_str()) } else { !is_hidden(&name, is_dir) };
            shown.then(|| FileNode { path: e.path(), name, is_dir })
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

    #[test]
    fn a_folder_with_one_dotnet_project_opens_it_without_changing_the_folder_session_key() {
        for extension in ["csproj", "fsproj", "vbproj"] {
            let dir = tempfile::tempdir().unwrap();
            let project = dir.path().join(format!("App.{extension}"));
            std::fs::write(&project, "<Project Sdk=\"Microsoft.NET.Sdk\" />\n").unwrap();
            std::fs::write(dir.path().join("Program.cs"), "class Program {}\n").unwrap();
            let root = dunce::canonicalize(dir.path()).unwrap();
            let project = dunce::canonicalize(&project).unwrap();

            let opened_folder = Workspace::open(&root).unwrap();
            assert_eq!(opened_folder.kind, WorkspaceKind::Project(project.clone()));
            assert_eq!(opened_folder.root, root);
            assert_eq!(opened_folder.key_path(), root);
            assert_eq!(opened_folder.name(), "App");

            let opened_file = Workspace::open(&project).unwrap();
            assert_eq!(opened_file.kind, WorkspaceKind::Project(project.clone()));
            assert_eq!(opened_file.key_path(), project);
        }
    }

    #[test]
    fn solution_precedence_and_ambiguous_folders_are_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let root = dunce::canonicalize(dir.path()).unwrap();
        std::fs::write(root.join("App.csproj"), "<Project Sdk=\"Microsoft.NET.Sdk\" />\n").unwrap();
        std::fs::write(root.join("Tests.csproj"), "<Project Sdk=\"Microsoft.NET.Sdk\" />\n").unwrap();
        assert_eq!(Workspace::open(&root).unwrap().kind, WorkspaceKind::Folder);

        let solution = root.join("App.sln");
        std::fs::write(&solution, "Microsoft Visual Studio Solution File, Format Version 12.00\n").unwrap();
        let workspace = Workspace::open(&root).unwrap();
        assert_eq!(workspace.kind, WorkspaceKind::Solution(solution.clone()));
        assert_eq!(workspace.key_path(), solution);

        std::fs::write(root.join("Other.slnx"), "<Solution />\n").unwrap();
        let workspace = Workspace::open(&root).unwrap();
        assert_eq!(workspace.kind, WorkspaceKind::Folder);
        assert_eq!(workspace.key_path(), root);
    }

    #[test]
    fn godot_manifest_opens_as_a_folder_and_hides_generated_cache() {
        let dir = tempfile::tempdir().unwrap();
        let root = dunce::canonicalize(dir.path()).unwrap();
        let manifest = root.join("project.godot");
        std::fs::write(&manifest, "config_version=5\n").unwrap();
        std::fs::create_dir(root.join(".godot")).unwrap();
        let folder = Workspace::open(&root).unwrap();
        let file = Workspace::open(&manifest).unwrap();
        assert_eq!(folder.kind, WorkspaceKind::Folder);
        assert_eq!(file.kind, WorkspaceKind::Folder);
        assert_eq!(folder.key_path(), root);
        assert_eq!(file.key_path(), root);
        assert_eq!(file.godot_project(), Some(manifest.as_path()));
        assert_eq!(folder.godot_project(), Some(manifest.as_path()));
        assert!(!list_dir(&root).iter().any(|entry| entry.name == ".godot"));
        assert!(list_dir_with(&root, true).iter().any(|entry| entry.name == ".godot"));
    }
}
