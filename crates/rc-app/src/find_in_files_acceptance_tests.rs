//! Find in Files result navigation in the Shop solution.

use crate::screens::{HOME_LOCK, fixtures, harness, test_app};
use crate::toolwindow::ToolWindow;
use egui::vec2;
use egui_kittest::kittest::Queryable;

#[test]
fn shop_find_in_files_result_opens_exact_csharp_line() {
    let _lock = HOME_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let home = tempfile::tempdir().unwrap();
    let mut app = test_app(home.path(), true);
    app.settings.roslyn_path = Some("/nonexistent".into());
    app.open_workspace(&fixtures().join("Shop/Shop.sln"));
    let mut find = crate::search::FindInFiles::new(&app.workspace.as_ref().unwrap().ws.root, rc_core::Waker::noop(), "The price of an order, step by step.".into());
    find.search();
    app.find_in_files = Some(find);
    app.dock.show(ToolWindow::Find);
    let mut h = harness(app, vec2(1200.0, 800.0));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while h.state().find_in_files.as_ref().unwrap().summary.is_none() && std::time::Instant::now() < deadline {
        h.run_steps(1);
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_eq!(h.state().find_in_files.as_ref().unwrap().match_count(), 1);
    let label = "src/Shop.Core/PriceCalculator.cs:3 /// <summary>The price of an order, step by step.</summary>";
    h.get_by_label(label).click();
    h.run_steps(2);
    assert_eq!(h.state().docs.active_doc().unwrap().path, fixtures().join("Shop/src/Shop.Core/PriceCalculator.cs"));
    assert_eq!(h.state().docs.active_doc().unwrap().editor.caret_line_col().0, 2);
}
