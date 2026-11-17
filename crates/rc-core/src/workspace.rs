//! The open workspace: a folder, or a solution file and its folder. Directory listings for the
//! explorer are read lazily and skip build output and tool folders.

use crate::unity::{self, UnityProject};
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
    /// The Unity project at the root (or above it), when there is one.
    pub unity: Option<UnityProject>,
    /// Session and recent-entry key; folder-opened projects retain the folder key.
    key: PathBuf,
    /// The Godot manifest found while opening the workspace, if present.
    godot_project: Option<PathBuf>,
}

impl Workspace {
    /// Opens `path`: a folder (using its single solution or standalone project), or a
    /// solution / project file. Ambiguous folders remain plain folders. A Unity project folder
    /// uses the solution Unity generated, if any; its generated `.csproj` files alone never make
    /// it a standalone project, so it opens as a folder.
    pub fn open(path: &Path) -> std::io::Result<Workspace> {
        let path = dunce::canonicalize(path)?;
        if path.is_dir() {
            let files: Vec<PathBuf> = std::fs::read_dir(&path)?
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_file())
                .collect();
            let unity = UnityProject::find(&path);
            let solutions: Vec<_> = files.iter().filter(|p| is_solution(p)).collect();
            let kind = match solutions.as_slice() {
                [only] => WorkspaceKind::Solution((*only).clone()),
                [] if unity.as_ref().is_some_and(|u| u.root == path) => WorkspaceKind::Folder,
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
            return Ok(Workspace { root: path, kind, unity, key, godot_project });
        }
        let root = path.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."));
        let unity = UnityProject::find(&root);
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
        Ok(Workspace { root, kind, unity, key, godot_project })
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

/// [`is_hidden`] for an entry of `parent`, plus the folders Unity generates when `parent` is a
/// Unity project root (`Library`, `Temp`… stay visible anywhere else).
pub fn is_hidden_in(parent: &Path, name: &str, is_dir: bool) -> bool {
    is_hidden(name, is_dir) || (is_dir && unity::is_generated_dir(parent, name))
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
/// [`is_hidden_in`] hides, except `.git` and the OS's own files.
pub fn list_dir_with(dir: &Path, all: bool) -> Vec<FileNode> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let canonical_dir = dunce::canonicalize(dir).ok();
    // Checked once per listing rather than per entry.
    let unity_root = !all && unity::is_unity_root(dir);
    let mut out: Vec<FileNode> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let file_type = e.file_type().ok()?;
            let is_dir = file_type.is_dir() || (file_type.is_symlink() && canonical_dir.as_ref().is_some_and(|base| {
                dunce::canonicalize(e.path()).ok().is_some_and(|target| target != *base && target.starts_with(base) && target.is_dir())
            }));
            let shown = if all {
                name != ".git" && !HIDDEN_FILES.contains(&name.as_str())
            } else {
                !is_hidden(&name, is_dir) && !(unity_root && is_dir && unity::GENERATED_DIRS.contains(&name.as_str()))
            };
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

    fn unity_project(root: &Path) {
        for dir in ["Assets/Scripts", "ProjectSettings", "Packages", "Library/PackageCache", "Temp", "Logs", "obj/Debug", "UserSettings"] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        std::fs::write(root.join("ProjectSettings/ProjectVersion.txt"), "m_EditorVersion: 2022.3.10f1\nm_EditorVersionWithRevision: 2022.3.10f1 (ff3792e53c62)\n").unwrap();
        std::fs::write(root.join("Assets/Scripts/Player.cs"), "public class Player {}\n").unwrap();
    }

    #[test]
    fn a_unity_folder_without_a_solution_opens_as_a_folder_and_hides_generated_folders() {
        let dir = tempfile::tempdir().unwrap();
        let root = dunce::canonicalize(dir.path()).unwrap();
        unity_project(&root);
        // A lone generated project is not a standalone .NET project here.
        std::fs::write(root.join("Assembly-CSharp.csproj"), "<Project />\n").unwrap();
        let ws = Workspace::open(&root).unwrap();
        assert_eq!(ws.kind, WorkspaceKind::Folder);
        assert_eq!(ws.key_path(), root);
        let unity = ws.unity.as_ref().unwrap();
        assert_eq!((unity.root.as_path(), unity.editor.as_ref().unwrap().version.as_str()), (root.as_path(), "2022.3.10f1"));
        let names: Vec<String> = list_dir(&root).into_iter().map(|n| n.name).collect();
        assert_eq!(names, ["Assets", "Packages", "ProjectSettings", "UserSettings", "Assembly-CSharp.csproj"]);
        let all: Vec<String> = list_dir_with(&root, true).into_iter().map(|n| n.name).collect();
        assert!(["Library", "Temp", "Logs", "obj"].iter().all(|n| all.iter().any(|a| a == n)), "{all:?}");
        assert!(is_hidden_in(&root, "Library", true) && !is_hidden_in(&root, "Library", false));
        assert!(!is_hidden_in(&root.join("Assets"), "Library", true));

        // With the solution Unity generates, the solution is used as before.
        std::fs::write(root.join("Game.sln"), "Microsoft Visual Studio Solution File, Format Version 12.00\n").unwrap();
        let ws = Workspace::open(&root).unwrap();
        assert_eq!(ws.kind, WorkspaceKind::Solution(root.join("Game.sln")));
        assert!(ws.unity.is_some());
        assert!(Workspace::open(&root.join("Game.sln")).unwrap().unity.is_some());
    }

    #[test]
    fn unity_folder_names_stay_visible_outside_unity_roots() {
        let dir = tempfile::tempdir().unwrap();
        let root = dunce::canonicalize(dir.path()).unwrap();
        for d in ["Library", "Temp", "Logs", "Assets"] {
            std::fs::create_dir(root.join(d)).unwrap();
        }
        std::fs::write(root.join("App.csproj"), "<Project Sdk=\"Microsoft.NET.Sdk\" />\n").unwrap();
        let ws = Workspace::open(&root).unwrap();
        assert_eq!(ws.kind, WorkspaceKind::Project(root.join("App.csproj")));
        assert_eq!(ws.unity, None);
        let names: Vec<String> = list_dir(&root).into_iter().map(|n| n.name).collect();
        assert_eq!(names, ["Assets", "Library", "Logs", "Temp", "App.csproj"]);
        // Inside a Unity project, an Assets subfolder with those names is ordinary content.
        let game = root.join("Game");
        unity_project(&game);
        std::fs::create_dir(game.join("Assets/Library")).unwrap();
        assert!(list_dir(&game.join("Assets")).iter().any(|n| n.name == "Library"));
        // A standalone .NET project nested in a Unity repository keeps its project behavior.
        let tool = game.join("Tools");
        std::fs::create_dir(&tool).unwrap();
        std::fs::write(tool.join("Tool.csproj"), "<Project Sdk=\"Microsoft.NET.Sdk\" />\n").unwrap();
        let ws = Workspace::open(&tool).unwrap();
        assert_eq!(ws.kind, WorkspaceKind::Project(tool.join("Tool.csproj")));
        assert_eq!(ws.unity.as_ref().map(|u| u.root.clone()), Some(game));
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
