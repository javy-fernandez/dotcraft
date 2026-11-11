//! Branch management against the real `git`: names, merge, rebase, compare, the conflict
//! guard, pushing branches and remote branches.

mod common;

use common::{TempRepo, git};
use rc_vcs::*;

#[tokio::test]
async fn branch_names_are_validated() {
    let t = TempRepo::new().await;
    t.repo.check_branch_name("feature/ok-1").await.unwrap();
    for bad in ["", "-x", "a..b", "a b", "a~1", "end.lock", "x:y"] {
        assert!(matches!(t.repo.check_branch_name(bad).await, Err(GitError::InvalidBranchName(_))), "{bad:?}");
    }
}

#[tokio::test]
async fn merge_rebase_and_compare_branches() {
    let t = TempRepo::new().await;
    t.commit_files(&[("a.txt", "a\n")], "init");
    t.repo.create_branch("topic", None, true).await.unwrap();
    t.commit_files(&[("t.txt", "t\n")], "topic work");
    t.repo.checkout_branch("main").await.unwrap();
    t.commit_files(&[("m.txt", "m\n")], "main work");

    // Compare: one commit on each side.
    let only_topic = t.repo.commits_between("main", "topic", 50).await.unwrap();
    let only_main = t.repo.commits_between("topic", "main", 50).await.unwrap();
    assert_eq!(only_topic.iter().map(|c| c.subject.as_str()).collect::<Vec<_>>(), vec!["topic work"]);
    assert_eq!(only_main.iter().map(|c| c.subject.as_str()).collect::<Vec<_>>(), vec!["main work"]);

    // Rebase topic onto main: topic then contains main's commit.
    t.repo.checkout_branch("topic").await.unwrap();
    t.repo.rebase("main").await.unwrap();
    assert!(t.repo.commits_between("topic", "main", 50).await.unwrap().is_empty());
    assert!(t.path("m.txt").exists());

    // Merge topic into main (fast-forward).
    t.repo.checkout_branch("main").await.unwrap();
    t.repo.merge("topic", false).await.unwrap();
    assert!(t.path("t.txt").exists());
    t.repo.delete_branch("topic", false).await.unwrap();
}

#[tokio::test]
async fn conflicting_rebase_is_guarded_and_aborted() {
    let t = TempRepo::new().await;
    t.commit_files(&[("a.txt", "base\n")], "init");
    t.repo.create_branch("topic", None, true).await.unwrap();
    t.commit_files(&[("a.txt", "topic\n")], "topic");
    t.repo.checkout_branch("main").await.unwrap();
    t.commit_files(&[("a.txt", "main\n")], "main");
    t.repo.ensure_no_conflicts().await.unwrap();

    t.repo.checkout_branch("topic").await.unwrap();
    assert!(matches!(t.repo.rebase("main").await, Err(GitError::Conflicts(_))));
    assert_eq!(t.repo.state().await, RepoState::Rebasing);
    assert!(matches!(t.repo.ensure_no_conflicts().await, Err(GitError::Conflicts(_))));
    t.repo.abort_rebase().await.unwrap();
    assert_eq!(t.repo.state().await, RepoState::Clean);
    assert_eq!(t.repo.current_branch().await.unwrap().as_deref(), Some("topic"));
    assert_eq!(t.read("a.txt"), "topic\n");

    // A conflicting merge is guarded the same way.
    assert!(matches!(t.repo.merge("main", false).await, Err(GitError::Conflicts(_))));
    assert!(matches!(t.repo.ensure_no_conflicts().await, Err(GitError::Conflicts(_))));
    t.repo.abort_merge().await.unwrap();
    t.repo.ensure_no_conflicts().await.unwrap();
}

#[tokio::test]
async fn push_checkout_and_delete_remote_branches() {
    let remote_dir = tempfile::tempdir().unwrap();
    let remote_root = remote_dir.path().canonicalize().unwrap();
    git(&remote_root, &["init", "-q", "--bare", "-b", "main"]);
    let url = format!("file://{}", remote_root.display());

    let a = TempRepo::new().await;
    a.commit_files(&[("a.txt", "a\n")], "init");
    assert!(matches!(a.repo.push_branch("main", None, None).await, Err(GitError::NoRemote)));
    a.git(&["remote", "add", "origin", &url]);

    // First push sets the upstream; the next one pushes to it.
    assert_eq!(a.repo.push_branch("main", None, None).await.unwrap(), "origin");
    a.repo.create_branch("feature", None, false).await.unwrap();
    a.repo.push_branch("feature", None, None).await.unwrap();
    let feature = a.repo.branches().await.unwrap().into_iter().find(|b| b.name == "feature").unwrap();
    assert_eq!(feature.upstream.as_deref(), Some("origin/feature"));
    a.repo.checkout_branch("feature").await.unwrap();
    a.commit_files(&[("f.txt", "f\n")], "feature work");
    a.repo.checkout_branch("main").await.unwrap();
    a.repo.push_branch("feature", None, None).await.unwrap();
    assert_eq!(git(&remote_root, &["log", "-1", "--format=%s", "feature"]).trim(), "feature work");

    // A second clone checks the remote branch out as a tracking branch, twice.
    let b_dir = tempfile::tempdir().unwrap();
    let b_root = b_dir.path().canonicalize().unwrap();
    git(&b_root, &["clone", "-q", &url, "."]);
    common::configure(&b_root);
    let b = Repository::discover(&b_root).await.unwrap();
    b.checkout_tracking("origin/feature").await.unwrap();
    assert_eq!(b.current_branch().await.unwrap().as_deref(), Some("feature"));
    b.checkout_branch("main").await.unwrap();
    b.checkout_tracking("origin/feature").await.unwrap();
    assert_eq!(b.current_branch().await.unwrap().as_deref(), Some("feature"));
    b.checkout_branch("main").await.unwrap();
    // A local branch with the same name tracking something else is not reused.
    b.create_branch("other", None, false).await.unwrap();
    a.repo.create_branch("other", None, false).await.unwrap();
    a.repo.push_branch("other", None, None).await.unwrap();
    b.fetch(&FetchOptions { prune: true, ..Default::default() }, None, None).await.unwrap();
    assert!(matches!(b.checkout_tracking("origin/other").await, Err(GitError::BranchExists(n)) if n == "other"));

    // Deleting on the remote removes the remote-tracking branch too; the other clone sees it
    // gone after a fetch with prune.
    a.repo.delete_remote_branch("origin/feature", None).await.unwrap();
    assert!(!a.repo.branches().await.unwrap().iter().any(|br| br.name == "origin/feature"));
    assert!(git(&remote_root, &["branch", "--list", "feature"]).trim().is_empty());
    b.fetch(&FetchOptions { prune: true, ..Default::default() }, None, None).await.unwrap();
    let branches = b.branches().await.unwrap();
    assert!(!branches.iter().any(|br| br.name == "origin/feature"));
    assert!(branches.iter().find(|br| br.name == "feature").unwrap().upstream_gone);
}
