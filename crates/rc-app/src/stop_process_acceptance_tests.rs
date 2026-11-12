//! Live stop of the Shop.Api application process with an isolated local SDK.

use crate::app::DotCraftApp;
use crate::screens::{HOME_LOCK, fixtures, test_app};
use rc_core::session::RunOptions;
use std::time::{Duration, Instant};

#[test]
#[ignore = "Requires a .NET 8 SDK and restored Shop.Api dependencies"]
fn live_shop_api_run_can_be_stopped() {
    let _lock = HOME_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let home = tempfile::tempdir().unwrap();
    let mut app: DotCraftApp = test_app(home.path(), true);
    app.settings.roslyn_path = Some("/nonexistent".into());
    app.open_workspace(&fixtures().join("Shop/Shop.sln"));
    let i = app.runner.configs.iter().position(|config| config.name.starts_with("Shop.Api")).unwrap();
    app.runner.selected = i;
    let name = app.runner.configs[i].name.clone();
    app.runner.options.insert(name, RunOptions {
        profile: Some(String::new()),
        env: "ASPNETCORE_URLS=http://127.0.0.1:0".into(),
        hot_reload: Some(false),
        ..Default::default()
    });
    app.runner.run_selected();
    let deadline = Instant::now() + Duration::from_secs(120);
    let mut listening = false;
    while Instant::now() < deadline {
        let outputs = app.runner.update();
        crate::ui::run_windows::handle_outputs(&mut app, outputs);
        if let Some(run) = app.runner.runs.first() {
            let text = run.console.term.screen_lines().join("\n");
            if text.contains("Now listening on") && run.handle.as_ref().and_then(|handle| handle.pid()).is_some() {
                listening = true;
                break;
            }
            if run.exit.is_some() {
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    app.runner.stop_all();
    let stop_deadline = Instant::now() + Duration::from_secs(15);
    while app.runner.runs.first().is_some_and(|run| run.exit.is_none()) && Instant::now() < stop_deadline {
        let outputs = app.runner.update();
        crate::ui::run_windows::handle_outputs(&mut app, outputs);
        std::thread::sleep(Duration::from_millis(50));
    }
    app.runner.kill_all_now();
    assert!(listening, "Shop.Api did not reach its listening state");
    let run = app.runner.runs.first().unwrap();
    assert!(run.exit.is_some(), "Shop.Api did not exit after Stop");
    assert!(run.console.term.screen_lines().join("\n").contains("Process stopped"));
}
