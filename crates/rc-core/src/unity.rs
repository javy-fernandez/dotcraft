//! Unity projects: detection of a project root (`Assets` + `ProjectSettings`), the editor
//! version from `ProjectSettings/ProjectVersion.txt`, the folders Unity regenerates (hidden
//! from the explorer, search and the file watcher only inside a Unity root) and the editor
//! installation that matches the project.

use std::path::{Path, PathBuf};

/// Folders Unity creates and rewrites on its own, directly below a project root. `obj` is
/// already hidden for every workspace (see [`crate::workspace::HIDDEN_DIRS`]).
pub const GENERATED_DIRS: &[&str] = &["Library", "Temp", "Logs", "MemoryCaptures"];

/// Whether `dir` is the root of a Unity project.
pub fn is_unity_root(dir: &Path) -> bool {
    dir.join("Assets").is_dir() && dir.join("ProjectSettings").is_dir()
}

/// Whether a folder named `name` inside `parent` is one Unity generates. Only touches the disk
/// when the name matches, so it is cheap for every other folder.
pub fn is_generated_dir(parent: &Path, name: &str) -> bool {
    GENERATED_DIRS.contains(&name) && is_unity_root(parent)
}

/// Whether `path` (at or below `root`) lies inside a generated folder of a Unity project at or
/// below `root`. Paths outside `root` are never generated.
pub fn in_generated_dir(root: &Path, path: &Path) -> bool {
    let Ok(rel) = path.strip_prefix(root) else {
        return false;
    };
    let mut parent = root.to_path_buf();
    for component in rel.components() {
        if is_generated_dir(&parent, &component.as_os_str().to_string_lossy()) {
            return true;
        }
        parent.push(component);
    }
    false
}

/// The editor a project was last saved with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorVersion {
    /// For example `2022.3.10f1`; also the folder name Unity Hub installs it under.
    pub version: String,
    /// The changeset, for example `ff3792e53c62`, when the file records it.
    pub revision: Option<String>,
}

/// Reads `m_EditorVersion` (and the revision of `m_EditorVersionWithRevision`) from the text of
/// `ProjectVersion.txt`. Versions that are not plain `digits.digits…` tokens are rejected: the
/// version becomes a path component when looking for the editor.
pub fn parse_project_version(text: &str) -> Option<EditorVersion> {
    let field = |key: &str| {
        text.lines().find_map(|line| {
            let (k, v) = line.split_once(':')?;
            (k.trim() == key).then(|| v.trim().to_string())
        })
    };
    let with_revision = field("m_EditorVersionWithRevision");
    let version = field("m_EditorVersion")
        .filter(|v| !v.is_empty())
        .or_else(|| {
            with_revision
                .as_deref()
                .and_then(|v| v.split_whitespace().next())
                .map(str::to_string)
        })?;
    if !valid_version(&version) {
        return None;
    }
    let revision = with_revision.and_then(|v| {
        let (head, rest) = v.split_once('(')?;
        let rev = rest.strip_suffix(')')?.trim();
        (head.trim() == version
            && !rev.is_empty()
            && rev.chars().all(|c| c.is_ascii_alphanumeric()))
        .then(|| rev.to_string())
    });
    Some(EditorVersion { version, revision })
}

fn valid_version(v: &str) -> bool {
    v.starts_with(|c: char| c.is_ascii_digit())
        && v.contains('.')
        && !v.contains("..")
        && v.chars().all(|c| c.is_ascii_alphanumeric() || c == '.')
}

/// A Unity project found at `root`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnityProject {
    pub root: PathBuf,
    /// `None` when `ProjectVersion.txt` is missing or unreadable.
    pub editor: Option<EditorVersion>,
}

impl UnityProject {
    /// The project whose root is `dir`, if it is one.
    pub fn detect(dir: &Path) -> Option<UnityProject> {
        if !is_unity_root(dir) {
            return None;
        }
        let editor =
            std::fs::read_to_string(dir.join("ProjectSettings").join("ProjectVersion.txt"))
                .ok()
                .and_then(|text| parse_project_version(&text));
        Some(UnityProject {
            root: dir.to_path_buf(),
            editor,
        })
    }

    /// The project at `dir` or at the nearest folder above it (a folder inside `Assets` was
    /// opened).
    pub fn find(dir: &Path) -> Option<UnityProject> {
        dir.ancestors().find_map(UnityProject::detect)
    }
}

/// The executable inside an editor installation folder (`…/Hub/Editor/<version>`).
pub fn editor_executable(install_dir: &Path) -> PathBuf {
    if cfg!(target_os = "macos") {
        install_dir.join("Unity.app/Contents/MacOS/Unity")
    } else if cfg!(windows) {
        install_dir.join("Editor").join("Unity.exe")
    } else {
        install_dir.join("Editor").join("Unity")
    }
}

/// Folders where Unity Hub installs editors, one subfolder per version: its default location
/// and the custom one chosen in the Hub (`secondaryInstallPath.json`).
pub fn default_editor_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(hub) = dirs::config_dir().map(|c| c.join("UnityHub"))
        && let Some(custom) = secondary_install_path(&hub)
    {
        dirs.push(custom);
    }
    if cfg!(target_os = "macos") {
        dirs.push(PathBuf::from("/Applications/Unity/Hub/Editor"));
    } else if cfg!(windows) {
        for var in ["ProgramFiles", "ProgramW6432"] {
            if let Some(pf) = std::env::var_os(var).filter(|v| !v.is_empty()) {
                let dir = PathBuf::from(pf).join("Unity").join("Hub").join("Editor");
                if !dirs.contains(&dir) {
                    dirs.push(dir);
                }
            }
        }
    } else if let Some(home) = dirs::home_dir() {
        dirs.push(home.join("Unity").join("Hub").join("Editor"));
    }
    dirs
}

/// The custom editors folder recorded by Unity Hub in `<hub config>/secondaryInstallPath.json`
/// (a JSON string, empty when unset).
pub fn secondary_install_path(hub_config: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(hub_config.join("secondaryInstallPath.json")).ok()?;
    let path: String = serde_json::from_str(text.trim()).ok()?;
    let path = path.trim();
    (!path.is_empty()).then(|| PathBuf::from(path))
}

/// Why no editor could be chosen for a project.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditorLookupError {
    /// `ProjectSettings/ProjectVersion.txt` is missing or has no valid version.
    UnknownVersion,
    /// The configured editor path does not exist.
    ConfiguredMissing(PathBuf),
    /// No installation of `version` in any of `searched`.
    NotInstalled {
        version: String,
        searched: Vec<PathBuf>,
    },
}

/// The editor to open `project` with. `configured` (Settings › Tools) wins: an executable (or a
/// macOS `Unity.app`) is used as is, a folder is searched for the project's version before
/// `editor_dirs`. Otherwise only the exact version the project was saved with is accepted, so
/// opening never upgrades the project to another editor by accident.
pub fn find_editor(
    project: &UnityProject,
    configured: Option<&Path>,
    editor_dirs: &[PathBuf],
) -> Result<PathBuf, EditorLookupError> {
    let mut dirs = Vec::new();
    if let Some(path) = configured {
        if path.is_file() {
            return Ok(path.to_path_buf());
        }
        if path.is_dir()
            && path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("app"))
        {
            let exe = path.join("Contents/MacOS/Unity");
            return if exe.is_file() {
                Ok(exe)
            } else {
                Err(EditorLookupError::ConfiguredMissing(path.to_path_buf()))
            };
        }
        if !path.is_dir() {
            return Err(EditorLookupError::ConfiguredMissing(path.to_path_buf()));
        }
        dirs.push(path.to_path_buf());
    }
    let Some(editor) = &project.editor else {
        return Err(EditorLookupError::UnknownVersion);
    };
    for dir in editor_dirs {
        if !dirs.contains(dir) {
            dirs.push(dir.clone());
        }
    }
    dirs.iter()
        .map(|dir| editor_executable(&dir.join(&editor.version)))
        .find(|exe| exe.is_file())
        .ok_or_else(|| EditorLookupError::NotInstalled {
            version: editor.version.clone(),
            searched: dirs,
        })
}

/// The command that opens `project` in `editor`: the project path is one argument, never
/// passed through a shell.
pub fn editor_command(editor: &Path, project: &Path) -> std::process::Command {
    let mut command = std::process::Command::new(editor);
    command
        .arg("-projectPath")
        .arg(project)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    command
}

/// Starts the editor on `project` and reaps it on a background thread when it exits.
pub fn launch_editor(editor: &Path, project: &Path) -> std::io::Result<()> {
    let mut child = editor_command(editor, project).spawn()?;
    std::thread::Builder::new()
        .name("rc-unity-editor".into())
        .spawn(move || {
            let _ = child.wait();
        })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unity_project(dir: &Path, version: Option<&str>) {
        std::fs::create_dir_all(dir.join("Assets/Scripts")).unwrap();
        std::fs::create_dir_all(dir.join("ProjectSettings")).unwrap();
        if let Some(v) = version {
            std::fs::write(
                dir.join("ProjectSettings/ProjectVersion.txt"),
                format!("m_EditorVersion: {v}\nm_EditorVersionWithRevision: {v} (ff3792e53c62)\n"),
            )
            .unwrap();
        }
    }

    fn fake_editor(editors: &Path, version: &str) -> PathBuf {
        let exe = editor_executable(&editors.join(version));
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, "").unwrap();
        exe
    }

    #[test]
    fn parses_editor_version_and_revision() {
        let text = "m_EditorVersion: 2022.3.10f1\r\nm_EditorVersionWithRevision: 2022.3.10f1 (ff3792e53c62)\r\n";
        assert_eq!(
            parse_project_version(text),
            Some(EditorVersion {
                version: "2022.3.10f1".into(),
                revision: Some("ff3792e53c62".into())
            })
        );
        // Unity 5 / 2017 files only have the first line.
        assert_eq!(
            parse_project_version("m_EditorVersion: 2017.4.40f1\n").unwrap(),
            EditorVersion {
                version: "2017.4.40f1".into(),
                revision: None
            }
        );
        // A revision that belongs to another version is ignored.
        assert_eq!(
            parse_project_version(
                "m_EditorVersion: 6000.0.23f1\nm_EditorVersionWithRevision: 6000.0.22f1 (abc)\n"
            )
            .unwrap()
            .revision,
            None
        );
        assert_eq!(
            parse_project_version("m_EditorVersionWithRevision: 6000.0.23f1 (1c4764c07fb4)\n")
                .unwrap()
                .version,
            "6000.0.23f1"
        );
    }

    #[test]
    fn rejects_versions_that_are_not_safe_folder_names() {
        for bad in [
            "",
            "../../bin",
            "2022.3/../x",
            "2022 3",
            "latest",
            "2022..3",
            "C:\\Unity",
        ] {
            assert_eq!(
                parse_project_version(&format!("m_EditorVersion: {bad}\n")),
                None,
                "{bad:?}"
            );
        }
        assert_eq!(parse_project_version("not a project version file"), None);
    }

    #[test]
    fn detects_roots_only_with_assets_and_project_settings() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("Game");
        std::fs::create_dir_all(root.join("Assets")).unwrap();
        assert!(!is_unity_root(&root));
        assert_eq!(UnityProject::detect(&root), None);
        unity_project(&root, Some("2022.3.10f1"));
        let project = UnityProject::detect(&root).unwrap();
        assert_eq!(project.editor.unwrap().version, "2022.3.10f1");
        assert_eq!(
            UnityProject::find(&root.join("Assets/Scripts"))
                .unwrap()
                .root,
            root
        );
        std::fs::remove_file(root.join("ProjectSettings/ProjectVersion.txt")).unwrap();
        assert_eq!(UnityProject::detect(&root).unwrap().editor, None);
    }

    #[test]
    fn generated_folders_count_only_directly_below_a_unity_root() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let game = root.join("Game");
        unity_project(&game, None);
        assert!(in_generated_dir(
            root,
            &game.join("Library/ScriptAssemblies/A.dll")
        ));
        assert!(in_generated_dir(root, &game.join("Temp")));
        assert!(in_generated_dir(&game, &game.join("Logs/Editor.log")));
        // The same names elsewhere are ordinary folders.
        assert!(!in_generated_dir(
            root,
            &game.join("Assets/Library/Shelf.cs")
        ));
        assert!(!in_generated_dir(root, &root.join("Library/Book.cs")));
        assert!(!in_generated_dir(
            root,
            &game.join("Assets/Scripts/Player.cs")
        ));
        assert!(!in_generated_dir(&game, Path::new("/elsewhere/Library")));
    }

    #[test]
    fn finds_the_matching_installed_editor_only() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("Game");
        unity_project(&root, Some("2022.3.10f1"));
        let project = UnityProject::detect(&root).unwrap();
        let hub = dir.path().join("Hub/Editor");
        fake_editor(&hub, "2021.3.1f1");
        let err = find_editor(&project, None, std::slice::from_ref(&hub)).unwrap_err();
        assert_eq!(
            err,
            EditorLookupError::NotInstalled {
                version: "2022.3.10f1".into(),
                searched: vec![hub.clone()]
            }
        );
        let exe = fake_editor(&hub, "2022.3.10f1");
        assert_eq!(
            find_editor(&project, None, std::slice::from_ref(&hub)),
            Ok(exe)
        );
    }

    #[test]
    fn configured_editor_path_overrides_the_lookup() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("Game");
        unity_project(&root, None);
        let project = UnityProject::detect(&root).unwrap();
        // Without a version, only an explicit executable can be used.
        assert_eq!(
            find_editor(&project, None, &[]),
            Err(EditorLookupError::UnknownVersion)
        );
        let exe = dir.path().join("CustomUnity");
        std::fs::write(&exe, "").unwrap();
        assert_eq!(find_editor(&project, Some(&exe), &[]), Ok(exe));
        let missing = dir.path().join("missing");
        assert_eq!(
            find_editor(&project, Some(&missing), &[]),
            Err(EditorLookupError::ConfiguredMissing(missing))
        );
        // A configured folder is searched first for the project's version.
        unity_project(&root, Some("6000.0.23f1"));
        let project = UnityProject::detect(&root).unwrap();
        let custom = dir.path().join("Editors");
        let hub = dir.path().join("Hub");
        fake_editor(&hub, "6000.0.23f1");
        let preferred = fake_editor(&custom, "6000.0.23f1");
        assert_eq!(find_editor(&project, Some(&custom), &[hub]), Ok(preferred));
    }

    #[test]
    fn reads_the_hub_custom_install_folder() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(secondary_install_path(dir.path()), None);
        std::fs::write(dir.path().join("secondaryInstallPath.json"), "\"\"").unwrap();
        assert_eq!(secondary_install_path(dir.path()), None);
        std::fs::write(
            dir.path().join("secondaryInstallPath.json"),
            r#""D:\\Unity Editors""#,
        )
        .unwrap();
        assert_eq!(
            secondary_install_path(dir.path()),
            Some(PathBuf::from(r"D:\Unity Editors"))
        );
    }

    #[test]
    fn editor_command_passes_the_project_as_one_argument() {
        let project = Path::new("/work/My Game; $(touch pwned) `id` & 'x'");
        let command = editor_command(Path::new("/opt/Unity/Editor/Unity"), project);
        assert_eq!(command.get_program(), "/opt/Unity/Editor/Unity");
        let args: Vec<_> = command.get_args().collect();
        assert_eq!(
            args,
            [std::ffi::OsStr::new("-projectPath"), project.as_os_str()]
        );
    }

    #[cfg(unix)]
    #[test]
    fn launching_runs_the_editor_without_a_shell() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("My Game $(touch pwned)");
        unity_project(&root, Some("2022.3.10f1"));
        let hub = dir.path().join("Hub");
        let exe = fake_editor(&hub, "2022.3.10f1");
        let log = dir.path().join("args.txt");
        std::fs::write(
            &exe,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}.tmp'\nmv '{0}.tmp' '{0}'\n",
                log.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        let project = UnityProject::detect(&root).unwrap();
        let editor = find_editor(&project, None, &[hub]).unwrap();
        launch_editor(&editor, &project.root).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !log.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(
            std::fs::read_to_string(&log).unwrap(),
            format!("-projectPath\n{}\n", root.display())
        );
        assert!(
            !dir.path().join("pwned").exists()
                && !root.join("pwned").exists()
                && !Path::new("pwned").exists()
        );
    }
}
