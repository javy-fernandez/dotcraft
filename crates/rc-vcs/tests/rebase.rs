//! Interactive rebase against the real `git`: reorder, squash, fixup, reword, drop, edit, and
//! the conflict pause with continue, skip and abort.

mod common;

use common::TempRepo;
use rc_vcs::*;

/// A repository with `init` plus commits c1..c4, each adding its own file.
async fn four_commits() -> TempRepo {
    let t = TempRepo::new().await;
    t.commit_files(&[("base.txt", "base\n")], "init");
    for i in 1..=4 {
        t.commit_files(&[(&format!("f{i}.txt"), &format!("{i}\n"))], &format!("c{i}"));
    }
    t
}

fn subjects(t: &TempRepo) -> Vec<String> {
    t.git(&["log", "--format=%s"]).lines().map(str::to_string).collect()
}

fn hash_of(t: &TempRepo, subject: &str) -> String {
    t.git(&["log", "--format=%H", &format!("--grep=^{subject}$")]).trim().to_string()
}

async fn plan_from(t: &TempRepo, subject: &str) -> RebasePlan {
    t.repo.interactive_rebase_plan(&hash_of(t, subject)).await.unwrap()
}

fn set(plan: &mut RebasePlan, subject: &str, action: RebaseAction, message: Option<&str>) {
    let s = plan.steps.iter_mut().find(|s| s.commit.subject == subject).unwrap();
    s.action = action;
    s.message = message.map(str::to_string);
}

#[tokio::test]
async fn plan_lists_commits_oldest_first() {
    let t = four_commits().await;
    let plan = plan_from(&t, "c2").await;
    assert_eq!(plan.steps.iter().map(|s| s.commit.subject.as_str()).collect::<Vec<_>>(), ["c2", "c3", "c4"]);
    assert!(plan.base.is_some());
    let root = plan_from(&t, "init").await;
    assert!(root.base.is_none());
    assert_eq!(root.steps.len(), 5);
    let original: Vec<String> = plan.steps.iter().map(|s| s.commit.hash.clone()).collect();
    assert!(plan.is_unchanged(&original));
}

#[tokio::test]
async fn reorders_and_drops() {
    let t = four_commits().await;
    let mut plan = plan_from(&t, "c1").await;
    plan.steps.swap(0, 2); // c3 c2 c1 c4
    set(&mut plan, "c2", RebaseAction::Drop, None);
    assert_eq!(t.repo.interactive_rebase(&plan).await.unwrap(), RebaseOutcome::Done);
    assert_eq!(subjects(&t), ["c4", "c1", "c3", "init"]);
    assert!(!t.path("f2.txt").exists());
    assert_eq!(t.repo.state().await, RepoState::Clean);
}

#[tokio::test]
async fn rebases_from_the_root_commit() {
    let t = four_commits().await;
    let mut plan = plan_from(&t, "init").await;
    set(&mut plan, "init", RebaseAction::Reword, Some("Start"));
    set(&mut plan, "c1", RebaseAction::Fixup, None);
    t.repo.interactive_rebase(&plan).await.unwrap();
    assert_eq!(subjects(&t), ["c4", "c3", "c2", "Start"]);
}

#[tokio::test]
async fn squashes_fixes_up_and_rewords() {
    let t = four_commits().await;
    let mut plan = plan_from(&t, "c1").await;
    set(&mut plan, "c2", RebaseAction::Squash, None);
    set(&mut plan, "c3", RebaseAction::Reword, Some("Third, reworded\n\nWith a body"));
    set(&mut plan, "c4", RebaseAction::Fixup, None);
    t.repo.interactive_rebase(&plan).await.unwrap();
    assert_eq!(subjects(&t), ["Third, reworded", "c1", "init"]);
    // Squash keeps both messages; fixup keeps only the target's.
    assert_eq!(t.git(&["log", "-1", "--format=%B", "HEAD~1"]).trim(), "c1\n\nc2");
    assert_eq!(t.git(&["log", "-1", "--format=%B", "HEAD"]).trim(), "Third, reworded\n\nWith a body");
    assert_eq!(t.git(&["show", "--format=", "--name-only", "HEAD"]).split_whitespace().collect::<Vec<_>>(), ["f3.txt", "f4.txt"]);
}

#[tokio::test]
async fn reword_of_a_squash_target_sets_the_group_message() {
    let t = four_commits().await;
    let mut plan = plan_from(&t, "c3").await;
    set(&mut plan, "c3", RebaseAction::Reword, Some("Both"));
    set(&mut plan, "c4", RebaseAction::Squash, None);
    t.repo.interactive_rebase(&plan).await.unwrap();
    assert_eq!(subjects(&t), ["Both", "c2", "c1", "init"]);
    assert_eq!(t.git(&["log", "-1", "--format=%B"]).trim(), "Both");
}

#[tokio::test]
async fn invalid_plans_and_merges_are_refused() {
    let t = four_commits().await;
    let mut plan = plan_from(&t, "c3").await;
    set(&mut plan, "c3", RebaseAction::Fixup, None);
    assert!(matches!(t.repo.interactive_rebase(&plan).await, Err(GitError::InvalidRebasePlan(_))));
    assert_eq!(t.repo.state().await, RepoState::Clean);

    t.git(&["checkout", "-q", "-b", "side", "HEAD~2"]);
    t.commit_files(&[("side.txt", "s\n")], "side");
    t.git(&["checkout", "-q", "main"]);
    t.git(&["merge", "-q", "--no-ff", "-m", "merge side", "side"]);
    assert!(matches!(t.repo.interactive_rebase_plan(&hash_of(&t, "c1")).await, Err(GitError::RebaseMergeCommits)));
    // A commit that is not in HEAD cannot start a rebase.
    t.git(&["checkout", "-q", "-b", "other", "HEAD~1"]);
    t.git(&["checkout", "-q", "main"]);
    t.git(&["checkout", "-q", "side"]);
    assert!(matches!(t.repo.interactive_rebase_plan(&hash_of(&t, "c4")).await, Err(GitError::NotFound(_))));
}

/// c1 and c2 both rewrite `a.txt`: moving c2 before c1 conflicts.
async fn conflicting() -> (TempRepo, RebasePlan) {
    let t = TempRepo::new().await;
    t.commit_files(&[("a.txt", "base\n")], "init");
    t.commit_files(&[("a.txt", "one\n")], "c1");
    t.commit_files(&[("a.txt", "two\n")], "c2");
    t.commit_files(&[("b.txt", "b\n")], "c3");
    let mut plan = plan_from(&t, "c1").await;
    plan.steps.swap(0, 1);
    (t, plan)
}

#[tokio::test]
async fn conflict_pauses_then_continues() {
    let (t, plan) = conflicting().await;
    let RebaseOutcome::Stopped(st) = t.repo.interactive_rebase(&plan).await.unwrap() else { panic!("expected a stop") };
    assert_eq!(st.stop, RebaseStop::Conflicts);
    assert_eq!(st.conflicts, ["a.txt"]);
    assert_eq!(st.commit.as_ref().map(|c| c.1.as_str()), Some("c2"));
    assert_eq!((st.step, st.total), (1, 3));
    assert_eq!(st.branch.as_deref(), Some("main"));
    assert_eq!(t.repo.state().await, RepoState::Rebasing);
    assert_eq!(t.repo.rebase_status().await.unwrap(), Some(st));
    // Continue refuses while files conflict.
    assert!(matches!(t.repo.continue_rebase().await, Err(GitError::Conflicts(_))));

    // Resolve with "two", continue: c1 then conflicts again (two -> one).
    t.write("a.txt", "two\n");
    t.repo.mark_resolved(&["a.txt".into()]).await.unwrap();
    let RebaseOutcome::Stopped(st) = t.repo.continue_rebase().await.unwrap() else { panic!("expected a second stop") };
    assert_eq!(st.commit.as_ref().map(|c| c.1.as_str()), Some("c1"));
    assert_eq!(st.step, 2);
    t.repo.resolve_with(&["a.txt".into()], true).await.unwrap();
    assert_eq!(t.repo.continue_rebase().await.unwrap(), RebaseOutcome::Done);
    assert_eq!(subjects(&t), ["c3", "c1", "c2", "init"]);
    assert_eq!(t.read("a.txt"), "one\n");
    assert_eq!(t.repo.rebase_status().await.unwrap(), None);
}

#[tokio::test]
async fn conflict_can_be_aborted_or_skipped() {
    let (t, plan) = conflicting().await;
    let before = t.git(&["rev-parse", "HEAD"]);
    assert!(matches!(t.repo.interactive_rebase(&plan).await.unwrap(), RebaseOutcome::Stopped(_)));
    t.repo.abort_rebase().await.unwrap();
    assert_eq!(t.repo.state().await, RepoState::Clean);
    assert_eq!(t.git(&["rev-parse", "HEAD"]), before);
    assert_eq!(t.read("a.txt"), "two\n");

    // Skipping the moved c2 leaves c1 (which then applies cleanly) and c3.
    assert!(matches!(t.repo.interactive_rebase(&plan).await.unwrap(), RebaseOutcome::Stopped(_)));
    assert_eq!(t.repo.skip_rebase().await.unwrap(), RebaseOutcome::Done);
    assert_eq!(subjects(&t), ["c3", "c1", "init"]);
}

#[tokio::test]
async fn reword_survives_a_conflict_pause() {
    let (t, mut plan) = conflicting().await;
    set(&mut plan, "c2", RebaseAction::Reword, Some("Second first"));
    let RebaseOutcome::Stopped(_) = t.repo.interactive_rebase(&plan).await.unwrap() else { panic!("expected a stop") };
    t.repo.resolve_with(&["a.txt".into()], true).await.unwrap();
    let RebaseOutcome::Stopped(_) = t.repo.continue_rebase().await.unwrap() else { panic!("expected a second stop") };
    t.repo.resolve_with(&["a.txt".into()], true).await.unwrap();
    assert_eq!(t.repo.continue_rebase().await.unwrap(), RebaseOutcome::Done);
    assert_eq!(subjects(&t), ["c3", "c1", "Second first", "init"]);
}

#[tokio::test]
async fn edit_stops_for_amending() {
    let t = four_commits().await;
    let mut plan = plan_from(&t, "c2").await;
    set(&mut plan, "c2", RebaseAction::Edit, None);
    let RebaseOutcome::Stopped(st) = t.repo.interactive_rebase(&plan).await.unwrap() else { panic!("expected a stop") };
    assert_eq!(st.stop, RebaseStop::Edit);
    assert_eq!(st.commit.as_ref().map(|c| c.1.as_str()), Some("c2"));
    t.write("f2.txt", "2 amended\n");
    t.repo.stage(&["f2.txt".into()]).await.unwrap();
    t.repo.commit(&CommitOptions { message: "c2 amended".into(), amend: true, ..Default::default() }).await.unwrap();
    assert_eq!(t.repo.continue_rebase().await.unwrap(), RebaseOutcome::Done);
    assert_eq!(subjects(&t), ["c4", "c3", "c2 amended", "c1", "init"]);
    assert_eq!(t.read("f2.txt"), "2 amended\n");
}
