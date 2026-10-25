//! Phase 4 closure against the real .NET SDK and netcoredbg, with `tests/fixtures/Shop`:
//! breakpoints, stack, variables, watches, evaluation, conditional breakpoints, stepping and
//! output on `Shop.Cli`; a breakpoint hit by an HTTP request on `Shop.Api` and a clean stop;
//! exception stops with `exceptionInfo`; attaching to a running process.
//!
//! netcoredbg is downloaded once into `target/test-home` (or set `DOTCRAFT_NETCOREDBG`).
//! The tests build inside the fixture, so they run one at a time.

use rc_core::Waker;
use rc_dap::netcoredbg::{self, AdapterCommand};
use rc_dap::{BreakpointSpec, Breakpoints, DebugEvent, DebugOptions, DebugSession, DebugTarget, EvalContext, OutputCategory, SessionState, StopReason, WatchValue};
use rc_dotnet::{Dotnet, RunRequest};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

static SERIAL: Mutex<()> = Mutex::new(());

fn test_home() -> PathBuf {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/test-home");
    std::fs::create_dir_all(&p).unwrap();
    p.canonicalize().unwrap()
}

fn adapter() -> AdapterCommand {
    // SAFETY: set before any thread of this test binary reads it (under SERIAL).
    unsafe { std::env::set_var("DOTCRAFT_HOME", test_home()) };
    let configured = std::env::var_os("DOTCRAFT_NETCOREDBG").map(PathBuf::from);
    if let Some(cmd) = netcoredbg::locate(configured.as_deref()).filter(|c| c.origin != netcoredbg::Origin::Path) {
        return cmd;
    }
    let inbox = rc_core::Inbox::new(Waker::noop());
    rc_core::Runtime::block_on(netcoredbg::download(inbox.sender()));
    let events = inbox.drain();
    assert!(events.iter().any(|e| matches!(e, netcoredbg::DownloadEvent::Progress { .. })));
    for ev in &events {
        if let netcoredbg::DownloadEvent::Failed(e) = ev {
            panic!("download failed: {e}");
        }
    }
    let cmd = netcoredbg::locate(None).expect("downloaded netcoredbg");
    assert_eq!(cmd.origin, netcoredbg::Origin::Downloaded);
    cmd
}

fn dotnet() -> Dotnet {
    Dotnet::locate(None).expect("dotnet SDK")
}

fn shop() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/Shop").canonicalize().unwrap()
}

/// 1-based line of the first line containing `needle`.
fn line_of(file: &Path, needle: &str) -> u32 {
    let text = std::fs::read_to_string(file).unwrap();
    text.lines().position(|l| l.contains(needle)).unwrap_or_else(|| panic!("{needle:?} not in {}", file.display())) as u32 + 1
}

struct Run {
    s: DebugSession,
    events: Vec<DebugEvent>,
}

impl Run {
    fn start(target: DebugTarget, options: DebugOptions, bps: Breakpoints) -> Run {
        Run { s: DebugSession::start(adapter(), target, options, bps, Waker::noop()), events: Vec::new() }
    }

    fn pump(&mut self) {
        for e in self.s.pump() {
            if let DebugEvent::Failed(f) = &e {
                panic!("session failed: {f}\nevents: {:#?}", self.events);
            }
            self.events.push(e);
        }
    }

    /// Pumps until the session satisfies `done`.
    fn until(&mut self, what: &str, timeout: Duration, mut done: impl FnMut(&DebugSession) -> bool) {
        let start = Instant::now();
        loop {
            self.pump();
            if done(&self.s) {
                return;
            }
            if start.elapsed() > timeout {
                let tail: Vec<_> = self.events.iter().rev().take(30).collect();
                panic!("timed out waiting for {what}; state {:?}; last events {tail:#?}", self.s.state());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn until_stopped(&mut self) {
        self.until("a stop", Duration::from_secs(180), |s| s.state().is_stopped() || s.state().is_ended());
        assert!(self.s.state().is_stopped(), "ended instead of stopping: {:?}", self.s.state());
    }

    /// Top frame location of the stop.
    fn location(&self) -> (String, PathBuf, u32) {
        let info = self.s.stop_info().expect("stopped");
        let f = &self.s.frames(info.thread_id).expect("frames").frames[0];
        (f.name.clone(), f.path().map(Path::to_path_buf).unwrap_or_default(), f.line)
    }

    fn locals(&mut self) -> Vec<rc_dap::Variable> {
        let frame = self.s.stop_info().unwrap().frame_id.unwrap();
        self.s.request_scopes(frame);
        self.until("scopes", Duration::from_secs(20), |s| s.scopes(frame).is_some());
        let reference = self.s.scopes(frame).unwrap()[0].variables_reference;
        self.children(reference)
    }

    fn children(&mut self, reference: i64) -> Vec<rc_dap::Variable> {
        self.s.request_variables(reference);
        self.until("variables", Duration::from_secs(20), |s| s.variables(reference).is_some());
        self.s.variables(reference).unwrap().to_vec()
    }

    fn eval(&mut self, expr: &str) -> rc_dap::EvalValue {
        let id = self.s.evaluate(expr, EvalContext::Repl);
        let start = self.events.len();
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if let Some(DebugEvent::Evaluated { result, .. }) = self.events[start..].iter().find(|e| matches!(e, DebugEvent::Evaluated { id: got, .. } if *got == id)) {
                return result.clone().unwrap_or_else(|e| panic!("evaluate {expr}: {e}"));
            }
            assert!(Instant::now() < deadline, "no answer to evaluate {expr}");
            std::thread::sleep(Duration::from_millis(10));
            self.pump();
        }
    }

    fn output(&self, category: OutputCategory) -> String {
        self.events.iter().filter_map(|e| if let DebugEvent::Output { category: c, text } = e { (*c == category).then_some(text.as_str()) } else { None }).collect()
    }

    fn until_ended(&mut self) {
        self.until("the end", Duration::from_secs(60), |s| s.state().is_ended());
    }
}

fn project(name: &str) -> PathBuf {
    shop().join(format!("src/{name}/{name}.csproj"))
}

#[test]
fn debugs_the_cli_breakpoints_stack_variables_watches_steps_and_output() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let calc = shop().join("src/Shop.Core/PriceCalculator.cs");
    let program = shop().join("src/Shop.Cli/Program.cs");
    let loop_line = line_of(&calc, "Breakpoint here");
    let discount_line = line_of(&calc, "var discount = DiscountFor(order, subtotal);");
    let taxable_line = line_of(&calc, "var taxable = subtotal - discount;");
    let switch_line = line_of(&calc, "switch (order.CouponCode");
    let call_line = line_of(&program, "new PriceCalculator().Calculate(order)");

    // Conditional from the start: the first iteration (subtotal 0) must not stop.
    let mut bps = Breakpoints::new();
    bps.set(BreakpointSpec::new(&calc, loop_line).with_condition("subtotal > 0"));
    let mut r = Run::start(DebugTarget::Project { dotnet: dotnet(), run: RunRequest::new(project("Shop.Cli")) }, DebugOptions::default(), bps);
    let watch = r.s.add_watch("subtotal + 1");
    r.until_stopped();
    assert!(r.events.iter().any(|e| matches!(e, DebugEvent::Prepared(_))));
    assert!(r.events.iter().any(|e| matches!(e, DebugEvent::Build(rc_dotnet::BuildEvent::Finished(b)) if b.success)));

    // Verified at the marked line, hit there.
    let b = r.s.breakpoints().get(&calc, loop_line).unwrap();
    assert!(b.verified, "breakpoint not verified: {b:?}");
    assert_eq!(b.display_line(), loop_line);
    let info = r.s.stop_info().unwrap().clone();
    assert_eq!(info.reason, StopReason::Breakpoint);
    assert_eq!(info.breakpoints, vec![(calc.clone(), loop_line)]);

    // Stack with files and lines.
    let frames = r.s.frames(info.thread_id).unwrap().frames.clone();
    assert!(frames[0].name.contains("PriceCalculator.Calculate"), "{:?}", frames[0].name);
    assert_eq!((frames[0].path().unwrap(), frames[0].line), (calc.as_path(), loop_line));
    assert_eq!((frames[1].path().unwrap(), frames[1].line), (program.as_path(), call_line));
    assert_eq!(r.s.execution_point(), Some((calc.clone(), loop_line)));

    // Locals and an object's fields.
    let locals = r.locals();
    let value = |vars: &[rc_dap::Variable], name: &str| vars.iter().find(|v| v.name == name).unwrap_or_else(|| panic!("no {name} in {vars:?}")).clone();
    assert_eq!(value(&locals, "subtotal").value, "30.00", "the condition held only in the second iteration");
    let order = value(&locals, "order");
    assert!(order.variables_reference > 0);
    let fields = r.children(order.variables_reference);
    assert_eq!(value(&fields, "Customer").value, "\"Ada\"");
    assert!(value(&fields, "_lines").variables_reference > 0);

    // Watch (evaluated on the stop) and evaluate.
    r.until("the watch", Duration::from_secs(20), |s| matches!(s.watches()[0].value, WatchValue::Value(_) | WatchValue::Error(_)));
    let WatchValue::Value(v) = &r.s.watches()[0].value else { panic!("{:?}", r.s.watches()) };
    assert_eq!(v.result, "31.00");
    assert_eq!(r.s.watches()[0].id, watch);
    assert_eq!(r.eval("order.ItemCount").result, "3");
    assert_eq!(r.eval("line.Sku").result, "\"MS-02\"");
    let repl = r.eval("order");
    assert!(repl.variables_reference > 0);
    assert!(r.children(repl.variables_reference).iter().any(|v| v.name == "CouponCode"));

    // Step over stays in the loop; step into DiscountFor; step out back to the caller.
    r.s.remove_breakpoint(&calc, loop_line);
    r.s.step_over();
    r.until_stopped();
    let (name, path, line) = r.location();
    assert!(name.contains("Calculate") && path == calc, "{name} {path:?}");
    // The loop body's closing brace.
    assert_eq!(line, loop_line + 1, "step over");

    r.s.set_breakpoint(BreakpointSpec::new(&calc, discount_line));
    r.s.set_breakpoint(BreakpointSpec::new(&calc, taxable_line).with_log_message("taxable for {order.Customer}"));
    r.s.continue_();
    r.until_stopped();
    assert_eq!(r.location().2, discount_line);
    r.s.step_in();
    r.until_stopped();
    let (name, _, line) = r.location();
    assert!(name.contains("DiscountFor"), "step in went to {name}");
    // The method's opening brace.
    assert_eq!(line, switch_line - 1, "step in");
    r.s.step_out();
    r.until_stopped();
    let (name, _, line) = r.location();
    assert!(name.contains("Calculate"), "step out went to {name}");
    assert_eq!(line, discount_line);

    // Run to the end: the logpoint prints without stopping, exit code 0, output captured.
    r.s.remove_breakpoint(&calc, discount_line);
    r.s.continue_();
    r.until_ended();
    assert_eq!(r.s.state(), &SessionState::Exited(0));
    let stdout = r.output(OutputCategory::Stdout);
    assert!(stdout.contains("Order for Ada (3 items)") && stdout.contains("Total: 66.55"), "stdout: {stdout}");
    assert!(r.output(OutputCategory::Stderr).contains("done"));
    assert!(r.output(OutputCategory::Console).contains("taxable for \"Ada\""), "console: {}", r.output(OutputCategory::Console));
}

fn port_open(port: u16) -> bool {
    TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), Duration::from_millis(200)).is_ok()
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

fn http_get(port: u16, path: &str) -> String {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(120))).unwrap();
    write!(s, "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n").unwrap();
    let mut out = String::new();
    let _ = s.read_to_string(&mut out);
    out
}

/// Whether a process is alive (zombies count as gone).
#[cfg(unix)]
fn alive(pid: u32) -> bool {
    let out = std::process::Command::new("ps").args(["-o", "stat=", "-p", &pid.to_string()]).output().unwrap();
    let stat = String::from_utf8_lossy(&out.stdout);
    let stat = stat.trim();
    !stat.is_empty() && !stat.starts_with('Z')
}

#[cfg(windows)]
fn alive(pid: u32) -> bool {
    let out = std::process::Command::new("tasklist").args(["/FI", &format!("PID eq {pid}"), "/NH"]).output().unwrap();
    String::from_utf8_lossy(&out.stdout).contains(&pid.to_string())
}

#[test]
fn debugs_the_api_through_an_http_request_and_stops_cleanly() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let controller = shop().join("src/Shop.Api/Controllers/OrdersController.cs");
    let line = line_of(&controller, "var order = Order.Sample();");
    let port = free_port();
    let mut run = RunRequest::new(project("Shop.Api"));
    run.env.push(("ASPNETCORE_URLS".into(), format!("http://127.0.0.1:{port}")));
    let mut bps = Breakpoints::new();
    bps.set(BreakpointSpec::new(&controller, line));
    let mut r = Run::start(DebugTarget::Project { dotnet: dotnet(), run }, DebugOptions::default(), bps);
    r.until("the app to listen", Duration::from_secs(180), |s| s.state().is_ended() || (s.state() == &SessionState::Running && port_open(port)));
    let plan = r.events.iter().find_map(|e| if let DebugEvent::Prepared(p) = e { Some(p.clone()) } else { None }).unwrap();
    assert_eq!(plan.urls, vec![format!("http://127.0.0.1:{port}")]);
    assert!(r.s.breakpoints().get(&controller, line).unwrap().verified);

    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || tx.send(http_get(port, "/orders/sample?coupon=SAVE10")).unwrap());
    r.until_stopped();
    let (name, path, at) = r.location();
    assert!(name.contains("OrdersController.Sample"), "{name}");
    assert_eq!((path, at), (controller.clone(), line));
    let locals = r.locals();
    let coupon = locals.iter().find(|v| v.name == "coupon").unwrap_or_else(|| panic!("{locals:?}"));
    assert_eq!(coupon.value, "\"SAVE10\"");

    r.s.continue_();
    let deadline = Instant::now() + Duration::from_secs(60);
    let response = loop {
        r.pump();
        if let Ok(resp) = rx.try_recv() {
            break resp;
        }
        assert!(Instant::now() < deadline, "no HTTP response");
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    assert!(response.contains("\"customer\":\"Ada\"") && response.contains("\"discount\":5.5"), "{response}");

    let adapter_pid = r.s.adapter_pid().unwrap();
    let debuggee = r.s.debuggee_pid().expect("process event");
    r.s.stop();
    r.pump();
    assert_eq!(r.s.state(), &SessionState::Terminated);
    let deadline = Instant::now() + Duration::from_secs(15);
    while (port_open(port) || alive(adapter_pid) || alive(debuggee)) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
        r.pump();
    }
    assert!(!port_open(port), "the app still listens");
    assert!(!alive(adapter_pid), "netcoredbg still runs");
    assert!(!alive(debuggee), "the app still runs");
}

#[test]
fn stops_on_exceptions_with_exception_info() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let order_cs = shop().join("src/Shop.Core/Order.cs");
    let throw_line = line_of(&order_cs, "throw new ArgumentException(\"A customer is required.\"");
    // A blank customer makes `new Order(" ")` throw, unhandled.
    let mut run = RunRequest::new(project("Shop.Cli"));
    run.args = vec![" ".into()];
    let options = DebugOptions { exception_filters: Some(vec!["all".into()]), ..Default::default() };
    let mut r = Run::start(DebugTarget::Project { dotnet: dotnet(), run }, options, Breakpoints::new());
    r.until_stopped();
    assert_eq!(r.s.stop_info().unwrap().reason, StopReason::Exception);
    let (name, path, line) = r.location();
    assert!(name.contains("Order"), "{name}");
    assert_eq!((path, line), (order_cs.clone(), throw_line));
    r.until("exceptionInfo", Duration::from_secs(20), |s| s.exception().is_some());
    let info = r.s.exception().unwrap().clone();
    assert!(info.exception_id.contains("ArgumentException"), "{info:?}");
    let details = info.details.clone().unwrap_or_default();
    let mentions = |s: &Option<String>| s.as_deref().is_some_and(|m| m.contains("A customer is required"));
    assert!(mentions(&details.message) || mentions(&info.description), "{info:?}");

    assert_eq!(info.break_mode, "always");

    // Continuing stops again on the unhandled exception, then the program dies with the
    // runtime's report on stderr.
    r.s.continue_();
    r.until_stopped();
    assert_eq!(r.s.stop_info().unwrap().reason, StopReason::Exception);
    r.until("exceptionInfo", Duration::from_secs(20), |s| s.exception().is_some());
    assert_eq!(r.s.exception().unwrap().break_mode, "unhandled");
    r.s.continue_();
    r.until_ended();
    // netcoredbg reports 0 for a debuggee killed by a signal (SIGABRT here) on macOS.
    assert!(matches!(r.s.state(), SessionState::Exited(_)), "{:?}", r.s.state());
    assert!(r.output(OutputCategory::Stderr).contains("Unhandled exception. System.ArgumentException"));

    // With the adapter's defaults (no filter) only the unhandled exception stops.
    let mut run = RunRequest::new(project("Shop.Cli"));
    run.args = vec![" ".into()];
    run.build_first = false;
    let mut r = Run::start(DebugTarget::Project { dotnet: dotnet(), run }, DebugOptions::default(), Breakpoints::new());
    r.until_stopped();
    r.until("exceptionInfo", Duration::from_secs(20), |s| s.exception().is_some());
    assert_eq!(r.s.exception().unwrap().break_mode, "unhandled");
    r.s.continue_();
    r.until_ended();
}

#[test]
fn attaches_to_a_running_api_and_detaches() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let dotnet = dotnet();
    let api = project("Shop.Api");
    let out = dotnet.std_command().args(["build", api.to_str().unwrap(), "-nologo", "-v:q"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stdout));
    let dll = shop().join("src/Shop.Api/bin/Debug/net8.0/Shop.Api.dll");
    let port = free_port();
    let mut app = std::process::Command::new(&dotnet.path)
        .arg(&dll)
        .current_dir(dll.parent().unwrap())
        .env("ASPNETCORE_URLS", format!("http://127.0.0.1:{port}"))
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let start = Instant::now();
    while !port_open(port) {
        assert!(start.elapsed() < Duration::from_secs(60), "the API did not start");
        std::thread::sleep(Duration::from_millis(100));
    }

    let controller = shop().join("src/Shop.Api/Controllers/OrdersController.cs");
    let line = line_of(&controller, "var order = Order.Sample();");
    let mut bps = Breakpoints::new();
    bps.set(BreakpointSpec::new(&controller, line));
    let mut r = Run::start(DebugTarget::Attach { pid: app.id() }, DebugOptions::default(), bps);
    r.until("attached", Duration::from_secs(60), |s| s.state() == &SessionState::Running && s.breakpoints().get(&controller, line).is_some_and(|b| b.verified));

    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || tx.send(http_get(port, "/orders/sample")).unwrap());
    r.until_stopped();
    assert_eq!(r.location().2, line);
    r.s.continue_();
    let deadline = Instant::now() + Duration::from_secs(60);
    let response = loop {
        r.pump();
        if let Ok(resp) = rx.try_recv() {
            break resp;
        }
        assert!(Instant::now() < deadline, "no HTTP response");
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");

    // Stopping an attach session detaches: the app keeps serving.
    let adapter_pid = r.s.adapter_pid().unwrap();
    r.s.stop();
    let deadline = Instant::now() + Duration::from_secs(15);
    while alive(adapter_pid) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
        r.pump();
    }
    assert!(!alive(adapter_pid), "netcoredbg still runs");
    let again = http_get(port, "/health");
    let _ = app.kill();
    let _ = app.wait();
    assert!(again.starts_with("HTTP/1.1 200") && again.contains("\r\n\r\n2\r\nok\r\n"), "{again}");
}
