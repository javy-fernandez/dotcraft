//! Remote back end end to end, with the agent served in this process (the "remote machine"
//! is this one, reached through the same streams as over SSH): Roslyn answers through the
//! agent with its paths mapped to the mirror, and netcoredbg stops at a breakpoint set on a
//! mirror file. Uses the Roslyn and netcoredbg of `target/test-home` as the remote tools.

use crate::insight::{InsightOutput, InsightStatus, NavKind};
use crate::remote_backend::test_support::backend;
use rc_core::Runtime;
use rc_dap::{BreakpointSpec, Breakpoints, DebugEvent, DebugOptions, DebugSession, DebugTarget};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn copy_tree(source: &Path, target: &Path) {
    std::fs::create_dir_all(target).unwrap();
    for entry in std::fs::read_dir(source).unwrap().flatten() {
        let name = entry.file_name();
        if name == "bin" || name == "obj" {
            continue;
        }
        if entry.path().is_dir() {
            copy_tree(&entry.path(), &target.join(&name));
        } else {
            std::fs::copy(entry.path(), target.join(&name)).unwrap();
        }
    }
}

#[test]
fn remote_roslyn_answers_through_the_agent_with_mirror_paths() {
    let _lock = crate::screens::HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let remote = tempfile::tempdir().unwrap();
    let root = dunce::canonicalize(remote.path()).unwrap();
    write(&root.join("Lib/Lib.csproj"), "<Project Sdk=\"Microsoft.NET.Sdk\">\n  <PropertyGroup><TargetFramework>net8.0</TargetFramework></PropertyGroup>\n</Project>\n");
    write(&root.join("Lib/Greeter.cs"), "namespace Lib;\n\npublic class Greeter\n{\n    public string Hello(string name) => $\"Hello {name}\";\n}\n");
    write(&root.join("App/App.csproj"), "<Project Sdk=\"Microsoft.NET.Sdk\">\n  <PropertyGroup><OutputType>Exe</OutputType><TargetFramework>net8.0</TargetFramework></PropertyGroup>\n  <ItemGroup><ProjectReference Include=\"..\\Lib\\Lib.csproj\" /></ItemGroup>\n</Project>\n");
    write(&root.join("App/Program.cs"), "using Lib;\n\nvar greeter = new Greeter();\nSystem.Console.WriteLine(greeter.Hello(\"world\"));\nint unused = undefinedThing;\n");
    write(&root.join("Demo.slnx"), "<Solution>\n  <Project Path=\"App/App.csproj\" />\n  <Project Path=\"Lib/Lib.csproj\" />\n</Solution>\n");
    assert!(std::process::Command::new("dotnet").args(["restore", root.join("Demo.slnx").to_str().unwrap()]).output().unwrap().status.success());

    let mirror_dir = tempfile::tempdir().unwrap();
    let backend = backend(&root, &dunce::canonicalize(mirror_dir.path()).unwrap());
    let dll = backend.tools.roslyn_dll.clone().expect("Roslyn in target/test-home: run the rc-lsp tests first");
    Runtime::block_on(backend.sync_folder()).unwrap();
    let local = |rel: &str| backend.local(&format!("{}/{rel}", root.display())).unwrap();

    let home = tempfile::tempdir().unwrap();
    let mut app = crate::screens::test_app(home.path(), true);
    let connect_backend = backend.clone();
    let connect: crate::insight::RemoteConnect = Arc::new(move |out| {
        let map = rc_lsp::remote::UriMap { mirror: connect_backend.mirror.clone() };
        rc_lsp::Client::connect(connect_backend.lsp_transport(dll.clone()), Some(Arc::new(map)), out)
    });
    app.insight.start_remote(&local(""), Some(&local("Demo.slnx")), Vec::new(), connect);
    assert!(app.insight.is_remote());
    let mut outputs = Vec::new();
    let mut pump = |app: &mut crate::app::DotCraftApp, what: &str, done: &dyn Fn(&crate::app::DotCraftApp, &[InsightOutput]) -> bool| {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(180) {
            outputs.extend(app.insight.update(&mut app.docs, true, false));
            app.docs.poll_parses();
            if let InsightStatus::Failed(why) = &app.insight.status {
                panic!("server failed: {why}");
            }
            if done(app, &outputs) {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("timed out waiting for {what}; status {:?}", app.insight.status);
    };
    pump(&mut app, "projects", &|a, _| a.insight.status == InsightStatus::Ready);

    let program = local("App/Program.cs");
    app.docs.open(&program).unwrap();
    // Diagnostics come back for the mirror path.
    pump(&mut app, "diagnostics", &|a, _| a.docs.active_doc().is_some_and(|d| d.editor.diagnostics.iter().any(|x| x.code.as_deref() == Some("CS0103"))));
    assert!(app.insight.problems.contains_key(&program));

    // Completion after "greeter.".
    let doc = app.docs.active_doc_mut().unwrap();
    let end = doc.editor.buffer.len_chars();
    doc.editor.replace(end, end, "greeter.");
    doc.editor.selection = rc_text::Selection::point(end + 8);
    let i = app.docs.active.unwrap();
    app.insight.on_editor_events(&mut app.docs.list[i], &[rc_editor::EditorEvent::Typed('.')]);
    pump(&mut app, "completion", &|a, _| a.docs.active_doc().and_then(|d| d.editor.popups.completion.as_ref()).is_some_and(|c| !c.filtered.is_empty()));
    let c = app.docs.active_doc().unwrap().editor.popups.completion.clone().unwrap();
    assert!(c.filtered.iter().any(|&k| c.items[k].label == "Hello"));

    // Go to declaration of `Hello`: the other project's file, as its mirror path.
    let doc = app.docs.active_doc().unwrap();
    let pos = doc.editor.buffer.rope().to_string().find("greeter.Hello(").unwrap() + "greeter.".len() + 1;
    app.insight.navigate(&app.docs.list[i], pos, NavKind::Declaration);
    pump(&mut app, "the declaration", &|_, out| out.iter().any(|o| matches!(o, InsightOutput::Navigate(_))));
    let target: PathBuf = outputs.iter().find_map(|o| if let InsightOutput::Navigate(t) = o { Some(t.path.clone()) } else { None }).unwrap();
    assert_eq!(target, local("Lib/Greeter.cs"));
    app.insight.stop();
}

#[test]
fn remote_netcoredbg_stops_at_a_breakpoint_in_the_shop_fixture() {
    let remote = tempfile::tempdir().unwrap();
    let root = dunce::canonicalize(remote.path()).unwrap();
    copy_tree(&crate::screens::fixtures().join("Shop"), &root);
    let mirror_dir = tempfile::tempdir().unwrap();
    let backend = backend(&root, &dunce::canonicalize(mirror_dir.path()).unwrap());
    let exe = backend.tools.netcoredbg.clone().expect("netcoredbg in target/test-home: run the rc-dap tests first");
    Runtime::block_on(backend.sync_folder()).unwrap();
    let project = format!("{}/src/Shop.Cli/Shop.Cli.csproj", root.display());

    // Built on the "remote machine", output streamed.
    let mut build = Runtime::block_on(backend.dotnet_stream(&["build", &project])).unwrap();
    let mut log = String::new();
    Runtime::block_on(async {
        use tokio::io::AsyncReadExt;
        build.stdout.take().unwrap().read_to_string(&mut log).await.unwrap();
    });
    assert_eq!(Runtime::block_on(build.wait()), Some(0), "{log}");
    let launch = Runtime::block_on(backend.launch_config(&project)).unwrap();
    assert!(launch.args[0].ends_with("Shop.Cli.dll"), "{launch:?}");

    let calc = backend.local(&format!("{}/src/Shop.Core/PriceCalculator.cs", root.display())).unwrap();
    let line = std::fs::read_to_string(&calc).unwrap().lines().position(|l| l.contains("Breakpoint here")).unwrap() as u32 + 1;
    let mut breakpoints = Breakpoints::new();
    breakpoints.set(BreakpointSpec::new(&calc, line));
    let launcher_backend = backend.clone();
    let launcher: rc_dap::session::Launcher = Arc::new(move |out, waker| {
        let map = rc_dap::remote::PathMap { mirror: launcher_backend.mirror.clone() };
        rc_dap::DapClient::connect(launcher_backend.dap_transport(exe.clone()), Some(Arc::new(map)), out, waker)
    });
    let adapter = rc_dap::AdapterCommand::new("netcoredbg", rc_dap::netcoredbg::Origin::Path);
    let mut session = DebugSession::start_with(launcher, adapter, DebugTarget::Launch(launch), DebugOptions::default(), breakpoints, rc_core::Waker::noop());
    let start = Instant::now();
    let mut events = Vec::new();
    while !(session.state().is_stopped() || session.state().is_ended()) {
        for event in session.pump() {
            if let DebugEvent::Failed(f) = &event {
                panic!("session failed: {f}");
            }
            events.push(event);
        }
        assert!(start.elapsed() < Duration::from_secs(120), "no stop; events {events:#?}");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(session.state().is_stopped(), "ended instead of stopping: {events:#?}");
    let info = session.stop_info().unwrap().clone();
    // The stop is reported on the mirror file, at the breakpoint's line.
    let start = Instant::now();
    while session.frames(info.thread_id).is_none() {
        session.pump();
        assert!(start.elapsed() < Duration::from_secs(20), "no frames");
        std::thread::sleep(Duration::from_millis(10));
    }
    let frame = session.frames(info.thread_id).unwrap().frames[0].clone();
    assert_eq!((frame.path().unwrap(), frame.line), (calc.as_path(), line));
    assert!(session.breakpoints().get(&calc, line).unwrap().verified);
    // Its locals.
    let frame_id = info.frame_id.unwrap();
    session.request_scopes(frame_id);
    let start = Instant::now();
    while session.scopes(frame_id).is_none() {
        session.pump();
        assert!(start.elapsed() < Duration::from_secs(20), "no scopes");
        std::thread::sleep(Duration::from_millis(10));
    }
    let reference = session.scopes(frame_id).unwrap()[0].variables_reference;
    session.request_variables(reference);
    let start = Instant::now();
    while session.variables(reference).is_none() {
        session.pump();
        assert!(start.elapsed() < Duration::from_secs(20), "no variables");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(session.variables(reference).unwrap().iter().any(|v| v.name == "subtotal"));
    session.stop();
    let start = Instant::now();
    while !session.state().is_ended() && start.elapsed() < Duration::from_secs(10) {
        session.pump();
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// The whole remote folder through the app: it opens over an agent served in this process,
/// is copied into the mirror, C# support starts on the "remote machine", a file opens in an
/// editor tab, its save reaches the remote folder, and going to a declaration in a file not
/// open yet fetches it.
#[test]
fn a_remote_folder_opens_edits_saves_and_navigates_in_the_app() {
    let _lock = crate::screens::HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let remote = tempfile::tempdir().unwrap();
    let root = dunce::canonicalize(remote.path()).unwrap();
    write(&root.join("Lib/Lib.csproj"), "<Project Sdk=\"Microsoft.NET.Sdk\">\n  <PropertyGroup><TargetFramework>net8.0</TargetFramework></PropertyGroup>\n</Project>\n");
    write(&root.join("Lib/Greeter.cs"), "namespace Lib;\n\npublic class Greeter\n{\n    public string Hello(string name) => $\"Hello {name}\";\n}\n");
    write(&root.join("App/App.csproj"), "<Project Sdk=\"Microsoft.NET.Sdk\">\n  <PropertyGroup><OutputType>Exe</OutputType><TargetFramework>net8.0</TargetFramework></PropertyGroup>\n  <ItemGroup><ProjectReference Include=\"..\\Lib\\Lib.csproj\" /></ItemGroup>\n</Project>\n");
    write(&root.join("App/Program.cs"), "using Lib;\n\nvar greeter = new Greeter();\nSystem.Console.WriteLine(greeter.Hello(\"world\"));\n");
    write(&root.join("Demo.slnx"), "<Solution>\n  <Project Path=\"App/App.csproj\" />\n  <Project Path=\"Lib/Lib.csproj\" />\n</Solution>\n");
    assert!(std::process::Command::new("dotnet").args(["restore", root.join("Demo.slnx").to_str().unwrap()]).output().unwrap().status.success());

    let home = tempfile::tempdir().unwrap();
    let mirror = tempfile::tempdir().unwrap();
    let mirror_root = dunce::canonicalize(mirror.path()).unwrap();
    let mut app = crate::screens::test_app(home.path(), true);
    app.remote.tools = crate::remote_backend::test_support::test_tools();
    app.remote.mirror_root = Some(mirror_root.clone());
    let dotnet = rc_dotnet::Dotnet::locate(None).map(|d| d.path.display().to_string()).unwrap_or_else(|| "dotnet".into());
    let connection = rc_core::settings::RemoteConnection { host: "test".into(), folder: root.display().to_string(), dotnet, ..Default::default() };
    app.remote.connect_local_for_test(connection);
    let local = |rel: &str| mirror_root.join(root.strip_prefix("/").unwrap()).join(rel);

    fn pump(app: &mut crate::app::DotCraftApp, what: &str, done: impl Fn(&crate::app::DotCraftApp) -> bool) {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(180) {
            for output in app.remote.pump() {
                app.handle_remote(output);
            }
            let out = app.insight.update(&mut app.docs, true, false);
            app.handle_insight(out);
            app.docs.poll_parses();
            if let InsightStatus::Failed(why) = &app.insight.status {
                panic!("server failed: {why}");
            }
            if done(app) {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("timed out waiting for {what}; insight {:?}", app.insight.status);
    }
    pump(&mut app, "the folder and C# support", |a| a.remote.is_open() && a.insight.is_remote() && a.insight.status == InsightStatus::Ready);
    assert!(app.remote.language_started_for_test());
    // The whole folder was copied, without opening anything.
    assert!(local("Lib/Greeter.cs").is_file());

    app.remote.open(&format!("{}/App/Program.cs", root.display()), 0, 0);
    pump(&mut app, "Program.cs in a tab", |a| a.docs.active_doc().is_some_and(|d| d.path == local("App/Program.cs")));

    // Edit and save: the remote file follows.
    let doc = app.docs.active_doc_mut().unwrap();
    doc.editor.replace(0, 0, "// edited\n");
    app.run_ide_action(&egui::Context::default(), rc_editor::Action::Save);
    pump(&mut app, "the save on the remote machine", |_| std::fs::read_to_string(root.join("App/Program.cs")).unwrap().starts_with("// edited\n"));

    // Go to declaration: Greeter.cs is fetched and opens at its line.
    std::fs::remove_file(local("Lib/Greeter.cs")).unwrap();
    let i = app.docs.active.unwrap();
    let pos = app.docs.list[i].editor.buffer.rope().to_string().find("greeter.Hello(").unwrap() + "greeter.".len() + 1;
    app.insight.navigate(&app.docs.list[i], pos, NavKind::Declaration);
    pump(&mut app, "Greeter.cs", |a| a.docs.active_doc().is_some_and(|d| d.path == local("Lib/Greeter.cs")));
    assert_eq!(app.docs.active_doc().unwrap().editor.caret_line_col().0, 4);
    app.close_workspace();
    assert!(!app.remote.is_open());
}

/// Debug from the remote Explorer: the project builds on the "remote machine" (output in a
/// console tab), netcoredbg runs there, and the Debug window stops at a breakpoint set in
/// the editor on the file's local copy.
#[test]
fn the_explorer_debugs_a_remote_project_into_the_debug_window() {
    let _lock = crate::screens::HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let remote = tempfile::tempdir().unwrap();
    let root = dunce::canonicalize(remote.path()).unwrap();
    copy_tree(&crate::screens::fixtures().join("Shop"), &root);
    let home = tempfile::tempdir().unwrap();
    let mirror = tempfile::tempdir().unwrap();
    let mirror_root = dunce::canonicalize(mirror.path()).unwrap();
    let mut app = crate::screens::test_app(home.path(), true);
    app.remote.tools = crate::remote_backend::test_support::test_tools();
    app.remote.mirror_root = Some(mirror_root.clone());
    let dotnet = rc_dotnet::Dotnet::locate(None).map(|d| d.path.display().to_string()).unwrap_or_else(|| "dotnet".into());
    app.remote.connect_local_for_test(rc_core::settings::RemoteConnection { host: "test".into(), folder: root.display().to_string(), dotnet, ..Default::default() });
    let local = |rel: &str| mirror_root.join(root.strip_prefix("/").unwrap()).join(rel);

    fn pump(app: &mut crate::app::DotCraftApp, what: &str, done: impl Fn(&crate::app::DotCraftApp) -> bool) {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(240) {
            for output in app.remote.pump() {
                app.handle_remote(output);
            }
            let outs = app.debugger.update(&mut app.docs);
            app.handle_debug(outs);
            if done(app) {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("timed out waiting for {what}; debugger {:?}", app.debugger.session.as_ref().map(|s| s.state().clone()));
    }
    pump(&mut app, "the folder copy", |a| a.remote.is_open() && a.remote.mirror().is_some() && local("src/Shop.Core/PriceCalculator.cs").is_file());
    let calc = local("src/Shop.Core/PriceCalculator.cs");
    let line = std::fs::read_to_string(&calc).unwrap().lines().position(|l| l.contains("Breakpoint here")).unwrap();
    app.remote.open(&format!("{}/src/Shop.Core/PriceCalculator.cs", root.display()), line, 0);
    pump(&mut app, "PriceCalculator.cs", |a| a.docs.active_doc().is_some_and(|d| d.path == calc));
    app.docs.active_doc_mut().unwrap().editor.toggle_breakpoint(line);

    app.remote.debug_for_test(&format!("{}/src/Shop.Cli/Shop.Cli.csproj", root.display()));
    pump(&mut app, "the stop", |a| a.debugger.session.as_ref().is_some_and(|s| s.state().is_stopped() || s.state().is_ended()));
    let session = app.debugger.session.as_ref().unwrap();
    assert!(session.state().is_stopped(), "{:?}", session.state());
    assert_eq!(session.execution_point(), Some((calc.clone(), line as u32 + 1)));
    // The build's output went to a console tab.
    assert!(app.terminals.titles().iter().any(|t| t.contains("Shop.Cli")), "{:?}", app.terminals.titles());
    app.debugger.stop();
    app.close_workspace();
}
