//! Open in Unity Editor: starts the editor version the open Unity project was saved with (or
//! the editor chosen in Settings › Tools) on the project. Play mode, builds and debugging stay
//! in Unity.

use crate::app::DotCraftApp;
use rc_core::unity::{self, EditorLookupError, UnityProject};
use rc_kit::i18n::{tr, trf};

/// The notice for an editor that could not be chosen.
pub fn lookup_error_text(err: &EditorLookupError) -> String {
    match err {
        EditorLookupError::UnknownVersion => {
            tr("ProjectSettings/ProjectVersion.txt has no Unity Editor version. Choose a Unity Editor in Settings › Tools.").to_string()
        }
        EditorLookupError::ConfiguredMissing(path) => {
            trf("The Unity Editor set in Settings › Tools was not found: {path}", &[("path", &path.display())])
        }
        EditorLookupError::NotInstalled { version, .. } => trf(
            "Unity {version} is not installed. Install it with Unity Hub or choose a Unity Editor in Settings › Tools.",
            &[("version", version)],
        ),
    }
}

impl DotCraftApp {
    /// The Unity project of the open workspace, if it is one.
    pub fn unity_project(&self) -> Option<&UnityProject> {
        self.workspace.as_ref()?.ws.unity.as_ref()
    }

    /// Tools › Open in Unity Editor.
    pub fn open_in_unity_editor(&mut self) {
        let Some(project) = self.unity_project().cloned() else {
            self.notify(tr("The open project is not a Unity project"), true);
            return;
        };
        let configured = self.settings.unity_editor_path.clone();
        match unity::find_editor(
            &project,
            configured.as_deref(),
            &unity::default_editor_dirs(),
        ) {
            Ok(editor) => match unity::launch_editor(&editor, &project.root) {
                Ok(()) => {
                    let name = project.root.file_name().map_or_else(
                        || project.root.display().to_string(),
                        |n| n.to_string_lossy().into_owned(),
                    );
                    self.notify(
                        trf(
                            "Opening {project} in the Unity Editor",
                            &[("project", &name)],
                        ),
                        false,
                    );
                }
                Err(e) => self.notify(
                    trf(
                        "Could not start the Unity Editor: {error}",
                        &[("error", &e)],
                    ),
                    true,
                ),
            },
            Err(e) => self.notify(lookup_error_text(&e), true),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn unity_project(root: &Path, version: &str) {
        std::fs::create_dir_all(root.join("Assets")).unwrap();
        std::fs::create_dir_all(root.join("ProjectSettings")).unwrap();
        std::fs::write(
            root.join("ProjectSettings/ProjectVersion.txt"),
            format!("m_EditorVersion: {version}\n"),
        )
        .unwrap();
    }

    fn app_with(home: &Path, root: &Path) -> DotCraftApp {
        let mut app = crate::screens::test_app(home, true);
        app.workspace = Some(crate::app::WorkspaceState {
            ws: rc_core::Workspace::open(root).unwrap(),
            solution: None,
        });
        app
    }

    #[test]
    fn reports_why_the_editor_cannot_be_opened() {
        let _home = crate::screens::HOME_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let home = tempfile::tempdir().unwrap();
        let plain = home.path().join("Plain");
        std::fs::create_dir(&plain).unwrap();
        let mut app = app_with(home.path(), &plain);
        assert!(app.unity_project().is_none());
        app.open_in_unity_editor();
        assert_eq!(
            app.notices.last().map(|n| (n.text.as_str(), n.error)),
            Some((tr("The open project is not a Unity project"), true))
        );

        // A version nobody has installed; a configured folder without it is searched too.
        let game = home.path().join("Game");
        unity_project(&game, "1999.1.0f1");
        let mut app = app_with(home.path(), &game);
        app.settings.unity_editor_path = Some(home.path().to_path_buf());
        app.open_in_unity_editor();
        let notice = app.notices.last().unwrap();
        assert!(
            notice.error && notice.text.contains("1999.1.0f1"),
            "{}",
            notice.text
        );

        let missing = home.path().join("NoSuchUnity");
        app.settings.unity_editor_path = Some(missing.clone());
        app.open_in_unity_editor();
        assert_eq!(
            app.notices.last().unwrap().text,
            lookup_error_text(&EditorLookupError::ConfiguredMissing(missing))
        );
    }

    #[test]
    fn opening_a_unity_folder_without_projects_explains_how_to_generate_them() {
        let _home = crate::screens::HOME_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let home = tempfile::tempdir().unwrap();
        let game = home.path().join("Game");
        unity_project(&game, "2022.3.10f1");
        let guidance = tr("Unity has not generated the .sln and .csproj files of this project yet. In Unity, choose Visual Studio or Visual Studio Code in Preferences › External Tools, click Regenerate project files and open the folder again.");
        let mut app = crate::screens::test_app(home.path(), true);
        app.settings.roslyn_path = Some("/nonexistent".into());
        app.open_workspace(&game);
        assert!(app.notices.iter().any(|n| n.text == guidance && !n.error));
        app.close_workspace();

        std::fs::write(game.join("Assembly-CSharp.csproj"), "<Project />\n").unwrap();
        let mut app = crate::screens::test_app(home.path(), true);
        app.settings.roslyn_path = Some("/nonexistent".into());
        app.open_workspace(&game);
        assert!(app.unity_project().is_some());
        assert!(!app.notices.iter().any(|n| n.text == guidance));
        app.close_workspace();
    }

    #[cfg(unix)]
    #[test]
    fn launches_the_editor_from_the_configured_folder_with_the_project_path() {
        use std::os::unix::fs::PermissionsExt;
        let _home = crate::screens::HOME_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let home = tempfile::tempdir().unwrap();
        let game = home.path().join("My Game");
        unity_project(&game, "2022.3.10f1");
        let editors = home.path().join("Editors");
        let exe = unity::editor_executable(&editors.join("2022.3.10f1"));
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        let log = home.path().join("args.txt");
        std::fs::write(
            &exe,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}.tmp'\nmv '{0}.tmp' '{0}'\n",
                log.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut app = app_with(home.path(), &game);
        app.settings.unity_editor_path = Some(editors);
        app.open_in_unity_editor();
        let notice = app.notices.last().unwrap();
        assert!(!notice.error, "{}", notice.text);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !log.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let root = dunce::canonicalize(&game).unwrap();
        assert_eq!(
            std::fs::read_to_string(&log).unwrap(),
            format!("-projectPath\n{}\n", root.display())
        );
    }
}
