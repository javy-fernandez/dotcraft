//! State machine tests against `tests/fake_adapter.py`, a scripted adapter that logs the
//! requests it receives.

use super::*;
use crate::netcoredbg::Origin;
use std::time::Instant;

fn fake(dir: &Path, mode: &str) -> (AdapterCommand, PathBuf) {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fake_adapter.py");
    let log = dir.join("requests.jsonl");
    let mut cmd = AdapterCommand::new("python3", Origin::Settings);
    cmd.prefix = vec!["-u".into(), script.display().to_string(), log.display().to_string(), mode.into()];
    (cmd, log)
}

fn requests(log: &Path) -> Vec<Value> {
    std::fs::read_to_string(log).unwrap_or_default().lines().map(|l| serde_json::from_str(l).unwrap()).collect()
}

fn commands(log: &Path) -> Vec<String> {
    requests(log).iter().map(|r| r["command"].as_str().unwrap().to_string()).collect()
}

fn launch() -> DebugTarget {
    DebugTarget::Launch(LaunchConfig { program: "/usr/bin/dotnet".into(), args: vec!["app.dll".into()], cwd: "/w".into(), env: vec![("A".into(), "1".into())] })
}

fn calc() -> PathBuf {
    PathBuf::from("/w/Calc.cs")
}

/// Pumps until `done` holds for an event or for the session.
fn pump_until(s: &mut DebugSession, all: &mut Vec<DebugEvent>, mut done: impl FnMut(&DebugEvent, &DebugSession) -> bool) {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(10) {
        for e in s.pump() {
            let stop = done(&e, s);
            all.push(e);
            if stop {
                return;
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("timed out in state {:?}; events {all:#?}", s.state());
}

/// Pumps until the session satisfies `done`.
fn pump_until_state(s: &mut DebugSession, all: &mut Vec<DebugEvent>, mut done: impl FnMut(&DebugSession) -> bool) {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(10) {
        all.extend(s.pump());
        if done(s) {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("timed out in state {:?}; events {all:#?}", s.state());
}

fn stopped(e: &DebugEvent, _: &DebugSession) -> bool {
    matches!(e, DebugEvent::Stopped(_))
}

fn watch_value(s: &DebugSession, id: u64) -> WatchValue {
    s.watches().iter().find(|w| w.id == id).unwrap().value.clone()
}

fn settled(s: &DebugSession) -> bool {
    s.watches().iter().all(|w| !matches!(w.value, WatchValue::Evaluating))
}

fn result_of(v: WatchValue) -> String {
    match v {
        WatchValue::Value(v) => v.result,
        other => panic!("no value: {other:?}"),
    }
}

#[test]
fn handshake_order_stops_caches_watches_and_exit() {
    let dir = tempfile::tempdir().unwrap();
    let (adapter, log) = fake(dir.path(), "late");
    let mut bps = Breakpoints::new();
    bps.set(BreakpointSpec::new(calc(), 9));
    bps.set(BreakpointSpec::new("/w/Other.cs", 3).with_condition("y > 1"));
    let mut s = DebugSession::start(adapter, launch(), DebugOptions::default(), bps, Waker::noop());
    let mut ev = Vec::new();
    pump_until(&mut s, &mut ev, stopped);

    // initialize → launch → (initialized) → setBreakpoints per file → exception filters → configurationDone.
    assert_eq!(commands(&log)[..6], ["initialize", "launch", "setBreakpoints", "setBreakpoints", "setExceptionBreakpoints", "configurationDone"]);
    let reqs = requests(&log);
    assert_eq!(reqs[1]["arguments"]["program"], "/usr/bin/dotnet");
    assert_eq!(reqs[1]["arguments"]["env"]["A"], "1");
    assert_eq!(reqs[1]["arguments"]["justMyCode"], true);
    assert_eq!(reqs[3]["arguments"]["breakpoints"][0]["condition"], "y > 1");
    assert_eq!(reqs[4]["arguments"]["filters"], json!(["user-unhandled"]));
    assert_eq!(s.exception_filters_available().len(), 2);

    let states: Vec<SessionState> = ev.iter().filter_map(|e| if let DebugEvent::StateChanged(s) = e { Some(s.clone()) } else { None }).collect();
    assert_eq!(states[0], SessionState::Running);
    assert!(matches!(states[1], SessionState::Stopped(_)));
    assert!(ev.contains(&DebugEvent::ProcessStarted(4242)));
    assert_eq!(s.debuggee_pid(), Some(4242));
    assert!(ev.contains(&DebugEvent::Output { category: OutputCategory::Stdout, text: "hello\n".into() }));
    assert!(!ev.iter().any(|e| matches!(e, DebugEvent::Output { text, .. } if text == "ignored")));

    // Verification and the line the debugger bound the breakpoint to.
    let b = s.breakpoints().get(&calc(), 9).unwrap();
    assert!(b.verified);
    assert_eq!(b.display_line(), 10);
    assert!(s.breakpoints().get(Path::new("/w/Other.cs"), 3).unwrap().verified);

    let info = s.stop_info().unwrap().clone();
    assert_eq!((info.thread_id, &info.reason, info.frame_id), (1, &StopReason::Breakpoint, Some(10)));
    assert_eq!(info.breakpoints, vec![(calc(), 9)]);
    let frames = s.frames(1).unwrap();
    assert_eq!(frames.frames.len(), 2);
    assert!(!frames.has_more());
    assert_eq!(s.execution_point(), Some((calc(), 10)));

    // Scopes come with the stop, variables on demand (cached).
    pump_until(&mut s, &mut ev, |e, _| matches!(e, DebugEvent::ScopesLoaded { frame_id: 10 }));
    let locals = s.scopes(10).unwrap()[0].variables_reference;
    assert!(s.variables(locals).is_none());
    s.request_variables(locals);
    assert!(s.is_loading_variables(locals));
    pump_until(&mut s, &mut ev, |e, _| matches!(e, DebugEvent::VariablesLoaded { .. }));
    let vars = s.variables(locals).unwrap().to_vec();
    assert_eq!((vars[0].name.as_str(), vars[0].value.as_str()), ("x", "1"));
    s.request_variables(vars[1].variables_reference);
    pump_until(&mut s, &mut ev, |e, _| matches!(e, DebugEvent::VariablesLoaded { .. }));
    assert_eq!(s.variables(vars[1].variables_reference).unwrap()[0].value, "\"Ada\"");

    // Watches and evaluation.
    let w = s.add_watch("x * 10");
    let bad = s.add_watch("nope");
    pump_until_state(&mut s, &mut ev, settled);
    assert_eq!(result_of(watch_value(&s, w)), "10");
    assert!(matches!(watch_value(&s, bad), WatchValue::Error(m) if m.contains("CS0103")));
    let id = s.evaluate("order", EvalContext::Repl);
    pump_until(&mut s, &mut ev, |e, _| matches!(e, DebugEvent::Evaluated { .. }));
    let Some(DebugEvent::Evaluated { id: got, result: Ok(v), .. }) = ev.last().cloned() else { panic!() };
    assert_eq!((got, v.variables_reference), (id, 2001));

    // setVariable updates the cache and re-evaluates the watches.
    s.set_variable(locals, "x", "2");
    pump_until(&mut s, &mut ev, |e, _| matches!(e, DebugEvent::VariableSet { .. }));
    pump_until_state(&mut s, &mut ev, settled);
    assert_eq!(s.variables(locals).unwrap()[0].value, "2");
    assert_eq!(result_of(watch_value(&s, w)), "20");

    // Continue invalidates everything fetched in the stop; the next stop re-evaluates watches.
    s.continue_();
    assert_eq!(s.state(), &SessionState::Running);
    assert!(s.variables(locals).is_none() && s.frames(1).is_none() && s.scopes(10).is_none());
    assert_eq!(watch_value(&s, w), WatchValue::NotEvaluated);
    pump_until(&mut s, &mut ev, stopped);
    pump_until_state(&mut s, &mut ev, settled);
    assert_eq!(result_of(watch_value(&s, w)), "30");
    assert_eq!(s.breakpoints().get(&calc(), 9).unwrap().hits, 2);

    // Steps.
    s.step_over();
    pump_until(&mut s, &mut ev, stopped);
    assert_eq!(s.stop_info().unwrap().reason, StopReason::Step);
    assert_eq!(s.execution_point().unwrap().1, 11);
    s.step_in();
    pump_until(&mut s, &mut ev, stopped);
    assert_eq!(s.execution_point().unwrap().1, 20);
    s.step_out();
    pump_until(&mut s, &mut ev, stopped);
    assert_eq!(s.execution_point().unwrap().1, 12);

    // Selecting another frame changes the context.
    let caller = s.frames(1).unwrap().frames[1].id;
    s.select_frame(caller);
    assert_eq!(s.execution_point(), Some((PathBuf::from("/w/Program.cs"), 5)));

    // Breakpoint edits while stopped re-send that file only.
    let before = commands(&log).len();
    s.remove_breakpoint(&calc(), 9);
    pump_until(&mut s, &mut ev, |e, _| matches!(e, DebugEvent::BreakpointsChanged(p) if p == &calc()));
    pump_until_state(&mut s, &mut ev, |_| requests(&log).len() > before);
    let last = requests(&log).pop().unwrap();
    assert_eq!(last["command"], "setBreakpoints");
    assert_eq!(last["arguments"]["source"]["path"], "/w/Calc.cs");
    assert_eq!(last["arguments"]["breakpoints"], json!([]));

    // Run to the end.
    s.continue_();
    pump_until(&mut s, &mut ev, |e, _| matches!(e, DebugEvent::Terminated));
    assert!(ev.contains(&DebugEvent::Exited(3)));
    assert!(ev.contains(&DebugEvent::Output { category: OutputCategory::Stdout, text: "bye\n".into() }));
    assert_eq!(s.state(), &SessionState::Exited(3));
    let start = Instant::now();
    while !commands(&log).contains(&"disconnect".to_string()) && start.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(commands(&log).last().unwrap(), "disconnect");
    assert_eq!(requests(&log).last().unwrap()["arguments"]["terminateDebuggee"], true);
}

#[test]
fn logpoints_and_hit_conditions_are_emulated() {
    let dir = tempfile::tempdir().unwrap();
    let (adapter, log) = fake(dir.path(), "early");
    let mut bps = Breakpoints::new();
    bps.set(BreakpointSpec::new(calc(), 10).with_hit_condition(">= 2").with_log_message("x={x}!"));
    let mut s = DebugSession::start(adapter, launch(), DebugOptions::default(), bps, Waker::noop());
    let mut ev = Vec::new();
    pump_until(&mut s, &mut ev, |e, _| matches!(e, DebugEvent::Terminated));
    assert_eq!(commands(&log)[..6], ["initialize", "launch", "setBreakpoints", "setExceptionBreakpoints", "configurationDone", "threads"]);
    // The adapter lacks both capabilities: neither goes on the wire.
    assert_eq!(requests(&log)[2]["arguments"]["breakpoints"], json!([{"line": 10}]));
    assert!(!ev.iter().any(|e| matches!(e, DebugEvent::Stopped(_))));
    let console: Vec<&str> = ev.iter().filter_map(|e| if let DebugEvent::Output { category: OutputCategory::Console, text } = e { Some(text.as_str()) } else { None }).collect();
    assert_eq!(console, ["x=2!\n", "x=3!\n"]);
    assert_eq!(s.state(), &SessionState::Exited(3));
}

#[test]
fn hit_condition_stops_on_the_nth_hit_and_stop_disconnects() {
    let dir = tempfile::tempdir().unwrap();
    let (adapter, log) = fake(dir.path(), "early");
    let mut bps = Breakpoints::new();
    bps.set(BreakpointSpec::new(calc(), 10).with_hit_condition("3"));
    let mut s = DebugSession::start(adapter, launch(), DebugOptions::default(), bps, Waker::noop());
    let w = s.add_watch("x");
    assert_eq!(watch_value(&s, w), WatchValue::NotEvaluated);
    let mut ev = Vec::new();
    pump_until(&mut s, &mut ev, stopped);
    pump_until_state(&mut s, &mut ev, settled);
    assert_eq!(result_of(watch_value(&s, w)), "3");
    assert_eq!(ev.iter().filter(|e| matches!(e, DebugEvent::Stopped(_))).count(), 1);

    s.stop();
    assert_eq!(s.state(), &SessionState::Terminated);
    let start = Instant::now();
    while !commands(&log).contains(&"disconnect".to_string()) && start.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(commands(&log).contains(&"disconnect".to_string()));
    assert!(s.pump().contains(&DebugEvent::Terminated));
    // Later messages do not revive an ended session.
    std::thread::sleep(Duration::from_millis(100));
    assert!(s.pump().iter().all(|e| !matches!(e, DebugEvent::StateChanged(_) | DebugEvent::Stopped(_))));
}

#[test]
fn a_missing_adapter_fails_the_session() {
    let adapter = AdapterCommand::new("/nonexistent/netcoredbg", Origin::Settings);
    let mut s = DebugSession::start(adapter, launch(), DebugOptions::default(), Breakpoints::new(), Waker::noop());
    let ev = s.pump();
    assert!(matches!(s.state(), SessionState::Failed(Failure::AdapterNotStarted(_))));
    assert!(ev.iter().any(|e| matches!(e, DebugEvent::Failed(_))));
    let id = s.evaluate("x", EvalContext::Repl);
    assert!(matches!(&s.pump()[..], [DebugEvent::Evaluated { id: got, result: Err(_), .. }] if *got == id));
}

#[test]
fn failure_messages_have_templates() {
    let f = Failure::AttachFailed { pid: 42, error: "denied".into() };
    assert_eq!(f.to_string(), "The debugger could not attach to process 42: denied");
    assert_eq!(StopReason::parse("exception").label(), "Paused on exception");
}
