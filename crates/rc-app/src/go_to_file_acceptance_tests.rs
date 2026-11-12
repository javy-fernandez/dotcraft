//! Go to File with a C# source line in the real Shop solution.

use crate::screens::{HOME_LOCK, fixtures, harness, test_app};
use egui::{Key, vec2};
use rc_editor::Action;

#[test]
fn shop_go_to_file_name_and_line_opens_csharp_caret() {
    let _lock = HOME_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let home = tempfile::tempdir().unwrap();
    let mut app = test_app(home.path(), true);
    app.settings.roslyn_path = Some("/nonexistent".into());
    app.open_workspace(&fixtures().join("Shop/Shop.sln"));
    let mut h = harness(app, vec2(1200.0, 800.0));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while h.state().files.index.as_ref().unwrap().snapshot().get("src/Shop.Core/PriceCalculator.cs").is_none() && std::time::Instant::now() < deadline {
        h.run_steps(1);
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(h.state().files.index.as_ref().unwrap().snapshot().get("src/Shop.Core/PriceCalculator.cs").is_some());
    h.state_mut().pending.push(Action::GotoFile);
    h.run_steps(2);
    let search = h.state_mut().search_everywhere.as_mut().unwrap();
    assert_eq!(search.tab, crate::search::SeTab::Files);
    search.text = "PriceCalculator.cs:12".into();
    h.run_steps(2);
    h.key_press(Key::Enter);
    h.run_steps(2);
    assert!(h.state().search_everywhere.is_none());
    assert_eq!(h.state().docs.active_doc().unwrap().path, fixtures().join("Shop/src/Shop.Core/PriceCalculator.cs"));
    assert_eq!(h.state().docs.active_doc().unwrap().editor.caret_line_col().0, 11);
}
