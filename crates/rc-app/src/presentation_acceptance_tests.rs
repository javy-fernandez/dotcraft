//! Presentation mode in the open Shop solution.

use crate::app::FocusMode;
use crate::screens::{HOME_LOCK, fixtures, harness, test_app};
use egui::vec2;
use egui_kittest::kittest::Queryable;

fn view_menu(h: &mut egui_kittest::Harness<'static, crate::app::DotCraftApp>) {
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Main Menu").click();
    h.run_steps(2);
    h.get_by_label("View").click();
    h.run_steps(2);
    h.get_by_label("Appearance").click();
    h.run_steps(2);
}

#[test]
fn presentation_mode_scales_shop_editor_and_preserves_dock() {
    let _lock = HOME_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let home = tempfile::tempdir().unwrap();
    let mut app = test_app(home.path(), true);
    app.settings.roslyn_path = Some("/nonexistent".into());
    app.open_workspace(&fixtures().join("Shop/Shop.sln"));
    app.open_file(&fixtures().join("Shop/src/Shop.Core/PriceCalculator.cs"));
    let mut h = harness(app, vec2(1100.0, 750.0));
    let dock = h.state().dock.clone();
    let font = h.state().editor_config.font.size;
    view_menu(&mut h);
    h.get_by_label("Presentation Mode").click();
    h.run_steps(2);
    assert_eq!(h.state().focus_mode, FocusMode::Presentation);
    assert!(h.state().editor_config.font.size >= 24.0);
    assert!(h.state().editor_config.font.size > font);
    assert_eq!(h.state().dock, dock);

    let mut settings = h.state().settings.clone();
    settings.editor_font_size = 20.0;
    h.state_mut().apply_settings(&egui::Context::default(), settings);
    assert_eq!(h.state().editor_config.font.size, 30.0);
    // The title bar is hidden: the floating button leaves the mode.
    h.get_by_label("Exit Presentation Mode").click();
    h.run_steps(2);
    assert_eq!(h.state().focus_mode, FocusMode::Normal);
    assert_eq!(h.state().editor_config.font.size, 20.0);
    assert_eq!(h.state().dock, dock);

    view_menu(&mut h);
    h.get_by_label("Presentation Mode").click();
    h.run_steps(2);
    // Closing the project leaves the mode: the welcome screen keeps its title bar.
    h.state_mut().close_project();
    h.run_steps(2);
    assert_eq!(h.state().focus_mode, FocusMode::Normal);
    assert_eq!(h.state().editor_config.font.size, 20.0);
}
