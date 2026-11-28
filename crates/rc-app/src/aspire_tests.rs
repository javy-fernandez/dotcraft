//! Aspire from the app, against a real app host: `dotnet new aspire-starter` (templates from
//! nuget.org into a private template hive in `target/test-home`), then Debug on the app host
//! must start the API service under its own netcoredbg session (the orchestrator asks the IDE
//! session endpoint) and stop on a breakpoint in the service's code during an HTTP request.

use crate::app::DotCraftApp;
use rc_core::{Settings, Waker};
use rc_editor::BreakpointKind;
use std::io::{Read, Write};
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

fn dotnet(args: &[&str], dir: &Path) {
    let out = Command::new("dotnet").args(args).current_dir(dir).env("DOTNET_CLI_TELEMETRY_OPTOUT", "1").env("DOTNET_NOLOGO", "1").output().expect("dotnet");
    assert!(out.status.success(), "dotnet {args:?} failed:\n{}\n{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
}

/// A new aspire-starter solution named Shop in `dir`.
fn starter(dir: &Path) -> std::path::PathBuf {
    let hive = crate::debugger_tests::test_home().join("aspire-templates");
    let hive_arg = hive.to_string_lossy().into_owned();
    let installed = Command::new("dotnet").args(["new", "list", "aspire-starter", "--debug:custom-hive", &hive_arg]).output().is_ok_and(|o| o.status.success());
    if !installed {
        dotnet(&["new", "install", "Aspire.ProjectTemplates", "--debug:custom-hive", &hive_arg], dir);
    }
    dotnet(&["new", "aspire-starter", "-n", "Shop", "-o", "Shop", "--debug:custom-hive", &hive_arg], dir);
    let root = dunce::canonicalize(dir.join("Shop")).unwrap();
    ["sln", "slnx"].iter().map(|e| root.join(format!("Shop.{e}"))).find(|p| p.is_file()).expect("solution")
}

fn pump(app: &mut DotCraftApp, what: &str, timeout: Duration, mut done: impl FnMut(&mut DotCraftApp) -> bool) {
    let start = Instant::now();
    while start.elapsed() < timeout {
        let outs = app.debugger.update(&mut app.docs);
        app.handle_debug(outs);
        app.update_aspire();
        if done(app) {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let tabs: Vec<String> = app.debugger.session_tabs().iter().map(|(_, n, s)| format!("{n}: {s:?}")).collect();
    let resources: Vec<String> = app.aspire.host.as_ref().map(|h| h.resources.iter().map(|r| format!("{} {:?}", r.name, r.state)).collect()).unwrap_or_default();
    let console = app.debugger.console.as_ref().map(|c| c.term.screen_lines().join("\n")).unwrap_or_default();
    panic!("timed out waiting for {what}\nsessions: {tabs:?}\nresources: {resources:?}\nconsole:\n{console}");
}

fn get(url: &str) -> String {
    let rest = url.trim_start_matches("http://");
    let (host, path) = rest.split_once('/').map(|(h, p)| (h.to_string(), format!("/{p}"))).unwrap_or((rest.to_string(), "/".into()));
    let mut stream = std::net::TcpStream::connect(host.replace("localhost", "127.0.0.1")).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(120))).unwrap();
    write!(stream, "GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n").unwrap();
    let mut body = String::new();
    let _ = stream.read_to_string(&mut body);
    body
}

#[test]
#[ignore = "Requires the .NET 10 SDK, nuget.org (Aspire templates and packages) and netcoredbg in target/test-home"]
fn debugging_the_app_host_stops_in_a_service() {
    let _lock = crate::screens::HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let netcoredbg = crate::debugger_tests::netcoredbg();
    let work = tempfile::tempdir().unwrap();
    let solution = starter(work.path());
    let root = solution.parent().unwrap().to_path_buf();
    let home = tempfile::tempdir().unwrap();
    // SAFETY: serialized by HOME_LOCK.
    unsafe { std::env::set_var("DOTCRAFT_HOME", home.path()) };
    let settings = Settings { roslyn_path: Some("/nonexistent".into()), netcoredbg_path: Some(netcoredbg), ..Settings::default() };
    let mut app = DotCraftApp::with_settings(settings, Waker::noop(), None);
    app.open_workspace(&solution);

    // The app host gets Aspire configurations (one per launch profile).
    let host = app.runner.configs.iter().position(|c| c.name == "Shop.AppHost: https").unwrap_or_else(|| panic!("{:?}", app.runner.configs));
    assert!(crate::aspire::is_app_host(&app.runner.configs[host]));
    assert_eq!(app.runner.configs[host].icon(), rc_kit::ico::CONFIG_ASPIRE);
    app.runner.selected = host;

    let program = root.join("Shop.ApiService/Program.cs");
    let line = std::fs::read_to_string(&program).unwrap().lines().position(|l| l.contains("var forecast = Enumerable.Range")).unwrap();
    app.open_file(&program);
    app.docs.active_doc_mut().unwrap().editor.breakpoints.insert(line, BreakpointKind::Normal);

    app.run_ide_action(&egui::Context::default(), rc_editor::Action::Debug);
    // The orchestrator asks for the API service: it runs in its own session, and the resource
    // service reports it running with its endpoints.
    pump(&mut app, "the API service", Duration::from_secs(600), |a| {
        let running = a.aspire.host.as_ref().and_then(|h| h.resources.iter().find(|r| r.display_name == "apiservice")).is_some_and(|r| r.state.as_deref() == Some("Running"));
        running && a.debugger.session_tabs().iter().any(|(_, name, _)| *name == "apiservice")
    });
    let host_state = app.aspire.host.as_ref().unwrap();
    assert!(host_state.connected);
    let api = host_state.resources.iter().find(|r| r.display_name == "apiservice").unwrap();
    let url = api.visible_urls().iter().map(|u| u.url.clone()).find(|u| u.starts_with("http://")).unwrap_or_else(|| panic!("{:?}", api.urls));
    assert!(app.debugger.dashboard.as_deref().is_some_and(|d| d.contains("/login?t=")), "{:?}", app.debugger.dashboard);

    // A request hits the breakpoint in the service; its session is selected.
    let request = std::thread::spawn(move || get(&format!("{}/weatherforecast", url.trim_end_matches('/'))));
    pump(&mut app, "the breakpoint in the service", Duration::from_secs(120), |a| a.debugger.stopped());
    assert_eq!(app.debugger.name, "apiservice");
    let doc = app.docs.active_doc().unwrap();
    assert_eq!(doc.path, program);
    assert_eq!(doc.editor.execution_line, Some(line));
    app.run_ide_action(&egui::Context::default(), rc_editor::Action::Resume);
    pump(&mut app, "the response", Duration::from_secs(60), |_| request.is_finished());
    let response = request.join().unwrap();
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");

    // Stop ends the app host and every service.
    app.run_ide_action(&egui::Context::default(), rc_editor::Action::StopRun);
    pump(&mut app, "the end of every session", Duration::from_secs(60), |a| !a.debugger.active() && a.aspire.host.as_ref().is_some_and(|h| !h.running));

    // Run (no debugger): the orchestrator starts the services itself; the Run tab links the
    // dashboard and the Services window lists the resources.
    app.run_ide_action(&egui::Context::default(), rc_editor::Action::Run);
    let running = |a: &mut DotCraftApp| {
        let outs = a.runner.update();
        crate::ui::run_windows::handle_outputs(a, outs);
        a.aspire.host.as_ref().is_some_and(|h| !h.debug && h.resources.iter().any(|r| r.display_name == "apiservice" && r.state.as_deref() == Some("Running")))
            && a.runner.runs.iter().any(|r| r.urls.first().is_some_and(|u| u.contains("/login?t=")))
    };
    pump(&mut app, "the app host to run", Duration::from_secs(300), running);
    assert!(!app.debugger.active(), "nothing runs under the debugger");
    assert!(app.aspire_dashboard().is_some_and(|d| d.contains("/login?t=")));
    app.run_ide_action(&egui::Context::default(), rc_editor::Action::StopRun);
    pump(&mut app, "the app host to stop", Duration::from_secs(60), |a| {
        let outs = a.runner.update();
        crate::ui::run_windows::handle_outputs(a, outs);
        a.aspire.host.as_ref().is_some_and(|h| !h.running)
    });
}
