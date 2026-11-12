//! Phase 6 closure from the app with the real `git`: the status lists changes, edited lines
//! get gutter marks, the diff view aligns both sides, a commit and a new branch work, and
//! blame annotates every line.

use crate::app::DotCraftApp;
use rc_core::{Settings, Waker};
use std::path::Path;
use std::time::{Duration, Instant};

fn git(dir: &Path, args: &[&str]) {
    let out = std::process::Command::new("git").args(args).current_dir(dir).output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

fn copy_fixture_tree(source: &Path, destination: &Path) {
    std::fs::create_dir_all(destination).unwrap();
    for entry in std::fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        let target = destination.join(entry.file_name());
        if path.is_dir() {
            copy_fixture_tree(&path, &target);
        } else {
            std::fs::copy(&path, &target).unwrap();
        }
    }
}

#[test]
fn clone_shop_solution_from_the_git_tool_window() {
    use egui_kittest::{Harness, kittest::Queryable};
    let _lock = crate::screens::HOME_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let home = tempfile::tempdir().unwrap();
    let fixture = crate::screens::fixtures().join("Shop");
    let source = home.path().join("shop-source");
    copy_fixture_tree(&fixture, &source);
    git(&source, &["init", "-q", "-b", "main"]);
    git(&source, &["config", "user.name", "Test"]);
    git(&source, &["config", "user.email", "test@example.com"]);
    git(&source, &["add", "."]);
    git(&source, &["commit", "-qm", "Shop fixture"]);
    let destination = home.path().canonicalize().unwrap().join("Shop clone");

    let mut app = crate::screens::test_app(home.path(), true);
    app.vcs.tools.visible = true;
    app.vcs.tools.source = "--upload-pack=unsafe".into();
    app.vcs.tools.destination = destination.display().to_string();
    let mut first = true;
    let mut harness = Harness::builder().with_size(egui::vec2(900.0, 1000.0)).build_ui_state(move |ui, app: &mut DotCraftApp| {
        if std::mem::take(&mut first) {
            rc_kit::theme::install_fonts(ui.ctx());
            return;
        }
        app.vcs.update(&mut app.docs, true);
        crate::ui::git::git_window(app, ui);
    }, app);
    harness.run_steps(3);
    harness.get_by_role_and_label(egui::accesskit::Role::Button, rc_kit::i18n::tr("Clone Repository")).click();
    let deadline = Instant::now() + Duration::from_secs(5);
    while harness.state().vcs.error.is_none() && Instant::now() < deadline {
        harness.run_steps(2);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(harness.state().vcs.error.is_some());
    assert!(!destination.exists());

    harness.state_mut().vcs.tools.source = source.display().to_string();
    harness.run_steps(2);
    harness.get_by_role_and_label(egui::accesskit::Role::Button, rc_kit::i18n::tr("Clone Repository")).click();
    let deadline = Instant::now() + Duration::from_secs(10);
    while harness.state().vcs.tools.cloned.is_none() && Instant::now() < deadline {
        harness.run_steps(2);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(harness.state().vcs.tools.cloned.as_deref(), Some(destination.as_path()));
    assert_eq!(std::fs::read(destination.join("Shop.sln")).unwrap(), std::fs::read(source.join("Shop.sln")).unwrap());
    assert!(destination.join("src/Shop.Core/Shop.Core.csproj").is_file());
    harness.get_by_role_and_label(egui::accesskit::Role::Button, rc_kit::i18n::tr("Open cloned repository")).click();
    harness.run_steps(3);
    let workspace = harness.state().workspace.as_ref().unwrap();
    assert_eq!(workspace.ws.root, destination);
    assert_eq!(workspace.solution.as_ref().unwrap().projects().len(), 6);
    harness.state_mut().close_workspace();
}

fn pump(app: &mut DotCraftApp, what: &str, mut done: impl FnMut(&mut DotCraftApp) -> bool) {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(30) {
        let n = app.vcs.update(&mut app.docs, true);
        app.notices.extend(n.into_iter().map(|text| crate::app::Notice { text, error: false, at: 0.0 }));
        if done(app) {
            return;
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    panic!("timed out waiting for {what}; error {:?}", app.vcs.error);
}

#[test]
fn git_changes_marks_diff_commit_branch_and_blame() {
    let _lock = crate::screens::HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = tempfile::tempdir().unwrap();
    // SAFETY: serialized by HOME_LOCK.
    unsafe { std::env::set_var("DOTCRAFT_HOME", home.path()) };
    let repo = tempfile::tempdir().unwrap();
    let root = repo.path().canonicalize().unwrap();
    git(&root, &["init", "-q", "-b", "main"]);
    git(&root, &["config", "user.name", "Test"]);
    git(&root, &["config", "user.email", "test@example.com"]);
    std::fs::write(root.join("Program.cs"), "class A\n{\n    void M() { }\n}\n").unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-q", "-m", "First"]);

    let settings = Settings { roslyn_path: Some("/nonexistent".into()), ..Settings::default() };
    let mut app = DotCraftApp::with_settings(settings, Waker::noop(), None);
    app.open_workspace(&root);
    pump(&mut app, "status", |a| a.vcs.status.is_some());
    assert_eq!(app.vcs.branch_name().as_deref(), Some("main"));

    // Edit the file: a modified line and an added one show in the gutter.
    app.open_file(&root.join("Program.cs"));
    let doc = app.docs.active_doc_mut().unwrap();
    let pos = doc.editor.buffer.pos_at(2, 4);
    doc.editor.replace(pos, pos + 8, "void N(int x) { }\n    int y;");
    pump(&mut app, "gutter marks", |a| !a.docs.active_doc().unwrap().editor.line_changes.is_empty());
    let marks = app.docs.active_doc().unwrap().editor.line_changes.clone();
    assert_eq!(marks.get(&2), Some(&rc_editor::LineChange::Modified), "{marks:?}");
    // One line replaced by two: IntelliJ marks the whole block as modified.
    assert!(matches!(marks.get(&3), Some(rc_editor::LineChange::Modified | rc_editor::LineChange::Added)), "{marks:?}");
    assert!(!marks.contains_key(&0) && !marks.contains_key(&4), "{marks:?}");

    // Save → the status lists the change; the diff view aligns HEAD and the working tree.
    app.save_all_docs();
    app.vcs.refresh();
    pump(&mut app, "changed status", |a| a.vcs.changes().iter().any(|c| c.path == "Program.cs"));
    app.vcs.open_diff("Program.cs");
    pump(&mut app, "diff", |a| a.vcs.diff.is_some());
    let d = app.vcs.diff.as_ref().unwrap();
    assert_eq!(d.left.buffer.len_lines(), d.right.buffer.len_lines());
    assert!(!d.right.line_backgrounds.is_empty());

    // Commit, then a new branch.
    app.vcs.message = "Second".into();
    app.vcs.commit(false);
    pump(&mut app, "commit", |a| a.vcs.busy.is_none() && a.notices.iter().any(|n| n.text == rc_kit::i18n::tr("Committed")));
    pump(&mut app, "clean status", |a| a.vcs.status.as_ref().is_some_and(|s| s.entries.is_empty()));
    app.vcs.new_branch_from("feature/x", None, true);
    pump(&mut app, "branch", |a| a.vcs.branch_name().as_deref() == Some("feature/x"));
    pump(&mut app, "log", |a| a.vcs.log.iter().any(|c| c.subject == "Second"));

    // Blame annotates every line.
    let (path, text) = (root.join("Program.cs"), app.docs.active_doc().unwrap().editor.buffer.rope().to_string());
    app.vcs.toggle_blame(&path, text, &mut app.docs);
    pump(&mut app, "blame", |a| a.docs.active_doc().unwrap().editor.annotations.is_some());
    let ann = app.docs.active_doc().unwrap().editor.annotations.clone().unwrap();
    assert_eq!(ann.len(), app.docs.active_doc().unwrap().editor.buffer.len_lines() - 1);
    assert!(ann.iter().all(|a| a.contains("Test")), "{ann:?}");
}

#[test]
fn stash_actions_refresh_and_keep_the_selected_entry() {
    let _lock = crate::screens::HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = tempfile::tempdir().unwrap();
    // SAFETY: serialized by HOME_LOCK.
    unsafe { std::env::set_var("DOTCRAFT_HOME", home.path()) };
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    git(&root, &["init", "-q", "-b", "main"]);
    for (key, value) in [("user.name", "Test"), ("user.email", "test@example.com"), ("commit.gpgsign", "false"), ("core.hooksPath", "no-hooks")] {
        git(&root, &["config", key, value]);
    }
    std::fs::write(root.join("file.txt"), "base\n").unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-qm", "base"]);
    let settings = Settings { roslyn_path: Some("/nonexistent".into()), ..Settings::default() };
    let mut app = DotCraftApp::with_settings(settings, Waker::noop(), None);
    app.open_workspace(&root);
    pump(&mut app, "repository", |a| a.vcs.status.is_some());

    std::fs::write(root.join("file.txt"), "saved change\n").unwrap();
    std::fs::write(root.join("new.txt"), "untracked\n").unwrap();
    app.vcs.stash_message = "first stash".into();
    app.vcs.stash_include_untracked = true;
    app.vcs.stash_push();
    pump(&mut app, "stash push", |a| a.vcs.busy.is_none() && a.vcs.stashes.len() == 1);
    let selected = app.vcs.stashes[0].clone();
    assert_eq!(std::fs::read_to_string(root.join("file.txt")).unwrap(), "base\n");
    assert!(!root.join("new.txt").exists());
    assert!(app.vcs.stash_message.is_empty());

    // Another client adds a stash: the selected entry moves from index 0 to index 1.
    std::fs::write(root.join("file.txt"), "other change\n").unwrap();
    git(&root, &["stash", "push", "-qm", "external stash"]);
    app.vcs.stash_apply(&selected, false);
    pump(&mut app, "stash apply", |a| a.vcs.busy.is_none() && a.vcs.stashes.len() == 2);
    assert_eq!(std::fs::read_to_string(root.join("file.txt")).unwrap(), "saved change\n");
    assert_eq!(std::fs::read_to_string(root.join("new.txt")).unwrap(), "untracked\n");

    git(&root, &["reset", "--hard", "HEAD"]);
    std::fs::remove_file(root.join("new.txt")).unwrap();
    app.vcs.stash_apply(&selected, true);
    pump(&mut app, "stash pop", |a| a.vcs.busy.is_none() && a.vcs.stashes.len() == 1);
    assert_eq!(std::fs::read_to_string(root.join("file.txt")).unwrap(), "saved change\n");
    let remaining = app.vcs.stashes[0].clone();
    assert_ne!(remaining.hash, selected.hash);
    app.vcs.stash_drop(&remaining);
    pump(&mut app, "stash drop", |a| a.vcs.busy.is_none() && a.vcs.stashes.is_empty());

    // A stale action reports an error instead of applying a different stash.
    app.vcs.stash_apply(&selected, false);
    pump(&mut app, "missing stash", |a| a.vcs.busy.is_none() && a.vcs.error.is_some());
    assert!(app.vcs.error.as_ref().unwrap().contains(&selected.hash));

    // The UI requires confirmation before dispatching destructive actions.
    use egui_kittest::{Harness, kittest::Queryable};
    app.vcs.error = None;
    app.vcs.show_stashes = true;
    app.vcs.stashes = vec![selected.clone()];
    let mut first = true;
    let mut harness = Harness::builder().with_size(egui::vec2(900.0, 600.0)).build_ui_state(
        move |ui, app: &mut DotCraftApp| {
            if std::mem::take(&mut first) {
                rc_kit::theme::install_fonts(ui.ctx());
                return;
            }
            crate::ui::git::git_window(app, ui);
        },
        app,
    );
    harness.run_steps(2);
    harness.get_by_label(rc_kit::i18n::tr("Drop")).click();
    harness.run_steps(2);
    assert!(harness.state().vcs.busy.is_none());
    assert!(harness.state().vcs.stash_confirmation.as_ref().is_some_and(|(s, drop)| s.hash == selected.hash && *drop));
    harness.get_by_label(rc_kit::i18n::tr("Confirm Drop"));
    harness.get_by_label(rc_kit::i18n::tr("Cancel")).click();
    harness.run_steps(2);
    assert!(harness.state().vcs.stash_confirmation.is_none());
    harness.get_by_label(rc_kit::i18n::tr("Pop")).click();
    harness.run_steps(2);
    harness.get_by_label(rc_kit::i18n::tr("Confirm Pop"));
    assert!(harness.state().vcs.busy.is_none());
    assert!(harness.state().vcs.stash_confirmation.as_ref().is_some_and(|(_, drop)| !drop));
}

#[test]
fn branch_merge_rebase_push_and_conflict_guard() {
    let _lock = crate::screens::HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = tempfile::tempdir().unwrap();
    // SAFETY: serialized by HOME_LOCK.
    unsafe { std::env::set_var("DOTCRAFT_HOME", home.path()) };
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().canonicalize().unwrap();
    let (remote, root) = (base.join("remote.git"), base.join("repo"));
    std::fs::create_dir(&remote).unwrap();
    std::fs::create_dir(&root).unwrap();
    git(&remote, &["init", "-q", "--bare", "-b", "main"]);
    git(&root, &["init", "-q", "-b", "main"]);
    for (key, value) in [("user.name", "Test"), ("user.email", "test@example.com"), ("commit.gpgsign", "false"), ("core.hooksPath", "no-hooks")] {
        git(&root, &["config", key, value]);
    }
    git(&root, &["remote", "add", "origin", &format!("file://{}", remote.display())]);
    std::fs::write(root.join("a.txt"), "base\n").unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-qm", "base"]);
    git(&root, &["checkout", "-qb", "topic"]);
    std::fs::write(root.join("t.txt"), "t\n").unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-qm", "topic"]);
    git(&root, &["checkout", "-q", "main"]);
    std::fs::write(root.join("m.txt"), "m\n").unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-qm", "main"]);

    let settings = Settings { roslyn_path: Some("/nonexistent".into()), ..Settings::default() };
    let mut app = DotCraftApp::with_settings(settings, Waker::noop(), None);
    app.open_workspace(&root);
    pump(&mut app, "repository", |a| a.vcs.status.is_some() && !a.vcs.branches.is_empty());
    let has = |a: &DotCraftApp, text: &str| a.notices.iter().any(|n| n.text == text);
    let branch = |a: &DotCraftApp, name: &str| a.vcs.branches.iter().find(|b| b.name == name).unwrap().clone();

    // Push sets the upstream on origin.
    app.vcs.push_branch("main");
    pump(&mut app, "push", |a| has(a, "Pushed main"));
    pump(&mut app, "upstream", |a| a.vcs.branches.iter().any(|b| b.name == "main" && b.upstream.as_deref() == Some("origin/main")));

    // Rebase topic onto main, then merge topic into main (fast-forward).
    app.vcs.checkout(&branch(&app, "topic"));
    pump(&mut app, "checkout", |a| a.vcs.busy.is_none() && a.vcs.branch_name().as_deref() == Some("topic"));
    app.vcs.rebase_onto("main");
    pump(&mut app, "rebase", |a| has(a, "Rebased onto main"));
    assert!(root.join("m.txt").exists());
    app.vcs.checkout(&branch(&app, "main"));
    pump(&mut app, "checkout main", |a| a.vcs.busy.is_none() && a.vcs.branch_name().as_deref() == Some("main"));
    app.vcs.merge_branch("topic");
    pump(&mut app, "merge", |a| has(a, "Merged topic"));
    assert!(root.join("t.txt").exists());

    // A conflicting merge leaves the repository merging: branch changes are refused until
    // the merge is aborted.
    git(&root, &["checkout", "-qb", "other"]);
    std::fs::write(root.join("a.txt"), "other\n").unwrap();
    git(&root, &["commit", "-qam", "other"]);
    git(&root, &["checkout", "-q", "main"]);
    std::fs::write(root.join("a.txt"), "main\n").unwrap();
    git(&root, &["commit", "-qam", "main change"]);
    app.vcs.refresh();
    pump(&mut app, "branches", |a| a.vcs.branches.iter().any(|b| b.name == "other"));
    app.vcs.merge_branch("other");
    pump(&mut app, "conflict", |a| a.vcs.busy.is_none() && a.vcs.error.is_some());
    pump(&mut app, "merging state", |a| a.vcs.state == rc_vcs::RepoState::Merging);
    app.vcs.checkout(&branch(&app, "topic"));
    pump(&mut app, "refused checkout", |a| a.vcs.busy.is_none() && a.vcs.error.as_deref() == Some(rc_kit::i18n::tr("Resolve the merge conflicts first")));
    assert_eq!(app.vcs.branch_name().as_deref(), Some("main"));
    app.vcs.delete_branch("other", true);
    pump(&mut app, "refused delete", |a| a.vcs.busy.is_none() && a.vcs.error.is_some());
    assert!(app.vcs.branches.iter().any(|b| b.name == "other"));
    app.vcs.abort_operation();
    pump(&mut app, "abort", |a| has(a, "Merge aborted"));
    pump(&mut app, "clean state", |a| a.vcs.state == rc_vcs::RepoState::Clean);
    assert_eq!(std::fs::read_to_string(root.join("a.txt")).unwrap(), "main\n");
}

type AppHarness = egui_kittest::Harness<'static, DotCraftApp>;

fn frames(h: &mut AppHarness, what: &str, done: impl Fn(&DotCraftApp) -> bool) {
    let start = Instant::now();
    while !done(h.state()) {
        assert!(start.elapsed() < Duration::from_secs(30), "timed out waiting for {what}; error {:?}", h.state().vcs.error);
        h.run_steps(1);
        std::thread::sleep(Duration::from_millis(20));
    }
    h.run_steps(2);
}

/// The subjects of the interactive rebase dialog's plan, in order.
fn rebase_plan(a: &DotCraftApp) -> Option<Vec<String>> {
    match &a.dialog {
        Some(crate::ui::dialogs::Dialog::InteractiveRebase(d)) => d.plan().map(|p| p.steps.iter().map(|s| s.commit.subject.clone()).collect()),
        _ => None,
    }
}

/// c1 and c2 both rewrite `a.txt`: dragging c2 above c1 in the dialog conflicts twice; both
/// pauses are resolved from the banner and the rebase continues to the end.
#[test]
fn interactive_rebase_drag_conflict_pause_and_continue() {
    use egui_kittest::kittest::{NodeT, Queryable};
    use rc_kit::i18n::tr;
    let _lock = crate::screens::HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    git(&root, &["init", "-q", "-b", "main"]);
    for (key, value) in [("user.name", "Test"), ("user.email", "test@example.com"), ("commit.gpgsign", "false"), ("core.hooksPath", "no-hooks")] {
        git(&root, &["config", key, value]);
    }
    for (file, text, subject) in [("a.txt", "base\n", "init"), ("a.txt", "one\n", "c1"), ("a.txt", "two\n", "c2"), ("b.txt", "b\n", "c3")] {
        std::fs::write(root.join(file), text).unwrap();
        git(&root, &["add", "."]);
        git(&root, &["commit", "-qm", subject]);
    }
    let mut app = crate::screens::test_app(home.path(), true);
    app.settings.roslyn_path = Some("/nonexistent".into());
    app.open_workspace(&root);
    app.dock.show(crate::toolwindow::ToolWindow::Git);
    let mut h = crate::screens::harness(app, egui::vec2(1200.0, 800.0));
    frames(&mut h, "log", |a| a.vcs.log.len() == 4);
    let c1 = h.state().vcs.log.iter().find(|c| c.subject == "c1").unwrap().hash.clone();
    crate::ui::rebase::open(h.state_mut(), &c1);
    frames(&mut h, "plan", |a| rebase_plan(a).is_some());
    assert_eq!(rebase_plan(h.state()).unwrap(), ["c1", "c2", "c3"]);
    // Nothing changed yet: Start is disabled.
    assert!(h.get_by_label(tr("Start Rebasing")).accesskit_node().is_disabled());

    // Drag c2 above c1.
    // The Git log behind the dialog exposes the same subjects; the dialog rows come last.
    let (from, to) = (h.query_all_by_label("c2").last().unwrap().rect(), h.query_all_by_label("c1").last().unwrap().rect());
    h.drag_at(from.center());
    h.run_steps(1);
    for y in [from.center().y - 8.0, to.center().y, to.top() + 4.0] {
        h.hover_at(egui::pos2(from.center().x, y));
        h.run_steps(1);
    }
    h.drop_at(egui::pos2(from.center().x, to.top() + 4.0));
    h.run_steps(2);
    assert_eq!(rebase_plan(h.state()).unwrap(), ["c2", "c1", "c3"]);

    h.get_by_label(tr("Start Rebasing")).click();
    frames(&mut h, "first stop", |a| a.dialog.is_none() && a.vcs.busy.is_none() && a.vcs.rebase.as_ref().is_some_and(|r| r.conflicts == ["a.txt"]));
    assert_eq!(h.state().vcs.state, rc_vcs::RepoState::Rebasing);
    assert!(h.get_by_label(tr("Continue Rebase")).accesskit_node().is_disabled());
    for step in 1..=2 {
        h.get_by_label(tr("Accept Commit")).click();
        frames(&mut h, "resolved", |a| a.vcs.busy.is_none() && a.vcs.rebase.as_ref().is_some_and(|r| r.conflicts.is_empty()));
        h.get_by_label(tr("Continue Rebase")).click();
        if step == 1 {
            frames(&mut h, "second stop", |a| a.vcs.busy.is_none() && a.vcs.rebase.as_ref().is_some_and(|r| r.step == 2 && !r.conflicts.is_empty()));
        }
    }
    frames(&mut h, "finished", |a| a.vcs.busy.is_none() && a.vcs.state == rc_vcs::RepoState::Clean && a.notices.iter().any(|n| n.text == tr("Rebase finished")));
    let log = std::process::Command::new("git").args(["log", "--format=%s"]).current_dir(&root).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&log.stdout).lines().collect::<Vec<_>>(), ["c3", "c1", "c2", "init"]);
    assert_eq!(std::fs::read_to_string(root.join("a.txt")).unwrap(), "one\n");
}
