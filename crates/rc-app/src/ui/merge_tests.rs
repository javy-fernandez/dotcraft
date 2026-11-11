//! The merge tool driven through its UI in kittest, against real repositories in temp dirs.

use crate::app::DotCraftApp;
use egui::accesskit::Role;
use egui_kittest::{Harness, kittest::Queryable};
use rc_kit::i18n::tr;
use std::path::Path;
use std::time::{Duration, Instant};

fn git(dir: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git").current_dir(dir).args(args).env("LC_ALL", "C").output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

pub(crate) const BASE: &str = "class Shop\n{\n    int a = 1;\n\n    int b = 2;\n\n    int c = 3;\n}\n";

/// A repository where merging `topic` into `main` conflicts on `b` (and changes `a` and `c`
/// without conflicts).
pub(crate) fn conflicting_repo(root: &Path) {
    std::fs::create_dir_all(root).unwrap();
    git(root, &["init", "-q", "-b", "main"]);
    for (k, v) in [("user.name", "Ada Lovelace"), ("user.email", "ada@example.com"), ("commit.gpgsign", "false"), ("core.hooksPath", "no-hooks"), ("merge.conflictStyle", "merge")] {
        git(root, &["config", k, v]);
    }
    let commit = |text: &str, msg: &str| {
        std::fs::write(root.join("Shop.cs"), text).unwrap();
        git(root, &["add", "."]);
        git(root, &["commit", "-qm", msg]);
    };
    commit(BASE, "Add the shop");
    git(root, &["switch", "-qc", "topic"]);
    commit(&BASE.replace("a = 1", "a = 10").replace("b = 2", "b = 20"), "Tune a and b");
    git(root, &["switch", "-q", "main"]);
    commit(&BASE.replace("c = 3", "c = 30").replace("b = 2", "b = 200"), "Tune b and c");
}

fn frames(h: &mut Harness<'static, DotCraftApp>, what: &str, mut done: impl FnMut(&DotCraftApp) -> bool) {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(30) {
        h.run_steps(1);
        if done(h.state()) {
            h.run_steps(2);
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("timed out waiting for {what}; error {:?} / {:?}", h.state().vcs.error, h.state().vcs.merge.as_ref().and_then(|m| m.error.clone()));
}

fn click(h: &mut Harness<'static, DotCraftApp>, label: &str) {
    h.get_by_role_and_label(Role::Button, label).click();
    h.run_steps(3);
}

fn open(base: &Path, root: &Path, commit_window: bool) -> Harness<'static, DotCraftApp> {
    let mut app = crate::screens::test_app(&base.join("home"), true);
    app.settings.roslyn_path = Some("/nonexistent".into());
    app.open_workspace(root);
    if commit_window {
        app.dock.show(crate::toolwindow::ToolWindow::Commit);
    }
    crate::screens::harness(app, egui::vec2(1400.0, 800.0))
}

fn result_text(h: &Harness<'static, DotCraftApp>) -> String {
    h.state().vcs.merge.as_ref().unwrap().open.as_ref().unwrap().result_text()
}

#[test]
fn failed_merge_opens_the_merge_tool_and_is_resolved_and_continued() {
    let _lock = crate::screens::HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = tempfile::tempdir().unwrap();
    let base = home.path().canonicalize().unwrap();
    let root = base.join("repo");
    conflicting_repo(&root);
    let mut h = open(&base, &root, false);
    frames(&mut h, "repository", |a| a.vcs.branches.iter().any(|b| b.name == "topic"));

    // The failed merge opens the conflicts list by itself.
    h.state_mut().vcs.merge_branch("topic");
    frames(&mut h, "merge tool", |a| a.vcs.merge.as_ref().is_some_and(|m| !m.loading && !m.files.is_empty()));
    assert_eq!(h.state().vcs.merge.as_ref().unwrap().files, ["Shop.cs"]);
    assert!(h.query_by_role_and_label(Role::Button, tr("Continue Merge")).is_some());
    click(&mut h, tr("Merge…"));
    frames(&mut h, "three panes", |a| a.vcs.merge.as_ref().is_some_and(|m| m.open.is_some()));
    assert_eq!(h.state().vcs.merge.as_ref().unwrap().open.as_ref().unwrap().model.unresolved(), 3);

    click(&mut h, tr("Apply All Non-Conflicting Changes"));
    assert_eq!(result_text(&h), BASE.replace("a = 1", "a = 10").replace("c = 3", "c = 30"));
    // The conflict: theirs, then ours appended below it.
    click(&mut h, tr("Accept Right Change"));
    click(&mut h, tr("Append Left Change"));
    let merged = BASE.replace("a = 1", "a = 10").replace("    int b = 2;\n", "    int b = 20;\n    int b = 200;\n").replace("c = 3", "c = 30");
    assert_eq!(result_text(&h), merged);
    assert!(h.query_by_role_and_label(Role::Button, tr("Accept Left Change")).is_none());

    click(&mut h, tr("Apply"));
    frames(&mut h, "resolved", |a| a.vcs.merge.as_ref().is_some_and(|m| m.open.is_none() && m.files.is_empty() && m.busy.is_none() && !m.loading));
    assert_eq!(std::fs::read_to_string(root.join("Shop.cs")).unwrap(), merged);
    click(&mut h, tr("Continue Merge"));
    frames(&mut h, "merge committed", |a| a.vcs.merge.is_none());
    assert_eq!(git(&root, &["log", "-1", "--format=%P"]).split_whitespace().count(), 2);
    assert!(git(&root, &["status", "--porcelain"]).trim().is_empty());
}

#[test]
fn result_is_editable_and_unresolved_changes_need_a_second_apply() {
    let _lock = crate::screens::HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = tempfile::tempdir().unwrap();
    let base = home.path().canonicalize().unwrap();
    let root = base.join("repo");
    conflicting_repo(&root);
    assert_ne!(std::process::Command::new("git").current_dir(&root).args(["merge", "-q", "topic"]).output().unwrap().status.code(), Some(0));
    let mut h = open(&base, &root, false);
    frames(&mut h, "repository", |a| a.vcs.repo.is_some());
    h.state_mut().vcs.open_merge_file("Shop.cs");
    frames(&mut h, "panes", |a| a.vcs.merge.as_ref().is_some_and(|m| m.open.is_some()));
    // Typing in the result resolves the change under the caret by hand.
    {
        let f = h.state_mut().vcs.merge.as_mut().unwrap().open.as_mut().unwrap();
        let at = f.result.buffer.rope().to_string().find("int b").unwrap();
        let at = f.result.buffer.rope().byte_to_char(at);
        f.result.replace(at, at, "// both: ");
    }
    h.run_steps(3);
    let f = h.state().vcs.merge.as_ref().unwrap().open.as_ref().unwrap();
    assert_eq!(f.model.unresolved_conflicts(), 0);
    assert_eq!(f.model.unresolved(), 2);
    // Two non-conflicting changes are still open: Apply asks for a second click.
    click(&mut h, tr("Apply"));
    assert!(h.state().vcs.merge.as_ref().unwrap().confirm_apply);
    assert!(h.state().vcs.merge.as_ref().unwrap().busy.is_none());
    click(&mut h, tr("Apply"));
    frames(&mut h, "resolved", |a| a.vcs.merge.as_ref().is_some_and(|m| m.open.is_none() && m.busy.is_none()));
    assert_eq!(std::fs::read_to_string(root.join("Shop.cs")).unwrap(), BASE.replace("int b", "// both: int b"));
}

#[test]
fn commit_window_opens_conflicts_and_whole_file_sides_and_abort_work() {
    let _lock = crate::screens::HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = tempfile::tempdir().unwrap();
    let base = home.path().canonicalize().unwrap();
    let root = base.join("repo");
    conflicting_repo(&root);
    assert_ne!(std::process::Command::new("git").current_dir(&root).args(["merge", "-q", "topic"]).output().unwrap().status.code(), Some(0));
    let mut h = open(&base, &root, true);
    frames(&mut h, "status", |a| a.vcs.status.as_ref().is_some_and(|s| s.conflicts().count() == 1));

    // Commit window: the toolbar action opens the list.
    click(&mut h, tr("Resolve Conflicts…"));
    frames(&mut h, "list", |a| a.vcs.merge.as_ref().is_some_and(|m| !m.loading));
    click(&mut h, tr("Close Merge Tool"));
    assert!(h.state().vcs.merge.is_none());
    h.state_mut().vcs.open_merge_file("Shop.cs");
    frames(&mut h, "panes", |a| a.vcs.merge.as_ref().is_some_and(|m| m.open.is_some()));
    click(&mut h, tr("Back to Conflicts"));
    assert!(h.state().vcs.merge.as_ref().unwrap().open.is_none());

    // Accept Yours for the whole file, then abort the merge.
    click(&mut h, tr("Accept Yours"));
    frames(&mut h, "accepted", |a| a.vcs.merge.as_ref().is_some_and(|m| m.files.is_empty() && m.busy.is_none() && !m.loading));
    assert!(std::fs::read_to_string(root.join("Shop.cs")).unwrap().contains("b = 200;"));
    click(&mut h, tr("Abort Merge"));
    frames(&mut h, "aborted", |a| a.vcs.merge.is_none());
    assert!(!root.join(".git/MERGE_HEAD").exists());
}
