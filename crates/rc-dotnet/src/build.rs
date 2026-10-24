//! `dotnet build` / rebuild / clean / restore on a solution or project, streamed as
//! [`BuildEvent`]s with parsed diagnostics and a final [`BuildResult`].

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rc_core::Runtime;
use rc_core::inbox::Outbox;

use crate::diagnostics::{self, Diagnostic, Severity};
use crate::process::{CancelToken, OpHandle, OutputLine, StreamOpts, run_streaming};
use crate::sdk::Dotnet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BuildAction {
    Build,
    Rebuild,
    Clean,
    Restore,
}

/// What to build and how.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuildRequest {
    /// `.sln`, `.slnx` or project file (or a folder with one).
    pub target: PathBuf,
    pub action: BuildAction,
    /// `Debug` by default.
    pub configuration: String,
    pub target_framework: Option<String>,
    pub no_restore: bool,
    /// Extra `-p:Name=Value` properties.
    pub properties: Vec<(String, String)>,
    pub extra_args: Vec<String>,
}

impl BuildRequest {
    pub fn new(target: impl Into<PathBuf>, action: BuildAction) -> Self {
        BuildRequest {
            target: target.into(),
            action,
            configuration: "Debug".into(),
            target_framework: None,
            no_restore: false,
            properties: Vec::new(),
            extra_args: Vec::new(),
        }
    }

    /// Arguments after `dotnet`.
    pub fn args(&self) -> Vec<String> {
        let mut a: Vec<String> = Vec::new();
        a.push(match self.action {
            BuildAction::Build | BuildAction::Rebuild => "build",
            BuildAction::Clean => "clean",
            BuildAction::Restore => "restore",
        }
        .into());
        a.push(self.target.to_string_lossy().into_owned());
        if self.action != BuildAction::Restore {
            a.extend(["-c".into(), self.configuration.clone()]);
            if let Some(tf) = &self.target_framework {
                a.extend(["-f".into(), tf.clone()]);
            }
        }
        if self.action == BuildAction::Rebuild {
            a.push("--no-incremental".into());
        }
        if self.no_restore && matches!(self.action, BuildAction::Build | BuildAction::Rebuild) {
            a.push("--no-restore".into());
        }
        a.extend(["-nologo", "-v:minimal", "-clp:NoSummary", "-tl:off", "-p:GenerateFullPaths=true"].map(String::from));
        for (k, v) in &self.properties {
            a.push(format!("-p:{k}={v}"));
        }
        a.extend(self.extra_args.iter().cloned());
        a
    }
}

/// Progress of a build.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BuildEvent {
    /// The command line, once the process started.
    Started { command: String },
    Output(OutputLine),
    Diagnostic(Diagnostic),
    /// Always the last event (also when cancelled).
    Finished(BuildResult),
    /// The process could not be started (no `dotnet`…); also the last event.
    Failed(String),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BuildResult {
    pub success: bool,
    pub cancelled: bool,
    pub exit_code: Option<i32>,
    pub errors: usize,
    pub warnings: usize,
    /// Unique diagnostics in output order.
    pub diagnostics: Vec<Diagnostic>,
    pub duration: Duration,
}

impl BuildResult {
    pub fn errors(&self) -> impl Iterator<Item = &Diagnostic> {
        self.diagnostics.iter().filter(|d| d.severity == Severity::Error)
    }
}

/// Starts a build on the shared runtime; events go to `outbox` (ending with `Finished` or
/// `Failed`). Dropping or cancelling the handle kills the build.
pub fn start_build(dotnet: &Dotnet, req: BuildRequest, outbox: Outbox<BuildEvent>) -> OpHandle {
    let cancel = CancelToken::new();
    let token = cancel.clone();
    let dotnet = dotnet.clone();
    let task = Runtime::spawn(async move {
        let out = outbox.clone();
        match build(&dotnet, &req, &token, &mut |e| {
            out.send(e);
        })
        .await
        {
            Ok(r) => outbox.send(BuildEvent::Finished(r)),
            Err(e) => outbox.send(BuildEvent::Failed(format!("{e:#}"))),
        };
    });
    OpHandle::new(cancel, task)
}

/// Runs a build to its end, reporting events (except the final one) to `on_event`.
pub async fn build(dotnet: &Dotnet, req: &BuildRequest, cancel: &CancelToken, on_event: &mut (dyn FnMut(BuildEvent) + Send)) -> anyhow::Result<BuildResult> {
    let started = Instant::now();
    let args = req.args();
    let mut cmd = dotnet.command();
    cmd.args(&args);
    if let Some(dir) = working_dir(&req.target) {
        cmd.current_dir(dir);
    }
    let command = format!("dotnet {}", args.join(" "));
    let mut diags: Vec<Diagnostic> = Vec::new();
    let mut on_start = |_pid: u32| {};
    let outcome = {
        let diags = &mut diags;
        let on_event_ref = &mut *on_event;
        on_event_ref(BuildEvent::Started { command });
        let mut on_line = |line: OutputLine| {
            let d = diagnostics::parse_line(&line.text);
            on_event_ref(BuildEvent::Output(line));
            if let Some(d) = d
                && !diags.contains(&d)
            {
                diags.push(d.clone());
                on_event_ref(BuildEvent::Diagnostic(d));
            }
        };
        run_streaming(cmd, cancel, StreamOpts::default(), &mut on_start, &mut on_line).await?
    };
    let errors = diags.iter().filter(|d| d.severity == Severity::Error).count();
    let warnings = diags.iter().filter(|d| d.severity == Severity::Warning).count();
    Ok(BuildResult {
        success: !outcome.cancelled && outcome.code == Some(0) && errors == 0,
        cancelled: outcome.cancelled,
        exit_code: outcome.code,
        errors,
        warnings,
        diagnostics: diags,
        duration: started.elapsed(),
    })
}

fn working_dir(target: &Path) -> Option<&Path> {
    if target.is_dir() { Some(target) } else { target.parent().filter(|p| !p.as_os_str().is_empty()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_the_command_lines() {
        let mut r = BuildRequest::new("/w/Shop.sln", BuildAction::Build);
        r.properties.push(("ContinuousIntegrationBuild".into(), "true".into()));
        let a = r.args();
        assert_eq!(&a[..4], ["build", "/w/Shop.sln", "-c", "Debug"]);
        assert!(a.contains(&"-p:GenerateFullPaths=true".to_string()));
        assert!(a.contains(&"-p:ContinuousIntegrationBuild=true".to_string()));

        let mut r = BuildRequest::new("/w/A.csproj", BuildAction::Rebuild);
        r.configuration = "Release".into();
        r.target_framework = Some("net8.0".into());
        r.no_restore = true;
        let a = r.args();
        assert_eq!(&a[..7], ["build", "/w/A.csproj", "-c", "Release", "-f", "net8.0", "--no-incremental"]);
        assert!(a.contains(&"--no-restore".to_string()));

        assert_eq!(BuildRequest::new("/w/A.csproj", BuildAction::Clean).args()[0], "clean");
        let a = BuildRequest::new("/w/A.csproj", BuildAction::Restore).args();
        assert_eq!(a[0], "restore");
        assert!(!a.contains(&"-c".to_string()));
    }
}
