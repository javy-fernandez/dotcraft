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

#[test]
fn a_compound_configuration_starts_several_applications() {
    let _lock = crate::screens::HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = tempfile::tempdir().unwrap();
    // SAFETY: serialized by HOME_LOCK.
    unsafe { std::env::set_var("DOTCRAFT_HOME", home.path()) };
    let settings = Settings { roslyn_path: Some("/nonexistent".into()), ..Settings::default() };
    let mut app = DotCraftApp::with_settings(settings, Waker::noop(), None);
    app.open_workspace(&crate::screens::fixtures().join("Shop/Shop.sln"));
    let apps = app.runner.app_names();
    assert!(apps.len() >= 2, "{apps:?}");
    app.runner.save_compound(None, "Both".into(), apps.clone());
    // Right after the applications, before the tests; selected.
    let i = app.runner.configs.iter().position(|c| c.name == "Both").unwrap();
    assert!(matches!(app.runner.configs[i - 1].kind, ConfigKind::App { .. }));
    assert_eq!(app.runner.selected_config().unwrap().name, "Both");
    app.runner.run_selected();
    assert_eq!(app.runner.runs.iter().map(|r| r.name.clone()).collect::<Vec<_>>(), apps);
    app.runner.stop_all();
    pump(&mut app, "the runs to stop", |a| a.runner.runs.iter().all(|r| r.exit.is_some()));
    // Kept in the session.
    app.save_session();
    let mut again = DotCraftApp::with_settings(Settings { roslyn_path: Some("/nonexistent".into()), ..Settings::default() }, Waker::noop(), None);
    again.open_workspace(&crate::screens::fixtures().join("Shop/Shop.sln"));
    assert!(again.runner.configs.iter().any(|c| c.name == "Both" && matches!(c.kind, ConfigKind::Compound { .. })));
}

#[test]
fn edited_run_configurations_reach_the_process_and_the_session() {
    let _lock = crate::screens::HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = tempfile::tempdir().unwrap();
    // SAFETY: serialized by HOME_LOCK.
    unsafe { std::env::set_var("DOTCRAFT_HOME", home.path()) };
    let settings = Settings { roslyn_path: Some("/nonexistent".into()), ..Settings::default() };
    let mut app = DotCraftApp::with_settings(settings.clone(), Waker::noop(), None);
    app.open_workspace(&crate::screens::fixtures().join("Shop/Shop.sln"));
    let cli = app.runner.configs.iter().find(|c| c.name.starts_with("Shop.Cli")).unwrap().clone();
    let options = rc_core::session::RunOptions { args: "Grace --exit 3".into(), env: "# comment\nDOTCRAFT_TEST=yes\n".into(), ..Default::default() };
    app.runner.options.insert(cli.name.clone(), options.clone());
    let req = app.runner.request_for(&cli).unwrap();
    assert_eq!(req.args, ["Grace", "--exit", "3"]);
    assert_eq!(req.env, [("DOTCRAFT_TEST".to_string(), "yes".to_string())]);
    app.runner.selected = app.runner.configs.iter().position(|c| c.name == cli.name).unwrap();
    app.runner.run_selected();
    pump(&mut app, "Shop.Cli", |a| a.runner.runs.first().is_some_and(|r| r.exit.is_some()));
    let r = &app.runner.runs[0];
    assert_eq!(r.exit, Some(Some(3)));
    assert!(r.console.term.screen_lines().join("\n").contains("Order for Grace"));
    app.save_session();
    let mut again = DotCraftApp::with_settings(settings, Waker::noop(), None);
    again.open_workspace(&crate::screens::fixtures().join("Shop/Shop.sln"));
    assert_eq!(again.runner.options.get(&cli.name), Some(&options));
}
