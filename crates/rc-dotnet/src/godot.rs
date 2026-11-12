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

/// Find an engine in the configured location or on PATH, preferring names used
/// by .NET-enabled Godot packages. A configured macOS `.app` resolves to its binary.
pub fn find_godot_executable(configured: Option<&Path>) -> Option<PathBuf> {
    if let Some(path) = configured {
        return resolve_executable(path).or_else(|| {
            (path.components().count() == 1)
                .then(|| std::env::var_os("PATH"))
                .flatten()
                .and_then(|value| std::env::split_paths(&value).find_map(|dir| resolve_executable(&dir.join(path))))
        });
    }
    let paths = std::env::var_os("PATH")
        .map(|value| std::env::split_paths(&value).collect::<Vec<_>>())
        .unwrap_or_default();
    find_in_paths(&paths)
}

fn resolve_executable(path: &Path) -> Option<PathBuf> {
    if path.is_dir()
        && path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("app"))
    {
        let binary = path.join("Contents/MacOS/Godot");
        return is_executable_file(&binary).then_some(binary);
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
