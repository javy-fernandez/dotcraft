//! Git stash acceptance against the Shop .NET fixture and the real Git tool window.

use crate::app::DotCraftApp;
use egui_kittest::{Harness, kittest::Queryable};
use std::path::Path;
use std::time::{Duration, Instant};

fn git(root: &Path, args: &[&str]) {
    let output = std::process::Command::new("git").current_dir(root).args(args).output().unwrap();
    assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
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
fn shelves_save_and_restore_shop_csharp_changes_from_the_git_window() {
    let _lock = crate::screens::HOME_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let home = tempfile::tempdir().unwrap();
    let root = home.path().join("Shop");
    copy_tree(&crate::screens::fixtures().join("Shop"), &root);
    git(&root, &["init", "-q", "-b", "main"]);
    for (key, value) in [("user.name", "Test"), ("user.email", "test@example.com"), ("commit.gpgsign", "false")] {
        git(&root, &["config", key, value]);
    }
    git(&root, &["add", "."]);
    git(&root, &["commit", "-qm", "Shop fixture"]);
    let file = root.join("src/Shop.Core/Order.cs");
    let base = std::fs::read_to_string(&file).unwrap();
    let note = root.join("src/Shop.Core/ShelfNote.cs");
    let mut app = crate::screens::test_app(home.path(), true);
    app.open_workspace(&root.join("Shop.sln"));
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.vcs.repo.is_none() && Instant::now() < deadline {
        app.vcs.update(&mut app.docs, true);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(app.vcs.repo.is_some());
    app.open_file(&file);
    app.docs.active_doc_mut().unwrap().editor.replace(0, 0, "// saved by shelf\n");
    assert!(app.docs.active_doc().unwrap().is_modified());
    std::fs::write(&note, "class ShelfNote {}\n").unwrap();
    app.vcs.show_stashes = true;
    app.vcs.stash_message = "Shop work in progress".into();
    app.vcs.stash_include_untracked = true;

    let mut first = true;
    let mut harness = Harness::builder().with_size(egui::vec2(1050.0, 700.0)).build_ui_state(move |ui, app: &mut DotCraftApp| {
        if std::mem::take(&mut first) {
            rc_kit::theme::install_fonts(ui.ctx());
            return;
        }
        app.vcs.update(&mut app.docs, true);
        crate::ui::git::git_window(app, ui);
    }, app);
    harness.run_steps(3);
    harness.get_by_label(rc_kit::i18n::tr("Stash Changes")).click();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !(harness.state().vcs.stashes.len() == 1 && harness.state().vcs.busy.is_none()) && Instant::now() < deadline {
        harness.run_steps(2);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(harness.state().vcs.stashes.len(), 1, "{:?}", harness.state().vcs.error);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), base);
    assert!(!note.exists());
    assert!(!harness.state().docs.active_doc().unwrap().is_modified());

    harness.get_by_label(rc_kit::i18n::tr("Pop")).click();
    harness.run_steps(2);
    assert!(harness.state().vcs.stash_confirmation.is_some());
    harness.get_by_label(rc_kit::i18n::tr("Confirm Pop")).click();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !(harness.state().vcs.stashes.is_empty() && harness.state().vcs.busy.is_none()) && Instant::now() < deadline {
        harness.run_steps(2);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(harness.state().vcs.stashes.is_empty(), "{:?}", harness.state().vcs.error);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), format!("// saved by shelf\n{base}"));
    assert_eq!(std::fs::read_to_string(&note).unwrap(), "class ShelfNote {}\n");
    harness.state_mut().close_workspace();
}
