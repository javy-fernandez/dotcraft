//! Running a project: optional build, launch profile (args, env, working dir, ASP.NET URLs),
//! then `dotnet <TargetPath>` with stdout/stderr streamed (ANSI kept) and stdin forwarded.
//! Stopping sends SIGTERM to the tree (graceful ASP.NET shutdown) and kills it after a grace.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use rc_core::Runtime;
use rc_core::inbox::Outbox;
use tokio::sync::mpsc;

use crate::build::{BuildAction, BuildEvent, BuildRequest, build};
use crate::launch::{LaunchProfile, LaunchSettings, split_urls};
use crate::msbuild::{ProjectProperties, evaluate};
use crate::process::{CancelToken, OpHandle, OutputLine, StreamOpts, run_streaming, split_command_line};
use crate::sdk::Dotnet;

/// What to run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunRequest {
    pub project: PathBuf,
    pub configuration: String,
    pub target_framework: Option<String>,
    /// Profile of `launchSettings.json`; `None` picks the first `Project` profile.
    pub launch_profile: Option<String>,
    /// Ignore `launchSettings.json` when false (`dotnet run --no-launch-profile`).
    pub use_launch_profile: bool,
    /// Program arguments; when empty, the profile's `commandLineArgs`.
    pub args: Vec<String>,
    /// Extra environment, over the profile's.
    pub env: Vec<(String, String)>,
    /// Overrides the profile's working directory and the project folder.
    pub working_dir: Option<PathBuf>,
    pub build_first: bool,
}

impl RunRequest {
    pub fn new(project: impl Into<PathBuf>) -> Self {
        RunRequest {
            project: project.into(),
            configuration: "Debug".into(),
            target_framework: None,
            launch_profile: None,
            use_launch_profile: true,
            args: Vec::new(),
            env: Vec::new(),
            working_dir: None,
            build_first: true,
        }
    }
}

/// The resolved process to start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaunchPlan {
    pub program: PathBuf,
    pub args: Vec<String>,
    /// Variables set on top of the IDE's environment.
    pub env: Vec<(String, String)>,
    pub working_dir: PathBuf,
    /// URLs the app listens on (`ASPNETCORE_URLS`), for web projects.
    pub urls: Vec<String>,
    pub profile: Option<String>,
    /// Page to open when the profile has `launchBrowser`.
    pub browser_url: Option<String>,
}

impl LaunchPlan {
    /// A readable command line for the console header.
    pub fn command_line(&self) -> String {
        let quote = |s: &str| if s.contains(' ') || s.is_empty() { format!("\"{s}\"") } else { s.to_string() };
        std::iter::once(quote(&self.program.to_string_lossy())).chain(self.args.iter().map(|a| quote(a))).collect::<Vec<_>>().join(" ")
    }

    pub fn env_var(&self, key: &str) -> Option<&str> {
        self.env.iter().rev().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }
}

/// Resolves the launch (pure: no processes, for tests).
pub fn plan_launch(dotnet: &Dotnet, req: &RunRequest, props: &ProjectProperties, settings: Option<&LaunchSettings>) -> anyhow::Result<LaunchPlan> {
    let profile: Option<&LaunchProfile> = match (req.use_launch_profile, settings, &req.launch_profile) {
        (false, ..) | (true, None, None) => None,
        (true, None, Some(name)) => anyhow::bail!("launch profile '{name}' not found (no launchSettings.json)"),
        (true, Some(s), Some(name)) => Some(s.profile(name).ok_or_else(|| anyhow::anyhow!("launch profile '{name}' not found"))?),
        (true, Some(s), None) => s.default_profile(),
    };
    let project_dir = if props.project_dir.as_os_str().is_empty() { req.project.parent().unwrap_or(Path::new(".")).to_path_buf() } else { props.project_dir.clone() };
    let expand = |s: &str| expand_vars(s, props, &project_dir);

    let (program, mut args) = match profile {
        Some(p) if p.command_name.eq_ignore_ascii_case("Executable") => {
            let exe = p.executable_path.as_deref().ok_or_else(|| anyhow::anyhow!("profile '{}' has no executablePath", p.name))?;
            (PathBuf::from(expand(exe)), Vec::new())
        }
        Some(p) if !p.is_project() => anyhow::bail!("launch profile '{}' ({}) cannot be run here", p.name, p.command_name),
        _ => {
            anyhow::ensure!(!props.target_path.as_os_str().is_empty(), "the project has no TargetPath");
            let run_command = props.raw.get("RunCommand").map(|s| s.trim()).filter(|s| !s.is_empty());
            if let (Some(cmd), true) = (run_command, !framework_dependent(&props.target_path)) {
                // Not on a shared framework (self-contained, or Blazor WebAssembly, which `dotnet run`
                // serves with its dev server): `dotnet app.dll` cannot run it, so start what
                // `dotnet run` starts. A bare `dotnet` is the SDK in use (apps opened from Finder
                // have no `dotnet` on PATH).
                let program = if matches!(cmd, "dotnet" | "dotnet.exe") { dotnet.path.clone() } else { PathBuf::from(cmd) };
                (program, split_command_line(props.raw.get("RunArguments").map(String::as_str).unwrap_or("")))
            } else if props.target_path.extension().is_some_and(|e| e.eq_ignore_ascii_case("dll")) {
                (dotnet.path.clone(), vec![props.target_path.to_string_lossy().into_owned()])
            } else {
                (props.target_path.clone(), Vec::new())
            }
        }
    };
    if !req.args.is_empty() {
        args.extend(req.args.iter().cloned());
    } else if let Some(cla) = profile.and_then(|p| p.command_line_args.as_deref()) {
        args.extend(split_command_line(&expand(cla)));
    }

    let mut env: Vec<(String, String)> = Vec::new();
    let set = |env: &mut Vec<(String, String)>, k: &str, v: String| {
        env.retain(|(ek, _)| ek != k);
        env.push((k.to_string(), v));
    };
    if let Some(p) = profile {
        for (k, v) in &p.environment_variables {
            set(&mut env, k, expand(v));
        }
    }
    for (k, v) in &req.env {
        set(&mut env, k, v.clone());
    }
    let has = |env: &[(String, String)], k: &str| env.iter().any(|(ek, _)| ek == k);
    if let Some(url) = profile.and_then(|p| p.application_url.clone())
        && !has(&env, "ASPNETCORE_URLS")
    {
        set(&mut env, "ASPNETCORE_URLS", url);
    }
    let urls = env.iter().find(|(k, _)| k == "ASPNETCORE_URLS").map(|(_, v)| split_urls(v)).unwrap_or_default();
    if !has(&env, "DOTNET_SYSTEM_CONSOLE_ALLOW_ANSI_COLOR_REDIRECTION") {
        // Keep colours although stdout is a pipe: the run console renders ANSI.
        set(&mut env, "DOTNET_SYSTEM_CONSOLE_ALLOW_ANSI_COLOR_REDIRECTION", "1".into());
    }

    let run_dir = props.raw.get("RunWorkingDirectory").map(|s| s.trim()).filter(|s| !s.is_empty()).map(PathBuf::from);
    let working_dir = match (&req.working_dir, profile.and_then(|p| p.working_directory.as_deref())) {
        (Some(d), _) => d.clone(),
        (None, None) if run_dir.is_some() => run_dir.clone().unwrap_or_default(),
        (None, Some(w)) if !w.trim().is_empty() => {
            let w = PathBuf::from(expand(w));
            if w.is_absolute() { w } else { project_dir.join(w) }
        }
        _ => project_dir.clone(),
    };

    let browser_url = profile.filter(|p| p.launch_browser).and_then(|p| {
        let base = urls.first().cloned().or_else(|| p.urls().into_iter().next())?;
        let probe = LaunchProfile { application_url: Some(base), ..p.clone() };
        probe.browser_url()
    });

    Ok(LaunchPlan { program, args, env, working_dir, urls, profile: profile.map(|p| p.name.clone()), browser_url })
}

/// Whether the built app runs on a shared framework (its `runtimeconfig.json` names one), so
/// `dotnet app.dll` can start it. Self-contained apps list `includedFrameworks` instead. When the
/// file cannot be read, assume framework-dependent (the usual case).
fn framework_dependent(target_path: &Path) -> bool {
    let config = target_path.with_extension("runtimeconfig.json");
    let Ok(text) = std::fs::read_to_string(&config) else { return true };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else { return true };
    let options = &json["runtimeOptions"];
    options.get("framework").is_some() || options.get("frameworks").is_some()
}

/// Expands `$(ProjectDir)`, `$(TargetDir)` and `$(TargetPath)`.
fn expand_vars(s: &str, props: &ProjectProperties, project_dir: &Path) -> String {
    let with_sep = |p: &Path| {
        let mut s = p.to_string_lossy().into_owned();
        if !s.is_empty() && !s.ends_with(std::path::MAIN_SEPARATOR) {
            s.push(std::path::MAIN_SEPARATOR);
        }
        s
    };
    let target_dir = props.target_path.parent().map(with_sep).unwrap_or_default();
    s.replace("$(ProjectDir)", &with_sep(project_dir)).replace("$(TargetDir)", &target_dir).replace("$(TargetPath)", &props.target_path.to_string_lossy())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunEvent {
    /// Events of the build before the launch.
    Build(BuildEvent),
    Started { pid: u32, plan: LaunchPlan },
    Output(OutputLine),
    /// Last event of a run that started.
    Exited { code: Option<i32>, cancelled: bool, duration: Duration },
    /// Last event when the build failed or the process could not start.
    Failed(String),
}

/// A running (or finished) program. `stop()` or dropping it kills the process tree.
pub struct RunHandle {
    op: OpHandle,
    input: mpsc::UnboundedSender<Vec<u8>>,
    pid: Arc<AtomicU32>,
}

impl RunHandle {
    /// Stops the program (graceful first, then killed after a few seconds).
    pub fn stop(&self) {
        self.op.cancel();
    }

    pub fn is_finished(&self) -> bool {
        self.op.is_finished()
    }

    /// Process id once started.
    pub fn pid(&self) -> Option<u32> {
        Some(self.pid.load(Ordering::SeqCst)).filter(|&p| p != 0)
    }

    /// Writes to the program's stdin; false when it is gone.
    pub fn send_input(&self, text: &str) -> bool {
        self.input.send(text.as_bytes().to_vec()).is_ok()
    }
}

/// Time between the graceful stop request and the kill.
pub const STOP_GRACE: Duration = Duration::from_secs(5);

/// Builds (if asked) and starts the project on the shared runtime; events go to `outbox`.
pub fn start_run(dotnet: &Dotnet, req: RunRequest, outbox: Outbox<RunEvent>) -> RunHandle {
    let cancel = CancelToken::new();
    let token = cancel.clone();
    let dotnet = dotnet.clone();
    let (input, input_rx) = mpsc::unbounded_channel();
    let pid = Arc::new(AtomicU32::new(0));
    let pid2 = pid.clone();
    let task = Runtime::spawn(async move {
        let out = outbox.clone();
        let r = run(&dotnet, &req, &token, input_rx, &pid2, &mut |e| {
            out.send(e);
        })
        .await;
        outbox.send(match r {
            Ok(e) => e,
            Err(e) => RunEvent::Failed(format!("{e:#}")),
        });
    });
    RunHandle { op: OpHandle::new(cancel, task), input, pid }
}

/// Runs to the end; returns the last event (`Exited` or `Failed`) instead of reporting it.
pub async fn run(dotnet: &Dotnet, req: &RunRequest, cancel: &CancelToken, input: mpsc::UnboundedReceiver<Vec<u8>>, pid: &AtomicU32, on_event: &mut (dyn FnMut(RunEvent) + Send)) -> anyhow::Result<RunEvent> {
    if req.build_first {
        let mut b = BuildRequest::new(&req.project, BuildAction::Build);
        b.configuration = req.configuration.clone();
        b.target_framework = req.target_framework.clone();
        let r = build(dotnet, &b, cancel, &mut |e| on_event(RunEvent::Build(e))).await?;
        on_event(RunEvent::Build(BuildEvent::Finished(r.clone())));
        if r.cancelled {
            return Ok(RunEvent::Exited { code: None, cancelled: true, duration: Duration::ZERO });
        }
        if !r.success {
            return Ok(RunEvent::Failed(format!("build failed with {} error(s)", r.errors)));
        }
    }
    let props = evaluate(dotnet, &req.project, &req.configuration, req.target_framework.as_deref()).await?;
    let settings = if req.use_launch_profile { LaunchSettings::load(&req.project)? } else { None };
    let plan = plan_launch(dotnet, req, &props, settings.as_ref())?;

    let mut cmd = tokio::process::Command::new(&plan.program);
    cmd.args(&plan.args).current_dir(&plan.working_dir);
    for (k, v) in &plan.env {
        cmd.env(k, v);
    }
    let started = Instant::now();
    let events = std::sync::Mutex::new(on_event);
    let opts = StreamOpts { partial_lines: true, grace: STOP_GRACE, stdin: Some(input) };
    let outcome = run_streaming(
        cmd,
        cancel,
        opts,
        &mut |p| {
            pid.store(p, Ordering::SeqCst);
            (events.lock().unwrap())(RunEvent::Started { pid: p, plan: plan.clone() });
        },
        &mut |line| (events.lock().unwrap())(RunEvent::Output(line)),
    )
    .await
    .map_err(|e| anyhow::anyhow!("cannot start {}: {e}", plan.program.display()))?;
    Ok(RunEvent::Exited { code: outcome.code, cancelled: outcome.cancelled, duration: started.elapsed() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn props() -> ProjectProperties {
        ProjectProperties {
            target_framework: "net8.0".into(),
            target_path: PathBuf::from("/w/Api/bin/Debug/net8.0/Api.dll"),
            project_dir: PathBuf::from("/w/Api/"),
            is_web: true,
            ..Default::default()
        }
    }

    fn settings() -> LaunchSettings {
        LaunchSettings::parse(
            r#"{"profiles":{
              "IIS Express":{"commandName":"IISExpress"},
              "http":{"commandName":"Project","launchBrowser":true,"launchUrl":"swagger","applicationUrl":"http://localhost:5112",
                      "commandLineArgs":"--mode \"a b\"","workingDirectory":"$(TargetDir)",
                      "environmentVariables":{"ASPNETCORE_ENVIRONMENT":"Development","ROOT":"$(ProjectDir)data"}},
              "tool":{"commandName":"Executable","executablePath":"/bin/echo","commandLineArgs":"hi"}}}"#,
        )
        .unwrap()
    }

    #[test]
    fn plans_the_default_project_profile() {
        let dotnet = Dotnet::new("/usr/local/share/dotnet/dotnet");
        let req = RunRequest::new("/w/Api/Api.csproj");
        let plan = plan_launch(&dotnet, &req, &props(), Some(&settings())).unwrap();
        assert_eq!(plan.profile.as_deref(), Some("http"));
        assert_eq!(plan.program, dotnet.path);
        assert_eq!(plan.args, vec!["/w/Api/bin/Debug/net8.0/Api.dll", "--mode", "a b"]);
        assert_eq!(plan.env_var("ASPNETCORE_URLS"), Some("http://localhost:5112"));
        assert_eq!(plan.env_var("ASPNETCORE_ENVIRONMENT"), Some("Development"));
        assert_eq!(plan.env_var("ROOT"), Some("/w/Api/data"));
        assert_eq!(plan.urls, vec!["http://localhost:5112"]);
        assert_eq!(plan.working_dir, PathBuf::from("/w/Api/bin/Debug/net8.0/"));
        assert_eq!(plan.browser_url.as_deref(), Some("http://localhost:5112/swagger"));
    }

    #[test]
    fn request_overrides_args_env_urls_and_working_dir() {
        let dotnet = Dotnet::new("dotnet");
        let mut req = RunRequest::new("/w/Api/Api.csproj");
        req.args = vec!["x".into()];
        req.env = vec![("ASPNETCORE_URLS".into(), "http://127.0.0.1:6001".into())];
        req.working_dir = Some("/tmp".into());
        let plan = plan_launch(&dotnet, &req, &props(), Some(&settings())).unwrap();
        assert_eq!(plan.args[1..], ["x"]);
        assert_eq!(plan.urls, vec!["http://127.0.0.1:6001"]);
        assert_eq!(plan.browser_url.as_deref(), Some("http://127.0.0.1:6001/swagger"));
        assert_eq!(plan.working_dir, PathBuf::from("/tmp"));
    }

    #[test]
    fn handles_executable_missing_and_disabled_profiles() {
        let dotnet = Dotnet::new("dotnet");
        let mut req = RunRequest::new("/w/Api/Api.csproj");
        req.launch_profile = Some("tool".into());
        let plan = plan_launch(&dotnet, &req, &props(), Some(&settings())).unwrap();
        assert_eq!((plan.program, plan.args), (PathBuf::from("/bin/echo"), vec!["hi".to_string()]));

        req.launch_profile = Some("IIS Express".into());
        assert!(plan_launch(&dotnet, &req, &props(), Some(&settings())).is_err());
        req.launch_profile = Some("nope".into());
        assert!(plan_launch(&dotnet, &req, &props(), Some(&settings())).is_err());

        req.use_launch_profile = false;
        let plan = plan_launch(&dotnet, &req, &props(), Some(&settings())).unwrap();
        assert_eq!(plan.profile, None);
        assert!(plan.urls.is_empty());
        assert_eq!(plan.working_dir, PathBuf::from("/w/Api/"));

        let mut exe = props();
        exe.target_path = PathBuf::from("/w/Tool/bin/tool.exe");
        let plan = plan_launch(&dotnet, &RunRequest::new("/w/Tool/Tool.csproj"), &exe, None).unwrap();
        assert_eq!(plan.program, PathBuf::from("/w/Tool/bin/tool.exe"));
        assert!(plan.command_line().starts_with("/w/Tool/bin/tool.exe"));
    }
}
