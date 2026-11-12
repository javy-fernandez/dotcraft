use crate::runner::ConfigKind;

#[test]
fn standalone_shop_project_opens_in_the_desktop_workspace() {
    let _home = crate::screens::HOME_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let home = tempfile::tempdir().unwrap();
    let folder = crate::screens::fixtures().join("Shop/src/Shop.Cli");
    let project = folder.join("Shop.Cli.csproj");
    let mut app = crate::screens::test_app(home.path(), true);
    app.open_path(&folder);
    let workspace = app.workspace.as_ref().expect("standalone project folder opens a workspace");
    assert_eq!(workspace.ws.root, folder);
    assert!(workspace.solution.is_none());
    assert!(app.runner.configs.iter().any(|config| matches!(&config.kind, ConfigKind::App { project: path, .. } if path == &project)));

    let mut harness = crate::screens::harness(app, egui::vec2(1000.0, 700.0));
    harness.run_steps(2);
    assert_eq!(harness.state().workspace.as_ref().unwrap().ws.root, folder);
    assert!(harness.state().docs.list.is_empty());
}
