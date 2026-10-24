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

    let mut settings = Settings::default();
    settings.roslyn_path = Some("/nonexistent".into());
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
    app.vcs.new_branch("feature/x");
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
