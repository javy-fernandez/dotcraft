//! History editing acceptance against the Shop solution and real Git.

use crate::app::DotCraftApp;
use egui::accesskit::Role;
use egui_kittest::{Harness, kittest::Queryable};
use std::path::Path;
use std::time::{Duration, Instant};

fn git(root: &Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git").current_dir(root).args(args).output().unwrap();
    assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn copy_tree(source: &Path, target: &Path) {
    std::fs::create_dir_all(target).unwrap();
    for entry in std::fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let source = entry.path();
        let target = target.join(entry.file_name());
        if source.is_dir() {
            copy_tree(&source, &target);
        } else {
            std::fs::copy(source, target).unwrap();
        }
    }
}

#[test]
fn amend_saves_csharp_buffer_and_rewrites_only_selected_commit_paths() {
    let _lock = crate::screens::HOME_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let home = tempfile::tempdir().unwrap();
    let root = home.path().join("Shop");
    copy_tree(&crate::screens::fixtures().join("Shop"), &root);
    git(&root, &["init", "-q", "-b", "main"]);
    for (key, value) in [("user.name", "Test"), ("user.email", "test@example.com"), ("commit.gpgsign", "false")] {
        git(&root, &["config", key, value]);
    }
    git(&root, &["add", "."]);
    git(&root, &["commit", "-qm", "Original Shop commit"]);
    let old_head = git(&root, &["rev-parse", "HEAD"]);
    let file = root.join("src/Shop.Core/Order.cs");
    let excluded_file = root.join("src/Shop.Core/PriceCalculator.cs");
    let original = std::fs::read_to_string(&file).unwrap();
    let excluded_original = std::fs::read_to_string(&excluded_file).unwrap();

    let mut app = crate::screens::test_app(home.path(), true);
    app.open_workspace(&root.join("Shop.sln"));
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.vcs.status.is_none() && Instant::now() < deadline {
        app.vcs.update(&mut app.docs, true);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(app.vcs.status.is_some());
    app.open_file(&file);
    app.docs.active_doc_mut().unwrap().editor.replace(0, 0, "// amended from editor\n");
    std::fs::write(&excluded_file, format!("// excluded\n{excluded_original}")).unwrap();
    app.vcs.excluded.insert("src/Shop.Core/PriceCalculator.cs".into());
    app.vcs.message = "Amended Shop commit".into();
    assert!(app.docs.active_doc().unwrap().is_modified());
    assert!(app.vcs.changes().is_empty(), "the UI status must still predate the saved edit");

    let mut first = true;
    let mut harness = Harness::builder().with_size(egui::vec2(900.0, 650.0)).build_ui_state(move |ui, app: &mut DotCraftApp| {
        if std::mem::take(&mut first) {
            rc_kit::theme::install_fonts(ui.ctx());
            return;
        }
        crate::ui::git::commit_window(app, ui);
    }, app);
    harness.run_steps(3);
    harness.get_by_role_and_label(Role::CheckBox, rc_kit::i18n::tr("Amend")).click();
    harness.run_steps(2);
    assert!(harness.state().vcs.amend);
    harness.get_by_role_and_label(Role::Button, rc_kit::i18n::tr("Commit")).click();
    harness.run_steps(2);
    assert!(harness.state().vcs.busy.is_some());
    let deadline = Instant::now() + Duration::from_secs(10);
    while harness.state().vcs.busy.is_some() && Instant::now() < deadline {
        let app = harness.state_mut();
        app.vcs.update(&mut app.docs, true);
        harness.run_steps(2);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(harness.state().vcs.busy.is_none(), "{:?}", harness.state().vcs.error);
    assert!(harness.state().vcs.error.is_none(), "{:?}", harness.state().vcs.error);
    assert!(!harness.state().vcs.amend);
    assert!(!harness.state().docs.active_doc().unwrap().is_modified());
    assert_ne!(git(&root, &["rev-parse", "HEAD"]), old_head);
    assert_eq!(git(&root, &["rev-list", "--count", "HEAD"]).trim(), "1");
    assert_eq!(git(&root, &["log", "-1", "--format=%s"]).trim(), "Amended Shop commit");
    assert_eq!(git(&root, &["show", "HEAD:src/Shop.Core/Order.cs"]), format!("// amended from editor\n{original}"));
    assert_eq!(git(&root, &["show", "HEAD:src/Shop.Core/PriceCalculator.cs"]), excluded_original);
    assert!(std::fs::read_to_string(&excluded_file).unwrap().starts_with("// excluded\n"));

    // The same save-before-commit path also includes a fresh editor change in a
    // regular commit even when the displayed status has not refreshed yet.
    harness.state_mut().docs.active_doc_mut().unwrap().editor.replace(0, 0, "// next commit\n");
    harness.state_mut().vcs.message = "Next Shop commit".into();
    harness.run_steps(2);
    harness.get_by_role_and_label(Role::Button, rc_kit::i18n::tr("Commit")).click();
    harness.run_steps(2);
    assert!(harness.state().vcs.busy.is_some());
    let deadline = Instant::now() + Duration::from_secs(10);
    while harness.state().vcs.busy.is_some() && Instant::now() < deadline {
        let app = harness.state_mut();
        app.vcs.update(&mut app.docs, true);
        harness.run_steps(2);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(harness.state().vcs.error.is_none(), "{:?}", harness.state().vcs.error);
    assert_eq!(git(&root, &["rev-list", "--count", "HEAD"]).trim(), "2");
    assert!(git(&root, &["show", "HEAD:src/Shop.Core/Order.cs"]).starts_with("// next commit\n// amended from editor\n"));
    assert_eq!(git(&root, &["show", "HEAD:src/Shop.Core/PriceCalculator.cs"]), excluded_original);
    harness.state_mut().close_workspace();
}
