//! Run configuration selection and session persistence with the Shop solution.

use crate::screens::{HOME_LOCK, fixtures, harness, test_app};
use egui::vec2;
use egui_kittest::kittest::Queryable;

#[test]
fn shop_run_configuration_selection_persists_across_workspace_reopen() {
    let _lock = HOME_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let home = tempfile::tempdir().unwrap();
    let solution = fixtures().join("Shop/Shop.sln");
    let mut app = test_app(home.path(), true);
    app.settings.roslyn_path = Some("/nonexistent".into());
    app.open_workspace(&solution);
    let cli = app.runner.configs.iter().find(|config| config.name.starts_with("Shop.Cli")).unwrap().name.clone();
    let selected = app.runner.selected_config().unwrap().name.clone();
    assert_ne!(selected, cli);
    let mut h = harness(app, vec2(1800.0, 900.0));
    h.get_by_label(&format!("Run configuration: {selected}")).click();
    h.run_steps(1);
    h.get_all_by_label(&cli).find(|node| node.rect().center().x > 800.0).unwrap().click();
    h.run_steps(2);
    assert_eq!(h.state().runner.selected_config().unwrap().name, cli);
    h.state_mut().save_session();

    let mut again = test_app(home.path(), true);
    again.settings.roslyn_path = Some("/nonexistent".into());
    again.open_workspace(&solution);
    assert_eq!(again.runner.selected_config().unwrap().name, cli);
}
