//! Phase 4 closure from the app, against the real .NET SDK and netcoredbg (downloaded once into
//! `target/test-home`): debugging Shop.Cli with a conditional breakpoint, variables, watches,
//! evaluation, steps and Run to Cursor; and Shop.Api stopping on a breakpoint in a controller
//! during an HTTP request.

use crate::app::DotCraftApp;
use rc_core::{Settings, Waker};
use rc_editor::BreakpointKind;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

fn test_home() -> PathBuf {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/test-home");
    std::fs::create_dir_all(&p).unwrap();
    p.canonicalize().unwrap()
}

/// netcoredbg, downloaded once into `target/test-home` (kept between runs).
fn netcoredbg() -> PathBuf {
    // SAFETY: serialized by HOME_LOCK.
    unsafe { std::env::set_var("DOTCRAFT_HOME", test_home()) };
    if let Some(cmd) = rc_dap::netcoredbg::locate(None).filter(|c| c.origin != rc_dap::netcoredbg::Origin::Path) {
        return cmd.program;
    }
    let inbox = rc_core::Inbox::new(Waker::noop());
    rc_core::Runtime::block_on(rc_dap::netcoredbg::download(inbox.sender()));
    for ev in inbox.drain() {
        if let rc_dap::DownloadEvent::Failed(e) = ev {
            panic!("netcoredbg download failed: {e}");
        }
    }
    rc_dap::netcoredbg::locate(None).expect("downloaded netcoredbg").program
}

/// An app with the Shop solution open, its own (empty) data folder and netcoredbg configured.
fn app(home: &Path) -> DotCraftApp {
    let netcoredbg = netcoredbg();
    // SAFETY: serialized by HOME_LOCK.
    unsafe { std::env::set_var("DOTCRAFT_HOME", home) };
    let settings = Settings { roslyn_path: Some("/nonexistent".into()), netcoredbg_path: Some(netcoredbg), ..Settings::default() };
    let mut app = DotCraftApp::with_settings(settings, Waker::noop(), None);
    app.open_workspace(&crate::screens::fixtures().join("Shop/Shop.sln"));
    app
}

fn pump(app: &mut DotCraftApp, what: &str, timeout: Duration, mut done: impl FnMut(&mut DotCraftApp) -> bool) {
    let start = Instant::now();
    while start.elapsed() < timeout {
        let outs = app.debugger.update(&mut app.docs);
        app.handle_debug(outs);
        if done(app) {
            return;
        }
        if let Some(rc_dap::SessionState::Failed(f)) = app.debugger.session.as_ref().map(|s| s.state()) {
            panic!("debugging failed waiting for {what}: {f}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("timed out waiting for {what} (state: {})", app.debugger.state_label());
}

fn until_stopped(app: &mut DotCraftApp, what: &str) {
    pump(app, what, Duration::from_secs(300), |a| a.debugger.stopped() || a.debugger.session.as_ref().is_some_and(|s| s.state().is_ended()));
    assert!(app.debugger.stopped(), "not stopped at {what}: {}", app.debugger.state_label());
}

/// 0-based line of `needle` in `file`.
fn line_of(file: &Path, needle: &str) -> usize {
    std::fs::read_to_string(file).unwrap().lines().position(|l| l.contains(needle)).unwrap_or_else(|| panic!("{needle} not in {}", file.display()))
}

fn locals(app: &mut DotCraftApp) -> Vec<rc_dap::Variable> {
    let frame = app.debugger.session.as_ref().unwrap().selected_frame().unwrap().id;
    pump(app, "scopes", Duration::from_secs(30), |a| {
        let s = a.debugger.session.as_mut().unwrap();
        match s.scopes(frame).and_then(|sc| sc.first()).map(|sc| sc.variables_reference) {
            Some(r) => match s.variables(r) {
                Some(_) => true,
                None => {
                    s.request_variables(r);
                    false
                }
            },
            None => {
                s.request_scopes(frame);
                false
            }
        }
    });
    let s = app.debugger.session.as_ref().unwrap();
    s.variables(s.scopes(frame).unwrap()[0].variables_reference).unwrap().to_vec()
}

fn value(vars: &[rc_dap::Variable], name: &str) -> String {
    vars.iter().find(|v| v.name == name).unwrap_or_else(|| panic!("no {name} in {vars:?}")).value.clone()
}

fn select(app: &mut DotCraftApp, prefix: &str) -> crate::runner::RunConfig {
    let i = app.runner.configs.iter().position(|c| c.name.starts_with(prefix)).unwrap();
    app.runner.selected = i;
    app.runner.configs[i].clone()
}

#[test]
fn debugs_the_cli_and_the_api_from_the_ide() {
    let _lock = crate::screens::HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = tempfile::tempdir().unwrap();
    let mut app = app(home.path());
    let ctx = egui::Context::default();
    let calc = crate::screens::fixtures().join("Shop/src/Shop.Core/PriceCalculator.cs");
    let loop_line = line_of(&calc, "Breakpoint here");
    let discount_line = line_of(&calc, "var discount = DiscountFor(order, subtotal);");
    let tax_line = line_of(&calc, "var tax = Math.Round");

    // A conditional breakpoint in the loop (the second line, the mouse) and a plain one after it.
    app.open_file(&calc);
    let d = app.docs.active_doc_mut().unwrap();
    d.editor.breakpoints.insert(loop_line, BreakpointKind::Conditional);
    d.editor.breakpoints.insert(discount_line, BreakpointKind::Normal);
    app.debugger.conditions.insert((calc.clone(), loop_line), "line.Sku == \"MS-02\"".into());
    app.debugger.session = None;

    select(&mut app, "Shop.Cli");
    app.run_ide_action(&ctx, rc_editor::Action::Debug);
    // netcoredbg may be downloaded first: Debug then starts by itself.
    until_stopped(&mut app, "the conditional breakpoint");
    assert_eq!(app.dock.bottom, Some(crate::toolwindow::ToolWindow::Debug));
    let doc = app.docs.active_doc().unwrap();
    assert_eq!(doc.path, calc);
    assert_eq!(doc.editor.execution_line, Some(loop_line));
    assert_eq!(doc.editor.caret_line_col().0, loop_line);
    let vars = locals(&mut app);
    assert_eq!(value(&vars, "subtotal"), "30.00", "{vars:?}");

    // Watches and Evaluate.
    app.debugger.session.as_mut().unwrap().add_watch("subtotal + 1");
    app.debugger.evaluate("line.Quantity * 2".into());
    pump(&mut app, "the evaluation and the watch", Duration::from_secs(30), |a| {
        let w = &a.debugger.session.as_ref().unwrap().watches()[0];
        matches!(a.debugger.eval, Some((_, _, Some(_)))) && matches!(w.value, rc_dap::WatchValue::Value(_))
    });
    match &app.debugger.eval {
        Some((_, _, Some(Ok(v)))) => assert_eq!(v.result, "4"),
        other => panic!("{other:?}"),
    }
    match &app.debugger.session.as_ref().unwrap().watches()[0].value {
        rc_dap::WatchValue::Value(v) => assert_eq!(v.result, "31.00"),
        other => panic!("{other:?}"),
    }
    // Hover values: the expression under the mouse.
    let rope = app.docs.active_doc().unwrap().editor.buffer.rope().clone();
    let pos = rope.line_to_char(loop_line) + rope.line(loop_line).to_string().find("Subtotal").unwrap() + 2;
    assert_eq!(crate::debugger::expression_at(&rope, pos), "line.Subtotal");
    let id = app.docs.active_doc().unwrap().editor.id;
    app.debugger.hover(id, pos, egui::Pos2::ZERO, "line.Subtotal".into());
    pump(&mut app, "the hover value", Duration::from_secs(30), |a| a.debugger.hover_result.is_some());
    assert!(app.debugger.hover_result.as_ref().unwrap().3.starts_with("line.Subtotal = 25.00"), "{:?}", app.debugger.hover_result);

    // Continue to the plain breakpoint: the loop condition is not met again.
    app.run_ide_action(&ctx, rc_editor::Action::Resume);
    until_stopped(&mut app, "the discount breakpoint");
    assert_eq!(app.docs.active_doc().unwrap().editor.execution_line, Some(discount_line));
    assert_eq!(value(&locals(&mut app), "subtotal"), "55.00");
    // Breakpoints were bound by the debugger.
    assert_eq!(app.docs.active_doc().unwrap().editor.breakpoints.get(&discount_line), Some(&BreakpointKind::Verified));

    // Step over, then Run to Cursor.
    app.run_ide_action(&ctx, rc_editor::Action::StepOver);
    until_stopped(&mut app, "the step");
    assert_eq!(app.docs.active_doc().unwrap().editor.execution_line, Some(discount_line + 1));
    app.docs.active_doc_mut().unwrap().editor.goto(tax_line + 1, 0);
    app.run_ide_action(&ctx, rc_editor::Action::RunToCursor);
    until_stopped(&mut app, "the cursor");
    assert_eq!(app.docs.active_doc().unwrap().editor.execution_line, Some(tax_line + 1), "{:?}", app.debugger.session.as_ref().unwrap().stop_info());
    let vars = locals(&mut app);
    assert_eq!(value(&vars, "tax"), "11.55", "{vars:?}");
    // The one-shot breakpoint is gone.
    assert!(app.debugger.session.as_ref().unwrap().breakpoints().get(&calc, tax_line as u32 + 2).is_none());

    // Toggling a breakpoint off in the editor removes it from the session.
    app.docs.active_doc_mut().unwrap().editor.breakpoints.remove(&loop_line);
    pump(&mut app, "the breakpoint removal", Duration::from_secs(10), |a| a.debugger.session.as_ref().unwrap().breakpoints().get(&calc, loop_line as u32 + 1).is_none());

    app.run_ide_action(&ctx, rc_editor::Action::Resume);
    pump(&mut app, "the end", Duration::from_secs(60), |a| a.debugger.session.as_ref().is_some_and(|s| s.state().is_ended()));
    assert_eq!(app.debugger.session.as_ref().unwrap().state(), &rc_dap::SessionState::Exited(0));
    let console = app.debugger.console.as_ref().unwrap().term.screen_lines().join("\n");
    assert!(console.contains("Total"), "console: {console}");
    assert!(app.docs.list.iter().all(|d| d.editor.execution_line.is_none()));
    assert_eq!(app.docs.active_doc().unwrap().editor.breakpoints.get(&discount_line), Some(&BreakpointKind::Normal));

    // ASP.NET: a breakpoint in a controller, hit by an HTTP request (the controller calls the
    // calculator, so its breakpoint goes first).
    app.docs.active_doc_mut().unwrap().editor.breakpoints.clear();
    let controller = crate::screens::fixtures().join("Shop/src/Shop.Api/Controllers/OrdersController.cs");
    let line = line_of(&controller, "var order = Order.Sample();");
    app.open_file(&controller);
    app.docs.active_doc_mut().unwrap().editor.breakpoints.insert(line, BreakpointKind::Normal);
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    app.debugger.env = vec![("ASPNETCORE_URLS".into(), format!("http://127.0.0.1:{port}"))];
    select(&mut app, "Shop.Api");
    app.run_ide_action(&ctx, rc_editor::Action::Debug);
    pump(&mut app, "the API to listen", Duration::from_secs(300), |a| {
        a.debugger.session.as_ref().is_some_and(|s| s.state() == &rc_dap::SessionState::Running) && std::net::TcpStream::connect(("127.0.0.1", port)).is_ok()
    });
    assert_eq!(app.debugger.urls, vec![format!("http://127.0.0.1:{port}")]);
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(120))).unwrap();
        write!(s, "GET /orders/sample?coupon=SAVE10 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n").unwrap();
        let mut out = String::new();
        let _ = s.read_to_string(&mut out);
        tx.send(out).unwrap();
    });
    until_stopped(&mut app, "the controller breakpoint");
    assert_eq!(app.docs.active_doc().unwrap().path, controller);
    assert_eq!(app.docs.active_doc().unwrap().editor.execution_line, Some(line));
    assert_eq!(value(&locals(&mut app), "coupon"), "\"SAVE10\"");
    app.run_ide_action(&ctx, rc_editor::Action::Resume);
    let mut response = None;
    pump(&mut app, "the HTTP response", Duration::from_secs(60), |_| {
        response = rx.try_recv().ok();
        response.is_some()
    });
    let response = response.unwrap();
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    app.run_ide_action(&ctx, rc_editor::Action::StopRun);
    pump(&mut app, "the session to end", Duration::from_secs(30), |a| a.debugger.session.as_ref().is_some_and(|s| s.state().is_ended()));
    let deadline = Instant::now() + Duration::from_secs(15);
    while std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(std::net::TcpStream::connect(("127.0.0.1", port)).is_err(), "the API still listens");
}

#[test]
#[ignore = "Requires a GPU; writes window captures to target/snapshots"]
fn debug_screens() {
    let _lock = crate::screens::HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/snapshots");
    std::fs::create_dir_all(&out).unwrap();
    let calc = crate::screens::fixtures().join("Shop/src/Shop.Core/PriceCalculator.cs");
    let loop_line = line_of(&calc, "Breakpoint here");
    let discount_line = line_of(&calc, "var discount = DiscountFor(order, subtotal);");
    for dark in [true, false] {
        let home = tempfile::tempdir().unwrap();
        let mut app = app(home.path());
        app.settings.theme = if dark { rc_core::settings::ThemeChoice::Dark } else { rc_core::settings::ThemeChoice::Light };
        app.open_file(&calc);
        let d = app.docs.active_doc_mut().unwrap();
        d.editor.breakpoints.insert(loop_line, BreakpointKind::Conditional);
        d.editor.breakpoints.insert(discount_line, BreakpointKind::Normal);
        d.editor.breakpoints.insert(discount_line + 2, BreakpointKind::Disabled);
        app.debugger.conditions.insert((calc.clone(), loop_line), "subtotal > 0".into());
        select(&mut app, "Shop.Cli");
        app.debugger.session = None;
        app.pending.push(rc_editor::Action::Debug);
        let mut h = crate::screens::harness(app, egui::vec2(1440.0, 900.0));
        let start = Instant::now();
        while !h.state().debugger.stopped() {
            assert!(start.elapsed() < Duration::from_secs(300), "not stopped: {}", h.state().debugger.state_label());
            h.run_steps(1);
            std::thread::sleep(Duration::from_millis(30));
        }
        let app = h.state_mut();
        app.debugger.session.as_mut().unwrap().add_watch("subtotal + line.Subtotal");
        app.debugger.expanded.insert("s:Locals/order".into());
        for _ in 0..60 {
            h.run_steps(1);
            std::thread::sleep(Duration::from_millis(30));
        }
        let suffix = if dark { "dark" } else { "light" };
        h.render().unwrap().save(out.join(format!("debug-{suffix}.png"))).unwrap();
        // The console tab.
        h.state_mut().debugger.show_console = true;
        h.run_steps(3);
        h.render().unwrap().save(out.join(format!("debug-console-{suffix}.png"))).unwrap();
        // Evaluate dialog with an expanded object.
        let app = h.state_mut();
        app.debugger.show_console = false;
        app.debugger.evaluate("order".into());
        app.debugger.expanded.insert("e:order".into());
        app.dialog = Some(crate::ui::dialogs::Dialog::Evaluate { expression: "order".into(), focus: true });
        for _ in 0..40 {
            h.run_steps(1);
            std::thread::sleep(Duration::from_millis(30));
        }
        h.render().unwrap().save(out.join(format!("debug-evaluate-{suffix}.png"))).unwrap();
        h.state_mut().dialog = None;
        h.state_mut().debugger.stop();
        for _ in 0..30 {
            h.run_steps(1);
            std::thread::sleep(Duration::from_millis(30));
        }
    }
}

#[test]
fn breakpoint_conditions_and_disabled_state_survive_the_session() {
    let _lock = crate::screens::HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = tempfile::tempdir().unwrap();
    let calc = crate::screens::fixtures().join("Shop/src/Shop.Core/PriceCalculator.cs");
    let mut app = crate::screens::test_app(home.path(), true);
    app.settings.roslyn_path = Some("/nonexistent".into());
    app.open_workspace(&crate::screens::fixtures().join("Shop/Shop.sln"));
    app.open_file(&calc);
    let d = app.docs.active_doc_mut().unwrap();
    d.editor.breakpoints.insert(20, BreakpointKind::Conditional);
    d.editor.breakpoints.insert(26, BreakpointKind::Disabled);
    app.debugger.conditions.insert((calc.clone(), 20), "subtotal > 0".into());
    app.save_session();

    let mut app = crate::screens::test_app(home.path(), true);
    app.settings.roslyn_path = Some("/nonexistent".into());
    app.open_workspace(&crate::screens::fixtures().join("Shop/Shop.sln"));
    let d = app.docs.list.iter().find(|d| d.path == calc).unwrap();
    assert_eq!(d.editor.breakpoints.get(&20), Some(&BreakpointKind::Conditional));
    assert_eq!(d.editor.breakpoints.get(&26), Some(&BreakpointKind::Disabled));
    assert_eq!(app.debugger.conditions.get(&(calc.clone(), 20)).map(String::as_str), Some("subtotal > 0"));
}
