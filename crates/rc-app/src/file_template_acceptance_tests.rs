//! Class-file creation from a real Shop project through the app dialog.

use crate::item_templates::ItemKind;
use crate::screens::{HOME_LOCK, fixtures, harness, test_app};
use crate::ui::dialogs::Dialog;
use egui::vec2;
use egui_kittest::kittest::Queryable;

#[test]
fn shop_class_template_creates_namespaced_file_and_refuses_duplicate() {
    let _lock = HOME_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let root = project.path().join("Shop.Core");
    std::fs::create_dir(&root).unwrap();
    std::fs::copy(fixtures().join("Shop/src/Shop.Core/Shop.Core.csproj"), root.join("Shop.Core.csproj")).unwrap();
    let dir = root.join("Orders");
    std::fs::create_dir(&dir).unwrap();
    let mut app = test_app(home.path(), true);
    app.settings.roslyn_path = Some("/nonexistent".into());
    app.open_workspace(&fixtures().join("Shop/Shop.sln"));
    app.dialog = Some(Dialog::NewFile { dir: dir.clone(), name: "Invoice".into(), kind: ItemKind::Class, error: None });
    let mut h = harness(app, vec2(1000.0, 700.0));
    h.get_by_label("Create").click();
    h.run_steps(2);
    let file = dir.join("Invoice.cs");
    let contents = std::fs::read_to_string(&file).unwrap();
    assert!(contents.contains("namespace Shop.Core.Orders;"));
    assert!(contents.contains("public class Invoice"));
    assert_eq!(h.state().docs.active_doc().unwrap().path, file);

    h.state_mut().dialog = Some(Dialog::NewFile { dir, name: "Invoice".into(), kind: ItemKind::Class, error: None });
    h.run_steps(2);
    h.get_by_label("Create").click();
    h.run_steps(2);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), contents);
    assert!(matches!(&h.state().dialog, Some(Dialog::NewFile { error: Some(_), .. })));
}
