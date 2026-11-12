#[test]
fn shop_folder_and_solution_open_as_distinct_desktop_workspaces() {
    let _home = crate::screens::HOME_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let home = tempfile::tempdir().unwrap();
    let shop = crate::screens::fixtures().join("Shop");
    let folder = shop.join("src");
    let solution = shop.join("Shop.sln");
    let mut app = crate::screens::test_app(home.path(), true);
    app.open_path(&folder);
    assert_eq!(app.workspace.as_ref().unwrap().ws.key_path(), folder);
    assert!(app.runner.configs.iter().any(|config| config.name == "Shop.Cli"));

    let mut harness = crate::screens::harness(app, egui::vec2(1000.0, 700.0));
    harness.state_mut().open_path(&solution);
    harness.run_steps(3);
    let app = harness.state();
    let workspace = app.workspace.as_ref().unwrap();
    assert_eq!(workspace.ws.key_path(), solution);
    assert_eq!(workspace.solution.as_ref().unwrap().projects().len(), 6);
    assert_eq!(app.settings.recent_workspaces.first(), Some(&solution));
    assert_eq!(app.settings.recent_workspaces.get(1), Some(&folder));
}
