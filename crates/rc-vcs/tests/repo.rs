//! Integration tests against the real `git` in temporary repositories.

mod common;

use common::{TempRepo, git, numbered};
use rc_vcs::*;
use std::sync::{Arc, Mutex};

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

#[tokio::test]
async fn discovers_repository_from_nested_paths() {
    let t = TempRepo::new().await;
    t.write("src/deep/file.cs", "x\n");
    let from_file = Repository::discover(&t.path("src/deep/file.cs")).await.unwrap();
    assert_eq!(from_file.root(), t.repo.root());
    assert!(from_file.git_dir().ends_with(".git"));
    assert_eq!(from_file.rel_path(&t.path("src/deep/file.cs")).as_deref(), Some("src/deep/file.cs"));
    assert!(Git::detect().await.unwrap().version() >= git::MIN_VERSION);

    let outside = tempfile::tempdir().unwrap();
    match Repository::discover(outside.path()).await {
        Err(GitError::NotARepository(_)) => {}
        other => panic!("expected NotARepository, got {other:?}"),
    }
}

#[tokio::test]
async fn status_reports_branch_files_renames_and_untracked() {
    let t = TempRepo::new().await;
    let st = t.repo.status(&StatusOptions::default()).await.unwrap();
    assert_eq!(st.branch.head.as_deref(), Some("main"));
    assert_eq!(st.branch.oid, None);

    t.commit_files(&[("a.txt", "a\n"), ("b.txt", &numbered(20)), ("gone.txt", "g\n")], "init");
    t.write("a.txt", "a changed\n");
    t.write("new dir/new file.txt", "n\n");
    t.git(&["mv", "b.txt", "c.txt"]);
    t.git(&["rm", "-q", "gone.txt"]);
    t.write(".gitignore", "*.log\n");
    t.write("x.log", "ignored\n");

    let st = t.repo.status(&StatusOptions { ignored: true, ..Default::default() }).await.unwrap();
    assert!(st.branch.oid.is_some());
    let a = st.entry("a.txt").unwrap();
    assert_eq!((a.index, a.worktree), (FileState::Unmodified, FileState::Modified));
    let c = st.entry("c.txt").unwrap();
    assert_eq!((c.index, c.orig_path.as_deref()), (FileState::Renamed, Some("b.txt")));
    assert_eq!(st.entry("gone.txt").unwrap().index, FileState::Deleted);
    assert!(st.entry("new dir/new file.txt").unwrap().is_untracked());
    assert!(st.ignored().any(|e| e.path == "x.log"));
    assert_eq!(st.staged().count(), 2);
    assert_eq!(st.unstaged().count(), 1);
}

#[tokio::test]
async fn diffs_parse_for_every_spec() {
    let t = TempRepo::new().await;
    t.commit_files(&[("f.txt", &numbered(30))], "one");
    let first = t.repo.resolve("HEAD").await.unwrap();
    let mut text = numbered(30).replace("line 3\n", "line three\n");
    t.write("f.txt", &text);
    t.git(&["commit", "-qam", "two"]);
    text = text.replace("line 20\n", "line twenty\n");
    t.write("f.txt", &text);
    t.git(&["add", "f.txt"]);
    text.push_str("line 31\n");
    t.write("f.txt", &text);

    let opts = DiffOptions::default();
    let unstaged = t.repo.diff(&DiffSpec::WorktreeVsIndex, &opts).await.unwrap();
    assert_eq!(unstaged.len(), 1);
    let h = &unstaged[0].hunks[0];
    assert_eq!(h.lines.iter().filter(|l| l.kind == DiffLineKind::Added).count(), 1);
    assert_eq!(h.lines.last().unwrap().new_line, Some(31));

    let staged = t.repo.diff(&DiffSpec::IndexVsHead, &opts).await.unwrap();
    assert_eq!(staged[0].hunks.len(), 1);
    assert!(staged[0].hunks[0].lines.iter().any(|l| l.text == "line twenty" && l.new_line == Some(20)));

    let all = t.repo.diff(&DiffSpec::WorktreeVsHead, &opts).await.unwrap();
    assert_eq!(all[0].hunks.len(), 2);

    let between = t.repo.diff(&DiffSpec::Revisions { from: first.clone(), to: "HEAD".into() }, &opts).await.unwrap();
    assert!(between[0].hunks[0].lines.iter().any(|l| l.text == "line three"));

    let root = t.repo.diff(&DiffSpec::Commit(first.clone()), &opts).await.unwrap();
    assert_eq!(root[0].status, DiffStatus::Added);
    assert_eq!(root[0].hunks[0].lines.len(), 30);
    let second = t.repo.diff(&DiffSpec::Commit("HEAD".into()), &opts).await.unwrap();
    assert_eq!(second[0].hunks[0].lines.iter().filter(|l| l.kind != DiffLineKind::Context).count(), 2);

    let one_file = t.repo.diff(&DiffSpec::WorktreeVsHead, &DiffOptions { paths: s(&["nope.txt"]), ..Default::default() }).await.unwrap();
    assert!(one_file.is_empty());

    t.write("untracked.txt", "u1\nu2\n");
    let u = t.repo.diff_untracked("untracked.txt", 3).await.unwrap().unwrap();
    assert_eq!((u.status, u.path(), u.hunks[0].lines.len()), (DiffStatus::Added, "untracked.txt", 2));
}

#[tokio::test]
async fn renames_binaries_and_file_contents() {
    let t = TempRepo::new().await;
    t.commit_files(&[("old.txt", &numbered(10)), ("crlf.txt", "a\r\nb\r\n")], "init");
    std::fs::write(t.path("bin.dat"), [0u8, 1, 2, 3, 0, 255]).unwrap();
    t.git(&["mv", "old.txt", "new.txt"]);
    t.git(&["add", "bin.dat"]);
    let d = t.repo.diff(&DiffSpec::IndexVsHead, &DiffOptions::default()).await.unwrap();
    let bin = d.iter().find(|f| f.path() == "bin.dat").unwrap();
    assert!(bin.binary);
    let ren = d.iter().find(|f| f.status == DiffStatus::Renamed).unwrap();
    assert_eq!((ren.old_path.as_deref(), ren.new_path.as_deref(), ren.similarity), (Some("old.txt"), Some("new.txt"), Some(100)));

    let head = t.repo.show_file(&Revision::Head, "old.txt").await.unwrap().unwrap();
    assert_eq!(head.text(), numbered(10));
    assert!(t.repo.show_file(&Revision::Head, "new.txt").await.unwrap().is_none());
    assert!(t.repo.show_file(&Revision::Index, "new.txt").await.unwrap().is_some());
    assert!(t.repo.show_file(&Revision::Index, "bin.dat").await.unwrap().unwrap().binary);
    assert_eq!(t.repo.show_file(&Revision::Head, "crlf.txt").await.unwrap().unwrap().text(), "a\r\nb\r\n");
    assert!(t.repo.show_file(&Revision::Commit("HEAD".into()), "missing").await.unwrap().is_none());
}

#[tokio::test]
async fn stage_unstage_and_discard_hunks_and_lines() {
    let t = TempRepo::new().await;
    let base = numbered(40);
    t.commit_files(&[("f.txt", &base)], "init");
    let edited = base.replace("line 2\n", "line TWO\n").replace("line 30\n", "line THIRTY\nline 30b\n");
    t.write("f.txt", &edited);

    // Stage only the second hunk.
    let d = t.repo.diff(&DiffSpec::WorktreeVsIndex, &DiffOptions::default()).await.unwrap();
    assert_eq!(d[0].hunks.len(), 2);
    t.repo.stage_hunk(&d[0], 1).await.unwrap();
    let staged = t.repo.show_file(&Revision::Index, "f.txt").await.unwrap().unwrap();
    assert_eq!(staged.text(), base.replace("line 30\n", "line THIRTY\nline 30b\n"));

    // Unstage it again.
    let sd = t.repo.diff(&DiffSpec::IndexVsHead, &DiffOptions::default()).await.unwrap();
    t.repo.unstage_hunk(&sd[0], 0).await.unwrap();
    assert_eq!(t.repo.show_file(&Revision::Index, "f.txt").await.unwrap().unwrap().text(), base);

    // Stage a single added line of the second hunk ("line 30b").
    let d = t.repo.diff(&DiffSpec::WorktreeVsIndex, &DiffOptions::default()).await.unwrap();
    let h = &d[0].hunks[1];
    let idx = h.lines.iter().position(|l| l.text == "line 30b").unwrap();
    t.repo.stage_lines(&d[0], 1, &[idx]).await.unwrap();
    assert_eq!(t.repo.show_file(&Revision::Index, "f.txt").await.unwrap().unwrap().text(), base.replace("line 30\n", "line 30\nline 30b\n"));

    // Unstage that line through the staged diff.
    let sd = t.repo.diff(&DiffSpec::IndexVsHead, &DiffOptions::default()).await.unwrap();
    let idx = sd[0].hunks[0].lines.iter().position(|l| l.kind == DiffLineKind::Added).unwrap();
    t.repo.unstage_lines(&sd[0], 0, &[idx]).await.unwrap();
    assert_eq!(t.repo.show_file(&Revision::Index, "f.txt").await.unwrap().unwrap().text(), base);

    // Discard the first hunk in the work tree, keep the second.
    let d = t.repo.diff(&DiffSpec::WorktreeVsIndex, &DiffOptions::default()).await.unwrap();
    t.repo.discard_hunk(&d[0], 0).await.unwrap();
    assert_eq!(t.read("f.txt"), base.replace("line 30\n", "line THIRTY\nline 30b\n"));

    // Discard just the "line THIRTY" replacement (its removed and added lines).
    let d = t.repo.diff(&DiffSpec::WorktreeVsIndex, &DiffOptions::default()).await.unwrap();
    let h = &d[0].hunks[0];
    let sel: Vec<usize> = h.lines.iter().enumerate().filter(|(_, l)| l.text == "line 30" && l.kind == DiffLineKind::Removed || l.text == "line THIRTY").map(|(i, _)| i).collect();
    t.repo.discard_lines(&d[0], 0, &sel).await.unwrap();
    assert_eq!(t.read("f.txt"), base.replace("line 30\n", "line 30\nline 30b\n"));

    // Whole-file discard and rollback.
    t.repo.discard_unstaged(&s(&["f.txt"])).await.unwrap();
    assert_eq!(t.read("f.txt"), base);
}

#[tokio::test]
async fn stage_files_rollback_and_delete_untracked() {
    let t = TempRepo::new().await;
    t.commit_files(&[("a.txt", "a\n"), ("b.txt", "b\n")], "init");
    t.write("a.txt", "A\n");
    std::fs::remove_file(t.path("b.txt")).unwrap();
    t.write("n.txt", "new\n");
    t.repo.stage(&s(&["a.txt", "b.txt", "n.txt"])).await.unwrap();
    let st = t.repo.status(&StatusOptions::default()).await.unwrap();
    assert_eq!(st.staged().count(), 3);

    t.repo.unstage(&s(&["a.txt"])).await.unwrap();
    let st = t.repo.status(&StatusOptions::default()).await.unwrap();
    assert!(st.entry("a.txt").unwrap().is_unstaged() && !st.entry("a.txt").unwrap().is_staged());

    t.repo.rollback(&s(&["a.txt", "b.txt", "n.txt"])).await.unwrap();
    assert_eq!(t.read("a.txt"), "a\n");
    assert_eq!(t.read("b.txt"), "b\n");
    // The added file is unstaged, not deleted.
    assert_eq!(t.read("n.txt"), "new\n");
    let st = t.repo.status(&StatusOptions::default()).await.unwrap();
    assert_eq!(st.entries.len(), 1);
    assert!(st.entry("n.txt").unwrap().is_untracked());

    t.repo.delete_untracked(&s(&["n.txt"])).await.unwrap();
    assert!(!t.path("n.txt").exists());
}

#[tokio::test]
async fn commit_amend_signoff_and_only_selected_files() {
    let t = TempRepo::new().await;
    match t.repo.commit(&CommitOptions { message: "empty".into(), ..Default::default() }).await {
        Err(GitError::NothingToCommit) => {}
        other => panic!("expected NothingToCommit, got {other:?}"),
    }
    t.write("a.txt", "a\n");
    t.repo.stage(&s(&["a.txt"])).await.unwrap();
    let h1 = t.repo.commit(&CommitOptions { message: "First\n\nBody".into(), signoff: true, ..Default::default() }).await.unwrap();
    let msg = t.repo.commit_message("HEAD").await.unwrap();
    assert!(msg.starts_with("First\n\nBody") && msg.contains("Signed-off-by: Test User <test@example.com>"), "{msg}");

    let h2 = t.repo.commit(&CommitOptions { message: "First, amended".into(), amend: true, ..Default::default() }).await.unwrap();
    assert_ne!(h1, h2);
    assert_eq!(t.repo.log(&LogOptions::default()).await.unwrap().len(), 1);

    // Only selected files: b (tracked, modified) and c (untracked); a stays staged.
    t.commit_files(&[("b.txt", "b\n")], "b");
    t.write("a.txt", "a2\n");
    t.repo.stage(&s(&["a.txt"])).await.unwrap();
    t.write("b.txt", "b2\n");
    t.write("c.txt", "c\n");
    t.repo.commit(&CommitOptions { message: "b and c".into(), only: s(&["b.txt", "c.txt"]), ..Default::default() }).await.unwrap();
    let files = t.repo.commit_files("HEAD").await.unwrap();
    let mut names: Vec<_> = files.iter().map(|f| (f.status, f.path.as_str())).collect();
    names.sort();
    assert_eq!(names, vec![('A', "c.txt"), ('M', "b.txt")]);
    let st = t.repo.status(&StatusOptions::default()).await.unwrap();
    assert_eq!(st.staged().map(|e| e.path.as_str()).collect::<Vec<_>>(), vec!["a.txt"]);
}

#[tokio::test]
async fn branches_create_checkout_rename_delete() {
    let t = TempRepo::new().await;
    t.commit_files(&[("a.txt", "a\n")], "init");
    t.repo.create_branch("feature", None, true).await.unwrap();
    assert_eq!(t.repo.current_branch().await.unwrap().as_deref(), Some("feature"));
    t.commit_files(&[("f.txt", "f\n")], "feature work");
    match t.repo.create_branch("feature", None, false).await {
        Err(GitError::BranchExists(b)) => assert_eq!(b, "feature"),
        other => panic!("expected BranchExists, got {other:?}"),
    }
    t.repo.checkout_branch("main").await.unwrap();
    match t.repo.delete_branch("feature", false).await {
        Err(GitError::BranchNotFullyMerged(b)) => assert_eq!(b, "feature"),
        other => panic!("expected BranchNotFullyMerged, got {other:?}"),
    }
    t.repo.rename_branch("feature", "topic").await.unwrap();
    let branches = t.repo.branches().await.unwrap();
    let names: Vec<_> = branches.iter().map(|b| (b.name.as_str(), b.is_head, b.kind)).collect();
    assert_eq!(names, vec![("main", true, BranchKind::Local), ("topic", false, BranchKind::Local)]);
    t.repo.delete_branch("topic", true).await.unwrap();
    assert_eq!(t.repo.branches().await.unwrap().len(), 1);
    assert!(matches!(t.repo.checkout_branch("nope").await, Err(GitError::NotFound(_))));

    // Detached HEAD.
    let head = t.repo.resolve("HEAD").await.unwrap();
    t.repo.checkout_detached(&head).await.unwrap();
    assert_eq!(t.repo.current_branch().await.unwrap(), None);
    let st = t.repo.status(&StatusOptions::default()).await.unwrap();
    assert_eq!(st.branch.head, None);

    // Local changes that a checkout would overwrite.
    t.repo.checkout_branch("main").await.unwrap();
    t.repo.create_branch("other", None, true).await.unwrap();
    t.commit_files(&[("a.txt", "other\n")], "other");
    t.repo.checkout_branch("main").await.unwrap();
    t.write("a.txt", "dirty\n");
    match t.repo.checkout_branch("other").await {
        Err(GitError::LocalChangesWouldBeOverwritten(files)) => assert_eq!(files, vec!["a.txt".to_string()]),
        other => panic!("expected LocalChangesWouldBeOverwritten, got {other:?}"),
    }
}

#[tokio::test]
async fn stash_push_list_apply_pop_drop() {
    let t = TempRepo::new().await;
    t.commit_files(&[("a.txt", "a\n")], "init");
    assert!(matches!(t.repo.stash_push(&StashOptions::default()).await, Err(GitError::NothingToStash)));
    t.write("a.txt", "one\n");
    t.repo.stash_push(&StashOptions { message: Some("first".into()), ..Default::default() }).await.unwrap();
    t.write("a.txt", "two\n");
    t.write("u.txt", "untracked\n");
    t.repo.stash_push(&StashOptions { message: Some("second".into()), include_untracked: true, ..Default::default() }).await.unwrap();
    assert_eq!(t.read("a.txt"), "a\n");
    assert!(!t.path("u.txt").exists());

    let list = t.repo.stashes().await.unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!((list[0].index, list[0].message.as_str(), list[0].branch.as_deref()), (0, "On main: second", Some("main")));
    assert_eq!(t.repo.status(&StatusOptions::default()).await.unwrap().stash_count, 2);

    t.repo.stash_apply(1, false).await.unwrap();
    assert_eq!(t.read("a.txt"), "one\n");
    t.repo.rollback(&s(&["a.txt"])).await.unwrap();
    t.repo.stash_pop(0, false).await.unwrap();
    assert_eq!(t.read("a.txt"), "two\n");
    assert!(t.path("u.txt").exists());
    t.repo.stash_drop(0).await.unwrap();
    assert!(t.repo.stashes().await.unwrap().is_empty());
}

#[tokio::test]
async fn log_paging_file_history_and_graph() {
    let t = TempRepo::new().await;
    assert!(t.repo.log(&LogOptions::default()).await.unwrap().is_empty());
    t.commit_files(&[("a.txt", "1\n")], "c1");
    t.commit_files(&[("b.txt", "1\n")], "c2");
    t.git(&["tag", "v1"]);
    t.git(&["switch", "-q", "-c", "side"]);
    t.commit_files(&[("a.txt", "side\n")], "side work");
    t.git(&["switch", "-q", "main"]);
    t.commit_files(&[("c.txt", "1\n")], "c3");
    t.git(&["merge", "-q", "--no-edit", "--no-ff", "side"]);
    t.git(&["mv", "a.txt", "renamed.txt"]);
    t.git(&["commit", "-q", "-m", "rename\n\nwith body"]);

    let all = t.repo.log(&LogOptions::default()).await.unwrap();
    assert_eq!(all.len(), 6);
    assert_eq!(all[0].subject, "rename");
    assert_eq!(all[0].body, "with body");
    assert_eq!(all[0].head_branch.as_deref(), Some("main"));
    assert!(all.iter().any(|c| c.is_merge()));
    assert!(all.iter().any(|c| c.refs.contains(&RefLabel::Tag("v1".into()))));
    assert_eq!(all[0].author_email, "test@example.com");

    let page = t.repo.log(&LogOptions { skip: 2, max_count: 2, ..Default::default() }).await.unwrap();
    assert_eq!(page.iter().map(|c| &c.hash).collect::<Vec<_>>(), all[2..4].iter().map(|c| &c.hash).collect::<Vec<_>>());

    let history = t.repo.log(&LogOptions { paths: s(&["renamed.txt"]), follow: true, ..Default::default() }).await.unwrap();
    let subjects: Vec<_> = history.iter().map(|c| c.subject.as_str()).collect();
    assert_eq!(subjects, vec!["rename", "side work", "c1"]);

    let grep = t.repo.log(&LogOptions { grep: Some("SIDE WORK".into()), ..Default::default() }).await.unwrap();
    assert_eq!(grep.len(), 1);

    let mut graph = Graph::new();
    let rows = graph.rows(&all);
    assert_eq!(rows.len(), all.len());
    assert!(rows.iter().any(|r| r.column == 1));
    assert!(rows.iter().all(|r| r.width <= 2));
    assert_eq!(rows.last().unwrap().column, 0);
}

#[tokio::test]
async fn blame_matches_unsaved_buffer() {
    let t = TempRepo::new().await;
    t.commit_files(&[("f.txt", "one\ntwo\n")], "first");
    t.commit_files(&[("f.txt", "one\ntwo\nthree\n")], "second");
    let b = t.repo.blame("f.txt", None, None, None).await.unwrap();
    assert_eq!(b.lines.len(), 3);
    assert_eq!(b.commit_of(0).unwrap().summary, "first");
    assert_eq!(b.commit_of(2).unwrap().summary, "second");
    assert_eq!(b.commit_of(2).unwrap().author, "Test User");

    // The buffer has an inserted line that is not on disk.
    let buffer = "zero\none\ntwo\nthree\n";
    let b = t.repo.blame("f.txt", Some(buffer), None, None).await.unwrap();
    assert_eq!(b.lines.len(), 4);
    assert!(b.commit_of(0).unwrap().is_uncommitted());
    assert_eq!(b.commit_of(1).unwrap().summary, "first");
    assert_eq!(b.lines[1].orig_line, 1);
    assert_eq!(b.commit_of(3).unwrap().summary, "second");

    let old = t.repo.blame("f.txt", None, Some("HEAD~1"), None).await.unwrap();
    assert_eq!(old.lines.len(), 2);
}

#[tokio::test]
async fn merge_conflicts_are_listed_and_resolved() {
    let t = TempRepo::new().await;
    t.commit_files(&[("c.txt", "base\n"), ("ok.txt", "x\n")], "base");
    t.git(&["switch", "-q", "-c", "theirs"]);
    t.commit_files(&[("c.txt", "theirs\n")], "theirs");
    t.git(&["switch", "-q", "main"]);
    t.commit_files(&[("c.txt", "ours\n")], "ours");

    match t.repo.merge("theirs", false).await {
        Err(GitError::Conflicts(_)) => {}
        other => panic!("expected Conflicts, got {other:?}"),
    }
    assert_eq!(t.repo.state().await, RepoState::Merging);
    assert_eq!(t.repo.conflicted_paths().await.unwrap(), vec!["c.txt".to_string()]);
    let st = t.repo.status(&StatusOptions::default()).await.unwrap();
    let c: Vec<_> = st.conflicts().collect();
    assert_eq!((c.len(), c[0].conflict), (1, Some(ConflictKind::BothModified)));
    let v = t.repo.conflict_versions("c.txt").await.unwrap();
    assert_eq!(v.base.unwrap().text(), "base\n");
    assert_eq!(v.ours.unwrap().text(), "ours\n");
    assert_eq!(v.theirs.unwrap().text(), "theirs\n");
    match t.repo.commit(&CommitOptions { message: "m".into(), ..Default::default() }).await {
        Err(GitError::Conflicts(_)) => {}
        other => panic!("expected Conflicts on commit, got {other:?}"),
    }

    t.repo.resolve_with(&s(&["c.txt"]), true).await.unwrap();
    assert_eq!(t.read("c.txt"), "theirs\n");
    assert!(t.repo.conflicted_paths().await.unwrap().is_empty());
    t.repo.commit(&CommitOptions { message: "merged".into(), ..Default::default() }).await.unwrap();
    assert_eq!(t.repo.state().await, RepoState::Clean);

    // Abort a second conflicting merge.
    t.git(&["switch", "-q", "theirs"]);
    t.commit_files(&[("c.txt", "theirs 2\n")], "theirs 2");
    t.git(&["switch", "-q", "main"]);
    t.commit_files(&[("c.txt", "ours 2\n")], "ours 2");
    assert!(t.repo.merge("theirs", false).await.is_err());
    t.repo.abort_merge().await.unwrap();
    assert_eq!(t.repo.state().await, RepoState::Clean);
    assert_eq!(t.read("c.txt"), "ours 2\n");
}

#[tokio::test]
async fn fetch_pull_push_with_progress() {
    let remote_dir = tempfile::tempdir().unwrap();
    git(remote_dir.path(), &["init", "-q", "--bare", "-b", "main"]);
    let url = format!("file://{}", remote_dir.path().display());

    let a = TempRepo::new().await;
    a.git(&["remote", "add", "origin", &url]);
    a.commit_files(&[("f.txt", &numbered(200))], "init");
    match a.repo.push(&PushOptions::default(), None, None).await {
        Err(GitError::NoUpstream) => {}
        other => panic!("expected NoUpstream, got {other:?}"),
    }
    let lines = Arc::new(Mutex::new(Vec::<Progress>::new()));
    let sink: ProgressSink = {
        let lines = lines.clone();
        Arc::new(move |p| lines.lock().unwrap().push(p))
    };
    a.repo.push(&PushOptions { remote: Some("origin".into()), refspec: Some("main".into()), set_upstream: true, ..Default::default() }, Some(sink.clone()), None).await.unwrap();
    assert!(!lines.lock().unwrap().is_empty());

    // A second clone pushes a commit; the first one fetches and pulls it.
    let b_dir = tempfile::tempdir().unwrap();
    git(b_dir.path(), &["clone", "-q", &url, "."]);
    common::configure(b_dir.path());
    let b = Repository::discover(b_dir.path()).await.unwrap();
    std::fs::write(b.root().join("g.txt"), "g\n").unwrap();
    b.stage(&s(&["g.txt"])).await.unwrap();
    b.commit(&CommitOptions { message: "from b".into(), ..Default::default() }).await.unwrap();
    b.push(&PushOptions::default(), None, None).await.unwrap();

    a.repo.fetch(&FetchOptions { remote: Some("origin".into()), prune: true, ..Default::default() }, Some(sink), None).await.unwrap();
    let st = a.repo.status(&StatusOptions::default()).await.unwrap();
    assert_eq!((st.branch.upstream.as_deref(), st.branch.behind, st.branch.ahead), (Some("origin/main"), 1, 0));
    let branches = a.repo.branches().await.unwrap();
    assert!(branches.iter().any(|br| br.kind == BranchKind::Remote && br.name == "origin/main"));

    // Diverge, then a push is rejected until we pull.
    a.commit_files(&[("h.txt", "h\n")], "from a");
    match a.repo.push(&PushOptions::default(), None, None).await {
        Err(GitError::PushRejected(_)) => {}
        other => panic!("expected PushRejected, got {other:?}"),
    }
    a.repo.pull(&PullOptions { rebase: Some(true), ..Default::default() }, None, None).await.unwrap();
    a.repo.push(&PushOptions::default(), None, None).await.unwrap();
    assert!(a.path("g.txt").exists());
    assert_eq!(a.repo.remotes().await.unwrap()[0].name, "origin");

    // Remote branch checkout and deletion on the remote.
    b.create_branch("topic", None, false).await.unwrap();
    b.push(&PushOptions { remote: Some("origin".into()), refspec: Some("topic".into()), ..Default::default() }, None, None).await.unwrap();
    a.repo.fetch(&FetchOptions::default(), None, None).await.unwrap();
    a.repo.checkout_remote_branch("origin/topic", None).await.unwrap();
    assert_eq!(a.repo.current_branch().await.unwrap().as_deref(), Some("topic"));
    let topic = a.repo.branches().await.unwrap().into_iter().find(|br| br.name == "topic").unwrap();
    assert_eq!(topic.upstream.as_deref(), Some("origin/topic"));
    a.repo.checkout_branch("main").await.unwrap();
    a.repo.push(&PushOptions { remote: Some("origin".into()), refspec: Some("topic".into()), delete: true, ..Default::default() }, None, None).await.unwrap();
    a.repo.fetch(&FetchOptions { prune: true, ..Default::default() }, None, None).await.unwrap();
    let topic = a.repo.branches().await.unwrap().into_iter().find(|br| br.name == "topic").unwrap();
    assert!(topic.upstream_gone);
}

#[tokio::test]
async fn cancelled_operations_stop() {
    let t = TempRepo::new().await;
    let token = CancelToken::new();
    token.cancel();
    assert!(matches!(t.repo.fetch(&FetchOptions::default(), None, Some(&token)).await, Err(GitError::Cancelled)));
}

#[cfg(unix)]
#[tokio::test]
async fn cancel_kills_a_hanging_fetch() {
    let t = TempRepo::new().await;
    t.git(&["config", "protocol.ext.allow", "always"]);
    t.git(&["remote", "add", "slow", "ext::sh -c sleep% 30"]);
    let token = CancelToken::new();
    let canceller = token.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        canceller.cancel();
    });
    let started = std::time::Instant::now();
    let r = t.repo.fetch(&FetchOptions { remote: Some("slow".into()), ..Default::default() }, None, Some(&token)).await;
    assert!(matches!(r, Err(GitError::Cancelled)), "{r:?}");
    assert!(started.elapsed() < std::time::Duration::from_secs(10));
}

#[tokio::test]
async fn gutter_marks_against_head_content() {
    let t = TempRepo::new().await;
    t.commit_files(&[("f.cs", &numbered(10))], "init");
    let head = t.repo.show_file(&Revision::Head, "f.cs").await.unwrap().unwrap();
    let buffer = numbered(10).replace("line 2\n", "line 2 edited\n").replace("line 5\n", "").replace("line 8\n", "line 8\nnew\n");
    let lc = LineChanges::compute(&head.text(), &buffer);
    let marks = lc.marks(10);
    assert_eq!(marks[1], Some(GutterMark::Modified));
    assert_eq!(marks[3], Some(GutterMark::DeletedAfter));
    assert_eq!(marks[7], Some(GutterMark::Added));
    assert_eq!(lc.change_at(1).unwrap().original_text(&head.text()), "line 2\n");
}

#[test]
fn errors_have_translatable_messages() {
    let (template, args) = GitError::BranchExists("dev".into()).user_message();
    assert_eq!(template, "A branch named {branch} already exists");
    assert_eq!(args, vec![("branch", "dev".to_string())]);
    let tsv = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../scripts/i18n/rc-vcs.tsv")).unwrap();
    let errors = [
        GitError::GitNotFound,
        GitError::UnsupportedVersion { found: String::new(), required: String::new() },
        GitError::NotARepository(Default::default()),
        GitError::NothingToCommit,
        GitError::NothingToStash,
        GitError::Conflicts(String::new()),
        GitError::AuthenticationRequired(String::new()),
        GitError::DetachedHead,
        GitError::LocalChangesWouldBeOverwritten(vec![]),
        GitError::BranchExists(String::new()),
        GitError::BranchNotFullyMerged(String::new()),
        GitError::NoUpstream,
        GitError::PushRejected(String::new()),
        GitError::PatchFailed(String::new()),
        GitError::NotFound(String::new()),
        GitError::IdentityUnknown,
        GitError::Locked(String::new()),
        GitError::Network(String::new()),
        GitError::Cancelled,
        GitError::Parse(String::new()),
    ];
    for e in errors {
        let (template, _) = e.user_message();
        assert!(tsv.lines().any(|l| l.split('\t').next() == Some(template)), "missing translation of {template:?}");
    }
}
