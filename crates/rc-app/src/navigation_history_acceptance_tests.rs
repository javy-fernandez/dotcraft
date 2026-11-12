//! Navigation history skips removed C# files without losing the current location.

use crate::screens::{HOME_LOCK, fixtures, harness, test_app};
use egui::vec2;
use rc_editor::Action;
use rc_text::Selection;

#[test]
fn shop_navigation_skips_removed_source_and_keeps_forward_location() {
    let _lock = HOME_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let home = tempfile::tempdir().unwrap();
    let copied = tempfile::tempdir().unwrap();
    let source = fixtures().join("Shop/src/Shop.Core/Order.cs");
    let current = fixtures().join("Shop/src/Shop.Api/Program.cs");
    let removed = copied.path().join("PriceCalculator.cs");
    std::fs::copy(fixtures().join("Shop/src/Shop.Core/PriceCalculator.cs"), &removed).unwrap();
    let mut app = test_app(home.path(), true);
    app.settings.roslyn_path = Some("/nonexistent".into());
    app.open_workspace(&fixtures().join("Shop/Shop.sln"));
    app.open_file(&source);
    app.docs.active_doc_mut().unwrap().editor.set_selection(Selection::point(5));
    app.push_nav();
    app.open_file(&removed);
    app.docs.active_doc_mut().unwrap().editor.set_selection(Selection::point(3));
    app.push_nav();
    app.open_file(&current);
    std::fs::remove_file(&removed).unwrap();
    let mut h = harness(app, vec2(1000.0, 700.0));

    h.state_mut().pending.push(Action::NavigateBack);
    h.run_steps(2);
    assert_eq!(h.state().docs.active_doc().unwrap().path, source);
    assert_eq!(h.state().docs.active_doc().unwrap().editor.caret(), 5);
    assert!(h.state().nav_back.is_empty());
    assert_eq!(h.state().nav_forward.len(), 1);

    h.state_mut().pending.push(Action::NavigateForward);
    h.run_steps(2);
    assert_eq!(h.state().docs.active_doc().unwrap().path, current);
    assert!(h.state().nav_forward.is_empty());
}
