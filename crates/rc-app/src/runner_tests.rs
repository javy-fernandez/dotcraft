//! Phase 3 closure from the app: the Shop fixture builds, Shop.Cli runs to exit code 0 with its
//! output in the Run console, and "All Tests" passes every test (xUnit, NUnit, MSTest).

use crate::app::DotCraftApp;
use crate::runner::ConfigKind;
use rc_core::{Settings, Waker};
use std::time::{Duration, Instant};

fn pump(app: &mut DotCraftApp, what: &str, mut done: impl FnMut(&mut DotCraftApp) -> bool) {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(300) {
        let outs = app.runner.update();
        crate::ui::run_windows::handle_outputs(app, outs);
        if done(app) {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("timed out waiting for {what}");
}

#[test]
fn shop_builds_runs_and_passes_its_tests() {
    let _lock = crate::screens::HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = tempfile::tempdir().unwrap();
    // SAFETY: serialized by HOME_LOCK.
    unsafe { std::env::set_var("DOTCRAFT_HOME", home.path()) };
    let settings = Settings { roslyn_path: Some("/nonexistent".into()), ..Settings::default() };
    let mut app = DotCraftApp::with_settings(settings, Waker::noop(), None);
    app.open_workspace(&crate::screens::fixtures().join("Shop/Shop.sln"));
    let names: Vec<String> = app.runner.configs.iter().map(|c| c.name.clone()).collect();
    assert!(names.iter().any(|n| n.starts_with("Shop.Cli")), "{names:?}");
    assert!(names.iter().any(|n| n.starts_with("Shop.Api")), "{names:?}");
    assert!(names.iter().any(|n| n == "All Tests"), "{names:?}");

    // Build.
    let target = app.workspace.as_ref().unwrap().ws.key_path().to_path_buf();
    app.runner.build(&target, rc_dotnet::BuildAction::Build);
    pump(&mut app, "build", |a| a.runner.build.as_ref().is_some_and(|b| b.result.is_some() || b.failed.is_some()));
    let b = app.runner.build.as_ref().unwrap();
    assert!(b.result.as_ref().is_some_and(|r| r.success), "build failed: {:?} {:?}", b.failed, b.diagnostics);

    // Run Shop.Cli.
    app.runner.selected = app.runner.configs.iter().position(|c| c.name.starts_with("Shop.Cli")).unwrap();
    app.runner.run_selected();
    pump(&mut app, "Shop.Cli", |a| a.runner.runs.first().is_some_and(|r| r.exit.is_some()));
    let r = &app.runner.runs[0];
    assert_eq!(r.exit, Some(Some(0)));
    let text: String = r.console.term.screen_lines().join("\n");
    assert!(text.contains("Total"), "console: {text}");

    // All tests.
    // "Run Tests" runs them all whatever configuration is selected (Shop.Cli here).
    assert!(matches!(app.runner.selected_config().unwrap().kind, ConfigKind::App { .. }));
    app.run_ide_action(&egui::Context::default(), rc_editor::Action::RunTests);
    assert_eq!(app.runner.tests.as_ref().unwrap().request.target, target);
    pump(&mut app, "tests", |a| a.runner.tests.as_ref().is_some_and(|t| !t.running));
    let t = app.runner.tests.as_ref().unwrap();
    assert_eq!(t.failed, 0, "{:?}", t.failure);
    assert!(t.passed >= 10, "passed {}", t.passed);
    assert!(app.runner.test_outcomes.values().all(|s| *s == crate::runner::TestState::Passed));
}
