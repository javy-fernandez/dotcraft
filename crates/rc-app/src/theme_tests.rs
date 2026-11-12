#[test]
fn dotnet_workspace_switches_between_light_and_dark_themes() {
    let _home = crate::screens::HOME_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let home = tempfile::tempdir().unwrap();
    let solution = crate::screens::fixtures().join("Shop/Shop.sln");
    let mut app = crate::screens::test_app(home.path(), false);
    app.workspace = Some(crate::app::WorkspaceState {
        ws: rc_core::Workspace::open(&solution).unwrap(),
        solution: Some(rc_dotnet::Solution::load(&solution).unwrap()),
    });
    let mut harness = crate::screens::harness(app, egui::vec2(1000.0, 700.0));
    assert!(!harness.ctx.global_style().visuals.dark_mode);
    harness.state_mut().settings.theme = rc_core::settings::ThemeChoice::Dark;
    harness.run_steps(3);
    assert!(harness.ctx.global_style().visuals.dark_mode);
    harness.state_mut().settings.theme = rc_core::settings::ThemeChoice::Light;
    harness.run_steps(3);
    assert!(!harness.ctx.global_style().visuals.dark_mode);
    assert_eq!(harness.state().workspace.as_ref().unwrap().solution.as_ref().unwrap().projects().len(), 6);
}
