//! Welcome-to-workspace lifecycle with the real Shop solution.

use crate::screens::{HOME_LOCK, fixtures, harness, test_app};
use egui::vec2;
use egui_kittest::kittest::Queryable;
use std::path::{Path, PathBuf};

#[derive(Debug)]
struct Dropped(PathBuf);

impl egui::DroppedFile for Dropped {
    fn path(&self) -> &Path {
        &self.0
    }

    fn bytes(&self) -> Result<Vec<u8>, String> {
        std::fs::read(&self.0).map_err(|error| error.to_string())
    }
}

#[test]
fn shop_solution_drop_close_and_recent_reopen_in_native_window() {
    let _lock = HOME_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let home = tempfile::tempdir().unwrap();
    let solution = fixtures().join("Shop/Shop.sln");
    let source = fixtures().join("Shop/src/Shop.Core/PriceCalculator.cs");
    let mut app = test_app(home.path(), true);
    app.settings.roslyn_path = Some("/nonexistent".into());
    let mut h = harness(app, vec2(1100.0, 750.0));
    assert!(h.state().workspace.is_none());
    h.input_mut().dropped_files.push(std::sync::Arc::new(Dropped(solution.clone())));
    h.run_steps(3);
    assert_eq!(h.state().workspace.as_ref().unwrap().solution.as_ref().unwrap().projects().len(), 6);
    h.state_mut().open_file(&source);
    h.run_steps(2);
    assert_eq!(h.state().docs.active_doc().unwrap().path, source);

    h.state_mut().close_project();
    h.run_steps(2);
    assert!(h.state().workspace.is_none());
    assert!(h.state().docs.list.is_empty());
    h.get_by_label("Shop").click();
    h.run_steps(3);
    assert!(h.state().workspace.is_some());
    assert_eq!(h.state().docs.active_doc().unwrap().path, source);
}
