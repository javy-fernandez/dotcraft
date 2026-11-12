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
    app.runner.kill_all_now();
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

#[test]
fn a_new_solution_starts_with_the_last_layout() {
    let _lock = crate::screens::HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = tempfile::tempdir().unwrap();
    // SAFETY: serialized by HOME_LOCK.
    unsafe { std::env::set_var("DOTCRAFT_HOME", home.path()) };
    let settings = Settings { roslyn_path: Some("/nonexistent".into()), ..Settings::default() };
    let mut app = DotCraftApp::with_settings(settings, Waker::noop(), None);
    app.open_workspace(&crate::screens::fixtures().join("Shop/Shop.sln"));
    app.dock.move_to(crate::toolwindow::ToolWindow::Explorer, crate::toolwindow::Side::Right);
    app.save_session();
    // Kept in the settings file too.
    let saved = Settings::load();
    assert_eq!(saved.tool_windows.get("explorer").and_then(|t| t.side.clone()).as_deref(), Some("right"));
    app.open_workspace(&crate::screens::fixtures().join("TenProjects/TenProjects.slnx"));
    assert_eq!(app.dock.side_of(crate::toolwindow::ToolWindow::Explorer), crate::toolwindow::Side::Right);
    assert_eq!(app.dock.right, Some(crate::toolwindow::ToolWindow::Explorer));
}

#[test]
fn shop_launch_profiles_can_be_selected_from_the_toolbar() {
    use egui_kittest::kittest::Queryable;

    let _lock = crate::screens::HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = tempfile::tempdir().unwrap();
    let project_dir = home.path().join("Shop.Api");
    let properties = project_dir.join("Properties");
    std::fs::create_dir_all(&properties).unwrap();
    let fixture = crate::screens::fixtures().join("Shop/src/Shop.Api");
    let project = project_dir.join("Shop.Api.csproj");
    std::fs::copy(fixture.join("Shop.Api.csproj"), &project).unwrap();
    let source = std::fs::read_to_string(fixture.join("Properties/launchSettings.json")).unwrap();
    let original: serde_json::Value = serde_json::from_str(source.trim_start_matches('\u{feff}')).unwrap();
    let mut settings_json = original;
    settings_json["profiles"]["alternate"] = serde_json::json!({
        "commandName": "Project",
        "applicationUrl": "http://localhost:5123",
        "environmentVariables": { "ASPNETCORE_ENVIRONMENT": "Staging" }
    });
    std::fs::write(properties.join("launchSettings.json"), serde_json::to_vec(&settings_json).unwrap()).unwrap();

    let mut app = crate::screens::test_app(home.path(), false);
    app.open_workspace(&project);
    let names: Vec<_> = app.runner.configs.iter().map(|c| c.name.as_str()).collect();
    assert!(names.contains(&"Shop.Api: http"), "{names:?}");
    assert!(names.contains(&"Shop.Api: alternate"), "{names:?}");
    assert!(!names.iter().any(|name| name.contains("IIS Express")), "{names:?}");

    let mut harness = crate::screens::harness(app, egui::vec2(1200.0, 800.0));
    let initial = harness.state().runner.selected_config().unwrap().name.clone();
    harness.get_by_label(&format!("Run configuration: {initial}")).click();
    harness.run_steps(2);
    harness.get_by_label("Shop.Api: http").click();
    harness.run_steps(2);
    let selected = harness.state().runner.selected_config().unwrap();
    assert_eq!(selected.name, "Shop.Api: http");
    let request = harness.state().runner.request_for(selected).unwrap();
    assert_eq!(request.launch_profile.as_deref(), Some("http"));
    assert!(request.use_launch_profile);
    harness.get_by_label("Run configuration: Shop.Api: http").click();
    harness.run_steps(2);
    harness.get_by_label("Shop.Api: alternate").click();
    harness.run_steps(2);
    let selected = harness.state().runner.selected_config().unwrap();
    assert_eq!(selected.name, "Shop.Api: alternate");
    assert_eq!(harness.state().runner.request_for(selected).unwrap().launch_profile.as_deref(), Some("alternate"));
}
