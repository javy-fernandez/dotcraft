//! Accessible keyboard activation in the real Shop workspace.

use crate::screens::{HOME_LOCK, fixtures, harness, test_app};
use crate::toolwindow::ToolWindow;
use egui::{Key, vec2};
use egui_kittest::kittest::Queryable;

#[test]
fn keyboard_opens_recent_shop_and_toggles_explorer() {
    let _lock = HOME_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let home = tempfile::tempdir().unwrap();
    let mut app = test_app(home.path(), true);
    app.settings.roslyn_path = Some("/nonexistent".into());
    app.settings.recent_workspaces = vec![fixtures().join("Shop/Shop.sln")];
    let mut h = harness(app, vec2(1000.0, 700.0));
    h.get_by_label("Shop").focus();
    h.run_steps(2);
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert!(h.state().workspace.is_some());

    let explorer = h.get_by_label(ToolWindow::Explorer.title());
    explorer.focus();
    h.run_steps(2);
    h.key_press(Key::Space);
    h.run_steps(2);
    assert!(!h.state().dock.is_open(ToolWindow::Explorer));
    h.get_by_label(ToolWindow::Explorer.title()).focus();
    h.run_steps(2);
    h.key_press(Key::Space);
    h.run_steps(2);
    assert!(h.state().dock.is_open(ToolWindow::Explorer));
}
