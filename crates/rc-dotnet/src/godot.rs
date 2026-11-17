//! Godot 4 project launches, using the same cancellable process and build events as .NET runs.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use rc_core::Runtime;
use rc_core::inbox::Outbox;
use tokio::sync::mpsc;

use crate::build::{BuildAction, BuildEvent, BuildRequest, build};
use crate::process::{CancelToken, OpHandle, StreamOpts, run_streaming};
use crate::run::{LaunchPlan, RunEvent, RunHandle, STOP_GRACE};
use crate::sdk::Dotnet;

/// A game, scene or editor launch. Set `build` for C# projects; Godot's C# SDK is
/// a library and must be built before the engine loads its scripts.
#[derive(Clone, Debug)]
pub struct GodotRunRequest {
    pub executable: PathBuf,
    pub project_root: PathBuf,
    pub scene: Option<PathBuf>,
    pub editor: bool,
    /// Require an engine build that includes C# support, including editor launches.
    pub require_dotnet: bool,
    /// SDK installation that Godot should use to locate hostfxr and invoke dotnet.
    pub dotnet_root: Option<PathBuf>,
    pub build: Option<(Dotnet, PathBuf)>,
}

impl GodotRunRequest {
    pub fn new(executable: impl Into<PathBuf>, project_root: impl Into<PathBuf>) -> Self {
        Self {
            executable: executable.into(),
            project_root: project_root.into(),
            scene: None,
            editor: false,
            require_dotnet: false,
            dotnet_root: None,
            build: None,
        }
    }

    /// Build the exact argv passed to Godot. This does not start a process.
    pub fn plan(&self) -> anyhow::Result<LaunchPlan> {
        anyhow::ensure!(
            self.project_root.join("project.godot").is_file(),
            "{} has no project.godot",
            self.project_root.display()
        );
        let mut args = vec![
            "--path".into(),
            self.project_root.to_string_lossy().into_owned(),
        ];
        if self.editor {
            args.push("--editor".into());
        }
        if let Some(scene) = &self.scene {
            args.extend(["--scene".into(), scene.to_string_lossy().into_owned()]);
        }
        let mut env = Vec::new();
        if let Some(root) = &self.dotnet_root {
            env.push(("DOTNET_ROOT".into(), root.to_string_lossy().into_owned()));
            let current = std::env::var_os("PATH").unwrap_or_default();
            let paths = std::iter::once(root.clone()).chain(std::env::split_paths(&current));
            if let Ok(path) = std::env::join_paths(paths) {
                env.push(("PATH".into(), path.to_string_lossy().into_owned()));
            }
        }
        Ok(LaunchPlan {
            program: self.executable.clone(),
            args,
            env,
            working_dir: self.project_root.clone(),
            urls: Vec::new(),
            profile: None,
            browser_url: None,
        })
    }
}

/// Where [`find_godot_executable`] looks, read from the process environment.
#[derive(Clone, Debug, Default)]
pub struct GodotSearch {
    /// `GODOT4` and `GODOT`, the variables Godot's C# documentation uses for the editor.
    pub env_paths: Vec<PathBuf>,
    pub path_var: Option<std::ffi::OsString>,
    pub home: Option<PathBuf>,
    /// Windows `%LOCALAPPDATA%` and `%ProgramFiles%`.
    pub local_app_data: Option<PathBuf>,
    pub program_files: Option<PathBuf>,
}

impl GodotSearch {
    pub fn from_env() -> Self {
        let env = |key: &str| std::env::var_os(key).filter(|value| !value.is_empty()).map(PathBuf::from);
        GodotSearch {
            env_paths: ["GODOT4", "GODOT"].into_iter().filter_map(env).collect(),
            path_var: std::env::var_os("PATH"),
            home: env("HOME").or_else(|| env("USERPROFILE")),
            local_app_data: env("LOCALAPPDATA"),
            program_files: env("ProgramFiles"),
        }
    }
}

/// Finds the Godot editor: the configured path (a binary, a macOS `.app` or a folder holding
/// one), `GODOT4` / `GODOT`, `PATH`, then the usual install folders, since an app started from
/// Finder or a desktop launcher has a minimal `PATH`. .NET builds (`mono` in the name) win.
pub fn find_godot_executable(configured: Option<&Path>) -> Option<PathBuf> {
    locate(configured, &GodotSearch::from_env())
}

/// [`find_godot_executable`] with an explicit environment (for tests).
pub fn locate(configured: Option<&Path>, search: &GodotSearch) -> Option<PathBuf> {
    let path_dirs: Vec<PathBuf> = search.path_var.as_ref().map(|value| std::env::split_paths(value).collect()).unwrap_or_default();
    if let Some(path) = configured {
        return resolve_any(path).or_else(|| {
            (path.components().count() == 1).then(|| path_dirs.iter().find_map(|dir| resolve_executable(&dir.join(path)))).flatten()
        });
    }
    if let Some(found) = search.env_paths.iter().find_map(|path| resolve_any(path)) {
        return Some(found);
    }
    if let Some(found) = find_in_paths(&path_dirs) {
        return Some(found);
    }
    best(install_dirs(search).iter().flat_map(|dir| scan(dir, 1)).collect())
}

/// Folders where Godot is commonly installed or unpacked (it ships as a portable binary).
fn install_dirs(search: &GodotSearch) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let home = search.home.as_deref();
    if cfg!(target_os = "macos") {
        // Not ~/Downloads: reading it triggers a macOS privacy prompt.
        dirs.push(PathBuf::from("/Applications"));
        dirs.extend(home.map(|home| home.join("Applications")));
        dirs.extend(["/opt/homebrew/bin", "/usr/local/bin"].map(PathBuf::from));
        dirs.extend(home.map(|home| home.join("Library/Application Support/Steam/steamapps/common/Godot Engine")));
    } else if cfg!(windows) {
        if let Some(local) = &search.local_app_data {
            dirs.push(local.join("Programs"));
            dirs.push(local.join("Microsoft").join("WinGet").join("Links"));
            dirs.push(local.join("Microsoft").join("WinGet").join("Packages"));
        }
        dirs.extend(search.program_files.clone());
        if let Some(home) = home {
            dirs.push(home.join("scoop").join("shims"));
            dirs.push(home.join("scoop").join("apps"));
            dirs.push(home.join("Godot"));
            dirs.push(home.join("Downloads"));
        }
    } else {
        dirs.extend(["/usr/local/bin", "/usr/bin", "/opt", "/var/lib/flatpak/exports/bin"].map(PathBuf::from));
        if let Some(home) = home {
            dirs.extend([".local/bin", "bin", "Applications", "Godot", "Downloads", ".local/share/flatpak/exports/bin", ".local/share/Steam/steamapps/common"].map(|dir| home.join(dir)));
        }
    }
    dirs
}

/// Godot entries of `dir`: an executable or `.app` named like Godot, or one inside a Godot
/// folder up to `depth` levels down (the unpacked `Godot_v4.4-stable_mono_win64` archive).
fn scan(dir: &Path, depth: usize) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut paths: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.file_name().is_some_and(|name| {
            let name = name.to_string_lossy().to_ascii_lowercase();
            name.contains("godot") && ![".pck", ".zip", ".so", ".dll", ".dylib", ".txt"].iter().any(|ext| name.ends_with(ext))
        }))
        .collect();
    paths.sort();
    let mut out = Vec::new();
    for path in paths {
        if let Some(found) = resolve_executable(&path) {
            out.push(found);
        } else if depth > 0 && path.is_dir() {
            out.extend(scan(&path, depth - 1));
        }
    }
    out
}

/// The .NET build first, then the windowed binary over Windows' `_console` wrapper.
fn best(mut found: Vec<PathBuf>) -> Option<PathBuf> {
    found.sort_by_key(|path| (!is_dotnet_name(path), is_console_name(path)));
    found.into_iter().next()
}

fn is_dotnet_name(path: &Path) -> bool {
    path.ancestors().take(4).any(|part| part.file_name().is_some_and(|name| {
        let name = name.to_string_lossy().to_ascii_lowercase();
        name.contains("mono") || name.contains("godotsharp")
    }))
}

fn is_console_name(path: &Path) -> bool {
    path.file_name().is_some_and(|name| name.to_string_lossy().to_ascii_lowercase().contains("_console"))
}

/// A binary, a `.app`, or a folder holding one.
fn resolve_any(path: &Path) -> Option<PathBuf> {
    resolve_executable(path).or_else(|| best(scan(path, 0)))
}

fn resolve_executable(path: &Path) -> Option<PathBuf> {
    if path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("app")) {
        if !path.is_dir() {
            return None;
        }
        let macos = path.join("Contents/MacOS");
        let binary = macos.join("Godot");
        if is_executable_file(&binary) {
            return Some(binary);
        }
        // A renamed bundle still holds a single executable.
        let mut entries: Vec<PathBuf> = std::fs::read_dir(&macos).ok()?.flatten().map(|entry| entry.path()).collect();
        entries.sort();
        return entries.into_iter().find(|entry| is_executable_file(entry));
    }
    if cfg!(windows) && !path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("exe")) {
        return None;
    }
    is_executable_file(path).then(|| path.to_path_buf())
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else { return false };
    if !metadata.is_file() { return false; }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn find_in_paths(paths: &[PathBuf]) -> Option<PathBuf> {
    #[cfg(windows)]
    const NAMES: &[&str] = &[
        "godot-mono.exe",
        "godot4-mono.exe",
        "godot.exe",
        "godot4.exe",
    ];
    #[cfg(not(windows))]
    const NAMES: &[&str] = &["godot-mono", "godot4-mono", "godot", "godot4"];
    for name in NAMES {
        for dir in paths {
            if let Some(path) = resolve_executable(&dir.join(name)) {
                return Some(path);
            }
        }
    }
    None
}

/// Probe the editor's version on a background task. Official .NET builds include
/// `.mono.` in the version string; the regular editor cannot load C# scripts.
pub async fn is_dotnet_enabled(executable: &Path) -> anyhow::Result<bool> {
    let mut command = tokio::process::Command::new(executable);
    command.arg("--version").kill_on_drop(true);
    let output = tokio::time::timeout(Duration::from_secs(5), command.output()).await??;
    anyhow::ensure!(
        output.status.success(),
        "cannot read Godot version from {}",
        executable.display()
    );
    let version = String::from_utf8_lossy(&output.stdout);
    Ok(version.to_ascii_lowercase().contains("mono"))
}

/// Start a Godot run on the shared runtime. Stopping the handle cancels both
/// the optional .NET build and the engine process tree.
pub fn start_godot(req: GodotRunRequest, outbox: Outbox<RunEvent>) -> RunHandle {
    let cancel = CancelToken::new();
    let token = cancel.clone();
    let (input, input_rx) = mpsc::unbounded_channel();
    let pid = Arc::new(AtomicU32::new(0));
    let pid2 = pid.clone();
    let task = Runtime::spawn(async move {
        let out = outbox.clone();
        let result = run(&req, &token, input_rx, &pid2, &mut |event| {
            out.send(event);
        }).await;
        outbox.send(match result {
            Ok(event) => event,
            Err(error) => RunEvent::Failed(format!("{error:#}")),
        });
    });
    RunHandle::from_parts(OpHandle::new(cancel, task), input, pid)
}

async fn run(
    req: &GodotRunRequest,
    cancel: &CancelToken,
    input: mpsc::UnboundedReceiver<Vec<u8>>,
    pid: &AtomicU32,
    on_event: &mut (dyn FnMut(RunEvent) + Send),
) -> anyhow::Result<RunEvent> {
    let plan = req.plan()?;
    if req.require_dotnet || req.build.is_some() {
        let enabled = tokio::select! {
            result = is_dotnet_enabled(&plan.program) => result?,
            _ = cancel.cancelled() => return Ok(RunEvent::Exited { code: None, cancelled: true, duration: Duration::ZERO }),
        };
        anyhow::ensure!(
            enabled,
            "{} does not support C#; select a Godot .NET editor",
            plan.program.display()
        );
    }
    if let Some((dotnet, project)) = &req.build {
        let build_req = BuildRequest::new(project, BuildAction::Build);
        let result = build(dotnet, &build_req, cancel, &mut |event| {
            on_event(RunEvent::Build(event))
        })
        .await?;
        on_event(RunEvent::Build(BuildEvent::Finished(result.clone())));
        if result.cancelled {
            return Ok(RunEvent::Exited {
                code: None,
                cancelled: true,
                duration: Duration::ZERO,
            });
        }
        if !result.success {
            return Ok(RunEvent::Failed(format!(
                "build failed with {} error(s)",
                result.errors
            )));
        }
    }
    let mut command = tokio::process::Command::new(&plan.program);
    command.args(&plan.args).current_dir(&plan.working_dir).envs(plan.env.iter().map(|(key, value)| (key, value)));
    let started = Instant::now();
    let events = std::sync::Mutex::new(on_event);
    let outcome = run_streaming(
        command,
        cancel,
        StreamOpts {
            partial_lines: true,
            grace: STOP_GRACE,
            stdin: Some(input),
        },
        &mut |process_id| {
            pid.store(process_id, Ordering::SeqCst);
            (events.lock().unwrap())(RunEvent::Started {
                pid: process_id,
                plan: plan.clone(),
            });
        },
        &mut |line| (events.lock().unwrap())(RunEvent::Output(line)),
    )
    .await
    .map_err(|error| anyhow::anyhow!("cannot start {}: {error}", plan.program.display()))?;
    Ok(RunEvent::Exited {
        code: outcome.code,
        cancelled: outcome.cancelled,
        duration: started.elapsed(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn fake_engine(dir: &Path, version: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("godot-mono");
        std::fs::write(&path, format!("#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo '{version}'; else echo \"DOTNET_ROOT=$DOTNET_ROOT\"; echo \"$*\"; fi\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[test]
    fn plans_project_scene_and_editor_modes() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("project.godot"), "config_version=5\n").unwrap();
        let mut request = GodotRunRequest::new("/opt/godot-mono", dir.path());
        let root = dir.path().to_string_lossy().into_owned();
        assert_eq!(
            request.plan().unwrap().args,
            vec!["--path".to_owned(), root]
        );
        request.scene = Some("res://level.tscn".into());
        assert_eq!(
            request.plan().unwrap().args[2..],
            ["--scene", "res://level.tscn"]
        );
        request.editor = true;
        assert_eq!(
            request.plan().unwrap().args[2..],
            ["--editor", "--scene", "res://level.tscn"]
        );
        request.dotnet_root = Some(PathBuf::from("/opt/dotnet"));
        let plan = request.plan().unwrap();
        assert!(plan.env.iter().any(|(key, value)| key == "DOTNET_ROOT" && value == "/opt/dotnet"));
        if cfg!(unix) {
            assert!(plan.env.iter().any(|(key, value)| key == "PATH" && value.starts_with("/opt/dotnet")));
        }
    }

    #[cfg(unix)]
    fn executable(path: &Path) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "").unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path.to_path_buf()
    }

    #[cfg(unix)]
    #[test]
    fn install_folders_prefer_the_dotnet_app_bundle_or_unpacked_archive() {
        let dir = tempfile::tempdir().unwrap();
        executable(&dir.path().join("Godot.app/Contents/MacOS/Godot"));
        let mono_app = executable(&dir.path().join("Godot_mono.app/Contents/MacOS/Godot"));
        assert_eq!(best(scan(dir.path(), 1)), Some(mono_app.clone()));
        // A configured bundle or folder resolves to the binary inside.
        assert_eq!(resolve_any(&dir.path().join("Godot_mono.app")), Some(mono_app));
        let archive = dir.path().join("Godot_v4.4-stable_mono_linux_x86_64");
        let binary = executable(&archive.join("Godot_v4.4-stable_mono_linux.x86_64"));
        std::fs::write(archive.join("GodotSharp.txt"), "").unwrap();
        assert_eq!(resolve_any(&archive), Some(binary));
        // A renamed bundle keeps its single executable.
        let renamed = executable(&dir.path().join("Engine.app/Contents/MacOS/Godot 4.4"));
        assert_eq!(resolve_executable(&dir.path().join("Engine.app")), Some(renamed));
    }

    #[cfg(unix)]
    #[test]
    fn godot4_variable_wins_over_path() {
        let dir = tempfile::tempdir().unwrap();
        let on_path = executable(&dir.path().join("bin/godot"));
        let from_env = executable(&dir.path().join("engines/Godot_mono.app/Contents/MacOS/Godot"));
        let mut search = GodotSearch { path_var: Some(dir.path().join("bin").into_os_string()), ..GodotSearch::default() };
        assert_eq!(locate(None, &search), Some(on_path.clone()));
        search.env_paths = vec![dir.path().join("engines/Godot_mono.app")];
        assert_eq!(locate(None, &search), Some(from_env));
        // A bare configured name is looked up on PATH.
        assert_eq!(locate(Some(Path::new("godot")), &search), Some(on_path));
    }

    #[test]
    fn finds_dotnet_named_binary_before_regular_editor() {
        let dir = tempfile::tempdir().unwrap();
        let regular = dir
            .path()
            .join(if cfg!(windows) { "godot.exe" } else { "godot" });
        let mono = dir.path().join(if cfg!(windows) {
            "godot-mono.exe"
        } else {
            "godot-mono"
        });
        std::fs::write(&regular, "").unwrap();
        std::fs::write(&mono, "").unwrap();
        #[cfg(unix)]
        for path in [&regular, &mono] {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        assert_eq!(find_in_paths(&[dir.path().to_path_buf()]), Some(mono));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn probes_dotnet_editor_version() {
        let dir = tempfile::tempdir().unwrap();
        let engine = fake_engine(dir.path(), "4.5.stable.mono.official");
        assert!(is_dotnet_enabled(&engine).await.unwrap());
        let engine = fake_engine(dir.path(), "4.5.stable.official");
        assert!(!is_dotnet_enabled(&engine).await.unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn streams_a_fake_engine_launch() {
        use rc_core::inbox::{Inbox, Waker};

        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("project.godot"), "config_version=5\n").unwrap();
        let engine = fake_engine(dir.path(), "4.5.stable.mono.official");
        let inbox = Inbox::new(Waker::noop());
        let mut request = GodotRunRequest::new(engine, dir.path());
        request.dotnet_root = Some(dir.path().to_path_buf());
        let handle = start_godot(request, inbox.sender());
        let mut started = false;
        let mut output = false;
        let mut root_forwarded = false;
        let mut exited = false;
        for _ in 0..6 {
            match inbox.recv_timeout(Duration::from_secs(5)).expect("Godot run event") {
                RunEvent::Started { .. } => started = true,
                RunEvent::Output(line) => {
                    output |= line.text.contains("--path");
                    root_forwarded |= line.text.contains(&format!("DOTNET_ROOT={}", dir.path().display()));
                }
                RunEvent::Exited { code, cancelled, .. } => {
                    assert_eq!(code, Some(0));
                    assert!(!cancelled);
                    exited = true;
                    break;
                }
                RunEvent::Failed(error) => panic!("{error}"),
                RunEvent::Build(_) => panic!("unexpected build"),
            }
        }
        assert!(started && output && root_forwarded && exited);
        assert!(handle.is_finished() || handle.pid().is_some());
    }
}
