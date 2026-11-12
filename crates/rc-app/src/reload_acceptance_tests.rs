//! Conflict resolution with Shop C# source in the open solution UI.

use crate::screens::{HOME_LOCK, fixtures, harness, test_app};
use egui::vec2;
use egui_kittest::kittest::Queryable;

#[test]
fn shop_source_conflict_load_and_keep_choices() {
    let _lock = HOME_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let home = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    let path = source.path().join("PriceCalculator.cs");
    std::fs::copy(fixtures().join("Shop/src/Shop.Core/PriceCalculator.cs"), &path).unwrap();
    let mut app = test_app(home.path(), true);
    app.settings.roslyn_path = Some("/nonexistent".into());
    app.open_workspace(&fixtures().join("Shop/Shop.sln"));
    app.open_file(&path);
    let mut h = harness(app, vec2(1440.0, 900.0));
    let i = h.state().docs.index_of(&path).unwrap();

    h.state_mut().docs.list[i].editor.replace(0, 0, "// mine\n");
    let disk = format!("// disk\n{}", std::fs::read_to_string(&path).unwrap());
    std::fs::write(&path, &disk).unwrap();
    h.state_mut().docs.on_disk_change(&path);
    h.run_steps(2);
    assert!(h.state().docs.list[i].conflict);
    h.get_by_label("Load from Disk").click();
    h.run_steps(2);
    assert_eq!(h.state().docs.list[i].editor.buffer.rope().to_string(), disk);
    assert!(!h.state().docs.list[i].is_modified());
    assert!(!h.state().docs.list[i].conflict);

    h.state_mut().docs.list[i].editor.replace(0, 0, "// keep\n");
    std::fs::write(&path, "// newer disk\n").unwrap();
    h.state_mut().docs.on_disk_change(&path);
    h.run_steps(2);
    assert!(h.state().docs.list[i].conflict);
    h.get_by_label("Keep Mine").click();
    h.run_steps(2);
    assert!(std::fs::read_to_string(&path).unwrap().starts_with("// keep\n// disk\n"));
    assert!(!h.state().docs.list[i].conflict);
}
