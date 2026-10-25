//! What to debug: a .NET project (built first, launched like `rc_dotnet::run` would), an
//! explicit program, or a running process to attach to; and the `launch`/`attach`
//! arguments netcoredbg takes.

use rc_dotnet::build::{BuildAction, BuildEvent, BuildRequest, build};
use rc_dotnet::{CancelToken, Dotnet, LaunchPlan, LaunchSettings, RunRequest};
use serde_json::{Map, Value, json};
use std::path::PathBuf;

/// The program the adapter starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaunchConfig {
    /// `dotnet` for framework-dependent apps (the dll is the first argument), or an apphost.
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    /// Variables added to the environment.
    pub env: Vec<(String, String)>,
}

impl From<&LaunchPlan> for LaunchConfig {
    fn from(p: &LaunchPlan) -> Self {
        LaunchConfig { program: p.program.clone(), args: p.args.clone(), cwd: p.working_dir.clone(), env: p.env.clone() }
    }
}

/// What a debug session runs.
#[derive(Clone, Debug)]
pub enum DebugTarget {
    /// Build (unless `run.build_first` is false), resolve the launch profile and launch.
    Project { dotnet: Dotnet, run: RunRequest },
    /// Launch this program directly.
    Launch(LaunchConfig),
    /// Attach to a running .NET process.
    Attach { pid: u32 },
}

impl DebugTarget {
    /// Debug a project with the defaults of [`RunRequest::new`] (Debug, build first, first
    /// `Project` launch profile).
    pub fn project(dotnet: &Dotnet, project: impl Into<PathBuf>) -> Self {
        DebugTarget::Project { dotnet: dotnet.clone(), run: RunRequest::new(project) }
    }

    pub fn is_attach(&self) -> bool {
        matches!(self, DebugTarget::Attach { .. })
    }
}

/// Debugger options of a session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DebugOptions {
    /// Stop on the first line of the program.
    pub stop_at_entry: bool,
    /// Step and break only in user code (code with symbols, not optimised).
    pub just_my_code: bool,
    /// Exception filters to enable (`all`, `user-unhandled`); `None` uses the adapter's
    /// defaults (those marked `default`, or none).
    pub exception_filters: Option<Vec<String>>,
    /// Frames fetched automatically on each stop (more on demand).
    pub top_frames: i64,
}

impl Default for DebugOptions {
    fn default() -> Self {
        DebugOptions { stop_at_entry: false, just_my_code: true, exception_filters: None, top_frames: 20 }
    }
}

/// `launch` arguments for netcoredbg (`type: coreclr`). Program output comes back as DAP
/// `output` events (stdout/stderr categories): netcoredbg pipes the debuggee's stdio.
pub fn launch_arguments(config: &LaunchConfig, options: &DebugOptions) -> Value {
    let env: Map<String, Value> = config.env.iter().map(|(k, v)| (k.clone(), Value::String(v.clone()))).collect();
    json!({
        "name": ".NET Launch",
        "type": "coreclr",
        "request": "launch",
        "program": config.program,
        "args": config.args,
        "cwd": config.cwd,
        "env": env,
        "console": "internalConsole",
        "stopAtEntry": options.stop_at_entry,
        "justMyCode": options.just_my_code,
        "enableStepFiltering": true,
    })
}

pub fn attach_arguments(pid: u32, options: &DebugOptions) -> Value {
    json!({
        "name": ".NET Attach",
        "type": "coreclr",
        "request": "attach",
        "processId": pid,
        "justMyCode": options.just_my_code,
    })
}

/// Builds the project (if asked) and resolves what to launch, like `rc_dotnet::run` does.
/// `Ok(None)` when cancelled.
pub async fn prepare(dotnet: &Dotnet, req: &RunRequest, cancel: &CancelToken, on_build: &mut (dyn FnMut(BuildEvent) + Send)) -> anyhow::Result<Option<LaunchPlan>> {
    if req.build_first {
        let mut b = BuildRequest::new(&req.project, BuildAction::Build);
        b.configuration = req.configuration.clone();
        b.target_framework = req.target_framework.clone();
        let r = build(dotnet, &b, cancel, on_build).await?;
        on_build(BuildEvent::Finished(r.clone()));
        if r.cancelled || cancel.is_cancelled() {
            return Ok(None);
        }
        anyhow::ensure!(r.success, "build failed with {} error(s)", r.errors);
    }
    let props = rc_dotnet::msbuild::evaluate(dotnet, &req.project, &req.configuration, req.target_framework.as_deref()).await?;
    if cancel.is_cancelled() {
        return Ok(None);
    }
    anyhow::ensure!(props.is_executable() || props.is_web, "{} is not an executable project", req.project.display());
    let settings = if req.use_launch_profile { LaunchSettings::load(&req.project)? } else { None };
    let plan = rc_dotnet::run::plan_launch(dotnet, req, &props, settings.as_ref())?;
    Ok(Some(plan))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_arguments_for_netcoredbg() {
        let cfg = LaunchConfig {
            program: "/usr/local/share/dotnet/dotnet".into(),
            args: vec!["/w/App/bin/Debug/net8.0/App.dll".into(), "Bob".into()],
            cwd: "/w/App".into(),
            env: vec![("ASPNETCORE_URLS".into(), "http://127.0.0.1:5000".into())],
        };
        let a = launch_arguments(&cfg, &DebugOptions::default());
        assert_eq!(a["program"], "/usr/local/share/dotnet/dotnet");
        assert_eq!(a["args"][0], "/w/App/bin/Debug/net8.0/App.dll");
        assert_eq!(a["env"]["ASPNETCORE_URLS"], "http://127.0.0.1:5000");
        assert_eq!(a["justMyCode"], true);
        assert_eq!(a["stopAtEntry"], false);
        assert_eq!(attach_arguments(42, &DebugOptions::default())["processId"], 42);
    }
}
