//! The merge tool's Git side: three-way models from the conflict stages, writing the result,
//! and continuing or aborting merges, rebases and cherry-picks.

mod common;

use common::TempRepo;
use rc_vcs::{GitError, MergeModel, RepoState, Side};

const BASE: &str = "class Shop\n{\n    int a = 1;\n\n    int b = 2;\n\n    int c = 3;\n}\n";

/// `main` and `topic` both change line `b`; `topic` also changes `a`, `main` changes `c`.
async fn conflicting(t: &TempRepo) {
    t.commit_files(&[("Shop.cs", BASE), ("other.txt", "x\n")], "base");
    t.git(&["switch", "-q", "-c", "topic"]);
    t.commit_files(&[("Shop.cs", &BASE.replace("a = 1", "a = 10").replace("b = 2", "b = 20"))], "topic");
    t.git(&["switch", "-q", "main"]);
    t.commit_files(&[("Shop.cs", &BASE.replace("c = 3", "c = 30").replace("b = 2", "b = 200"))], "main");
}

async fn model(t: &TempRepo) -> MergeModel {
    let v = t.repo.conflict_versions("Shop.cs").await.unwrap();
    MergeModel::new(&v.base.unwrap().text(), &v.ours.unwrap().text(), &v.theirs.unwrap().text())
}

#[tokio::test]
async fn merge_is_resolved_from_the_model_and_continued() {
    let t = TempRepo::new().await;
    conflicting(&t).await;
    assert!(matches!(t.repo.merge("topic", false).await, Err(GitError::Conflicts(_))));
    assert_eq!(t.repo.conflict_sides().await, ("main".to_string(), "topic".to_string()));
    let mut m = model(&t).await;
    assert_eq!((m.chunks.len(), m.unresolved_conflicts()), (3, 1));
    m.apply_non_conflicting(None);
    let conflict = (0..m.chunks.len()).find(|i| !m.is_resolved(*i)).unwrap();
    m.accept(conflict, Side::Theirs);
    let text = m.result_text();
    assert_eq!(text, BASE.replace("a = 1", "a = 10").replace("b = 2", "b = 20").replace("c = 3", "c = 30"));

    // Continuing with conflicts left is refused.
    assert!(matches!(t.repo.continue_operation().await, Err(GitError::Conflicts(_))));
    assert!(t.repo.resolve_with_text("other.txt", "nope").await.is_err());
    t.repo.resolve_with_text("Shop.cs", &text).await.unwrap();
    assert!(t.repo.conflicted_paths().await.unwrap().is_empty());
    t.repo.continue_operation().await.unwrap();
    assert_eq!(t.repo.state().await, RepoState::Clean);
    assert_eq!(t.read("Shop.cs"), text);
    assert_eq!(t.git(&["log", "-1", "--format=%P"]).split_whitespace().count(), 2);
}

#[tokio::test]
async fn rebase_and_cherry_pick_continue_or_abort() {
    let t = TempRepo::new().await;
    conflicting(&t).await;
    t.git(&["switch", "-q", "topic"]);
    assert!(matches!(t.repo.rebase("main").await, Err(GitError::Conflicts(_))));
    assert_eq!(t.repo.state().await, RepoState::Rebasing);
    // Ours is the branch rebased onto, theirs the rebased branch.
    assert_eq!(t.repo.conflict_sides().await, ("main".to_string(), "topic".to_string()));
    let mut m = model(&t).await;
    for i in 0..m.chunks.len() {
        m.accept(i, Side::Ours);
    }
    t.repo.resolve_with_text("Shop.cs", &m.result_text()).await.unwrap();
    t.repo.continue_operation().await.unwrap();
    assert_eq!(t.repo.state().await, RepoState::Clean);
    assert_eq!(t.repo.current_branch().await.unwrap().as_deref(), Some("topic"));

    // A conflicting cherry-pick, aborted.
    t.git(&["switch", "-q", "-c", "pick", "main~1"]);
    t.commit_files(&[("Shop.cs", &BASE.replace("b = 2", "b = 7"))], "pick me");
    let pick = t.git(&["rev-parse", "HEAD"]).trim().to_string();
    t.git(&["switch", "-q", "main"]);
    assert!(t.repo.cherry_pick_commit(&pick).await.is_err());
    assert_eq!(t.repo.state().await, RepoState::CherryPicking);
    assert_eq!(t.repo.conflict_sides().await.0, "main");
    t.repo.abort_operation().await.unwrap();
    assert_eq!(t.repo.state().await, RepoState::Clean);
    assert!(t.read("Shop.cs").contains("b = 200;"));
    // Nothing in progress: both are no-ops.
    t.repo.continue_operation().await.unwrap();
    t.repo.abort_operation().await.unwrap();
}
