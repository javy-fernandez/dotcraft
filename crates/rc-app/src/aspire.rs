//! .NET Aspire app hosts in the IDE (Rider's Aspire workflow). Running an app host
//! configuration starts it with a known resource service endpoint and API key, so the Services
//! window streams its resources (state, URLs, commands) and their consoles. Debugging it also
//! starts the IDE session endpoint: the orchestrator then asks DotCraft to start every .NET
//! project, and each one runs under its own netcoredbg session from the first line.

use crate::runner::{ConfigKind, Console, RunConfig};
use rc_core::{Inbox, Waker};
use rc_dotnet::aspire::{IdeSessionServer, Notifier, Resource, ResourceEvent, ResourceService, SessionRequest, WatchHandle};
use rc_dotnet::{RunEvent, RunHandle, RunRequest};
use rc_kit::i18n::{tr, trf};
use std::path::{Path, PathBuf};

/// What the Aspire integration asks the rest of the IDE to do.
#[derive(Debug)]
pub enum AspireOutput {
    /// Debug a service the orchestrator asked to run (run session `id`).
    DebugService { id: String, name: String, request: Box<RunRequest>, notifier: NotifierHandle },
    /// Stop the debugged service of run session `id`.
    StopService(String),
    Notify(String, bool),
}

/// [`Notifier`] with `Debug`, for [`AspireOutput`].
#[derive(Clone)]
pub struct NotifierHandle(pub Notifier);

impl std::fmt::Debug for NotifierHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Notifier")
    }
}

/// What is selected in the Aspire part of the Services window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Selection {
    Host,
    Resource(String),
}

/// One resource's console, streamed from the resource service.
pub struct ResourceLogs {
    pub resource: String,
    pub console: Console,
    _watch: WatchHandle,
}

/// A service started without the debugger for the orchestrator (`NoDebug` run sessions).
struct ServiceRun {
    id: String,
    handle: RunHandle,
    inbox: Inbox<RunEvent>,
    pid_sent: bool,
}

/// A running (or finished) app host.
pub struct AspireHost {
    /// The run configuration that started it.
    pub config: RunConfig,
    pub debug: bool,
    /// Its debug session in the debugger, once started.
    pub debug_session: Option<u64>,
    pub running: bool,
    /// The dashboard login URL it printed.
    pub dashboard: Option<String>,
    pub resources: Vec<Resource>,
    /// The resource service answered (else the app host is still starting).
    pub connected: bool,
    /// Commands in flight: (resource, command).
    pub pending: Vec<(String, String)>,
    /// The app host's run or debug session is followed (false for samples without one).
    pub tracked: bool,
    service: ResourceService,
    watch: Option<WatchHandle>,
    inbox: Inbox<ResourceEvent>,
    ide: Option<IdeSessionServer>,
    ide_inbox: Inbox<SessionRequest>,
    runs: Vec<ServiceRun>,
}

impl AspireHost {
    /// Resources to list (the dashboard's hidden ones left out), in the app model's order.
    pub fn visible(&self) -> Vec<&Resource> {
        self.resources.iter().filter(|r| !r.hidden).collect()
    }

    pub fn resource(&self, name: &str) -> Option<&Resource> {
        self.resources.iter().find(|r| r.name == name)
    }

    pub fn name(&self) -> String {
        match &self.config.kind {
            ConfigKind::App { project, .. } => project.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
            _ => self.config.name.clone(),
        }
    }

    pub fn busy(&self, resource: &str) -> bool {
        self.pending.iter().any(|(r, _)| r == resource)
    }
}

pub struct Aspire {
    waker: Waker,
    pub host: Option<AspireHost>,
    pub selection: Option<Selection>,
    pub logs: Option<ResourceLogs>,
    /// The Aspire section of the Services window is expanded.
    pub open: bool,
}

impl Aspire {
    pub fn new(waker: Waker) -> Self {
        Aspire { waker, host: None, selection: None, logs: None, open: true }
    }

    pub fn clear(&mut self) {
        self.host = None;
        self.selection = None;
        self.logs = None;
    }

    /// Prepares `request` for app host `config` (run or debug): the resource service endpoint
    /// and key, and for debugging the IDE session endpoint. Replaces the previous app host.
    pub fn start(&mut self, config: &RunConfig, request: &mut RunRequest, debug: bool) -> Result<(), String> {
        self.host = None;
        self.logs = None;
        let (env, endpoint) = rc_dotnet::aspire::host_environment(request);
        let service = ResourceService::new(endpoint).map_err(|e| e.to_string())?;
        request.env.extend(env);
        let ide_inbox = Inbox::new(self.waker.clone());
        let ide = if debug {
            let server = IdeSessionServer::start(ide_inbox.sender()).map_err(|e| trf("Could not start the Aspire session endpoint: {error}", &[("error", &e)]))?;
            request.env.extend(server.host_environment(true));
            Some(server)
        } else {
            None
        };
        let inbox = Inbox::new(self.waker.clone());
        let watch = service.watch(inbox.sender());
        self.host = Some(AspireHost {
            config: config.clone(),
            debug,
            debug_session: None,
            running: true,
            dashboard: None,
            resources: Vec::new(),
            connected: false,
            pending: Vec::new(),
            tracked: true,
            service,
            watch: Some(watch),
            inbox,
            ide,
            ide_inbox,
            runs: Vec::new(),
        });
        self.selection = Some(Selection::Host);
        Ok(())
    }

    /// A running app host with these resources and nothing behind it (captures and UI tests).
    #[cfg(test)]
    pub fn demo(&mut self, config: &RunConfig, resources: Vec<Resource>, dashboard: &str) {
        let endpoint = rc_dotnet::aspire::ResourceEndpoint { url: "https://localhost:1".into(), api_key: String::new() };
        self.host = Some(AspireHost {
            config: config.clone(),
            debug: true,
            debug_session: None,
            running: true,
            dashboard: Some(dashboard.into()),
            resources,
            connected: true,
            pending: Vec::new(),
            tracked: false,
            service: ResourceService::new(endpoint).expect("client"),
            watch: None,
            inbox: Inbox::new(self.waker.clone()),
            ide: None,
            ide_inbox: Inbox::new(self.waker.clone()),
            runs: Vec::new(),
        });
    }

    /// Shows `lines` as the selected resource's console (captures and UI tests).
    #[cfg(test)]
    pub fn demo_logs(&mut self, name: &str, lines: &[&str]) {
        self.selection = Some(Selection::Resource(name.to_string()));
        let console = Console::new(&self.waker);
        for line in lines {
            console.raw(&format!("{line}\n"), rc_dotnet::Stream::Stdout);
        }
        let watch = self.host.as_ref().expect("host").service.watch_logs(name, Inbox::new(self.waker.clone()).sender());
        self.logs = Some(ResourceLogs { resource: name.to_string(), console, _watch: watch });
    }

    /// The app host ended: its resources are gone.
    pub fn host_finished(&mut self) {
        let Some(host) = &mut self.host else { return };
        if !host.running {
            return;
        }
        host.running = false;
        host.watch = None;
        host.ide = None;
        host.runs.clear();
        host.resources.clear();
        host.pending.clear();
        host.connected = false;
        self.logs = None;
        if matches!(self.selection, Some(Selection::Resource(_))) {
            self.selection = Some(Selection::Host);
        }
    }

    /// Selects a resource and streams its console.
    pub fn select_resource(&mut self, name: &str) {
        self.selection = Some(Selection::Resource(name.to_string()));
        if self.logs.as_ref().is_some_and(|l| l.resource == name) {
            return;
        }
        self.logs = None;
        let Some(host) = &self.host else { return };
        if !host.running {
            return;
        }
        let console = Console::new(&self.waker);
        let watch = host.service.watch_logs(name, host.inbox.sender());
        self.logs = Some(ResourceLogs { resource: name.to_string(), console, _watch: watch });
    }

    /// Runs `command` (`resource-stop`, `resource-start`, `resource-restart`…) on a resource.
    pub fn execute(&mut self, resource: &str, command: &str) {
        let Some(host) = &mut self.host else { return };
        let Some(r) = host.resource(resource).cloned() else { return };
        host.pending.push((resource.to_string(), command.to_string()));
        host.service.execute(&r, command, host.inbox.sender());
    }

    pub fn update(&mut self, dotnet: &Path) -> Vec<AspireOutput> {
        let mut out = Vec::new();
        let Some(host) = &mut self.host else { return out };
        for event in host.inbox.drain() {
            match event {
                ResourceEvent::Snapshot(list) => {
                    host.connected = true;
                    host.resources = list;
                }
                ResourceEvent::Upsert(r) => {
                    host.connected = true;
                    match host.resources.iter_mut().find(|x| x.name == r.name) {
                        Some(x) => *x = r,
                        None => host.resources.push(r),
                    }
                }
                ResourceEvent::Deleted(name) => host.resources.retain(|r| r.name != name),
                ResourceEvent::Disconnected(_) => host.connected = false,
                ResourceEvent::Logs { resource, lines } => {
                    if let Some(logs) = self.logs.as_ref().filter(|l| l.resource == resource) {
                        for line in lines {
                            let stream = if line.stderr { rc_dotnet::Stream::Stderr } else { rc_dotnet::Stream::Stdout };
                            logs.console.raw(&format!("{}\n", line.text), stream);
                        }
                    }
                }
                ResourceEvent::CommandDone { resource, command, result } => {
                    if let Some(i) = host.pending.iter().position(|(r, c)| *r == resource && *c == command) {
                        host.pending.remove(i);
                    }
                    let name = host.resource(&resource).map(|r| r.display_name.clone()).unwrap_or(resource);
                    match result {
                        Ok(Some(message)) => out.push(AspireOutput::Notify(message, false)),
                        Ok(None) => {}
                        Err(error) => out.push(AspireOutput::Notify(format!("{name}: {error}"), true)),
                    }
                }
            }
        }
        for request in host.ide_inbox.drain() {
            let Some(notifier) = host.ide.as_ref().map(IdeSessionServer::notifier) else { continue };
            match request {
                SessionRequest::Start { id, session } => {
                    let name = service_name(&host.resources, &session.project);
                    let request = rc_dotnet::aspire::run_request(&session);
                    if session.debug {
                        out.push(AspireOutput::DebugService { id, name, request: Box::new(request), notifier: NotifierHandle(notifier) });
                    } else {
                        match rc_dotnet::Dotnet::locate(Some(dotnet)) {
                            Some(dotnet) => {
                                let inbox = Inbox::new(self.waker.clone());
                                let handle = rc_dotnet::start_run(&dotnet, request, inbox.sender());
                                host.runs.push(ServiceRun { id, handle, inbox, pid_sent: false });
                            }
                            None => {
                                notifier.message(&id, true, tr("The .NET SDK was not found. Install it or set its path in Settings › Tools."));
                                notifier.terminated(&id, None);
                            }
                        }
                    }
                }
                SessionRequest::Stop { id } => match host.runs.iter().find(|r| r.id == id) {
                    Some(run) => run.handle.stop(),
                    None => out.push(AspireOutput::StopService(id)),
                },
            }
        }
        if let Some(notifier) = host.ide.as_ref().map(IdeSessionServer::notifier) {
            host.runs.retain_mut(|run| {
                for event in run.inbox.drain() {
                    match event {
                        RunEvent::Started { pid, .. } if !run.pid_sent => {
                            run.pid_sent = true;
                            notifier.process_started(&run.id, pid);
                        }
                        RunEvent::Output(line) => notifier.log(&run.id, line.stream == rc_dotnet::Stream::Stderr, &line.text),
                        RunEvent::Exited { code, .. } => {
                            notifier.terminated(&run.id, code);
                            return false;
                        }
                        RunEvent::Failed(error) => {
                            notifier.message(&run.id, true, &error);
                            notifier.terminated(&run.id, None);
                            return false;
                        }
                        _ => {}
                    }
                }
                true
            });
        }
        out
    }
}

/// The resource name of the project DCP asked to run (`apiservice`), else the project's name.
fn service_name(resources: &[Resource], project: &Path) -> String {
    let same = |p: &str| dunce::canonicalize(p).ok() == dunce::canonicalize(project).ok() || Path::new(p) == project;
    resources
        .iter()
        .find(|r| r.project_path().is_some_and(same))
        .map(|r| r.display_name.clone())
        .unwrap_or_else(|| project.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default())
}

/// Whether a run configuration starts an Aspire app host.
pub fn is_app_host(config: &RunConfig) -> bool {
    matches!(config.kind, ConfigKind::App { aspire: true, .. })
}

/// The project of an app host configuration.
pub fn host_project(config: &RunConfig) -> Option<PathBuf> {
    match &config.kind {
        ConfigKind::App { project, aspire: true, .. } => Some(project.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn services_are_named_after_their_resource() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("Shop.Api.csproj");
        std::fs::write(&project, "<Project/>").unwrap();
        let mut api = Resource { name: "apiservice-x1".into(), display_name: "apiservice".into(), ..Default::default() };
        api.properties.push(rc_dotnet::aspire::resources::ResourceProperty { name: "project.path".into(), value: project.display().to_string(), ..Default::default() });
        assert_eq!(service_name(&[api], &project), "apiservice");
        assert_eq!(service_name(&[], &project), "Shop.Api");
    }
}
