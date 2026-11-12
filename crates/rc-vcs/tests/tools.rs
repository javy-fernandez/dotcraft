//! Repository tools use real Git repositories without remote network services.
mod common;
use common::TempRepo;
use rc_vcs::{Git, Repository};
use rc_vcs::tools::SubmoduleState;

#[cfg(unix)]
#[tokio::test]
async fn cancelling_or_dropping_git_stops_spawned_helpers() {
    use std::os::unix::fs::PermissionsExt;
    use std::time::Duration;
    for abort_task in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let executable = dir.path().join("fake-git");
        std::fs::write(&executable, "#!/bin/sh\nif [ \"$1\" = --version ]; then echo 'git version 2.45.1'; exit; fi\n(while [ ! -f release ]; do sleep 0.01; done; printf done > marker) &\nprintf ready > ready\nwait\n").unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
        let git = Git::open(executable).await.unwrap();
        let token = rc_vcs::CancelToken::new();
        let cancel = token.clone();
        let cwd = dir.path().to_owned();
        let task = tokio::spawn(async move { git.command(&cwd).arg("clone").cancel(Some(&cancel)).output().await });
        tokio::time::timeout(Duration::from_secs(3), async {
            while !dir.path().join("ready").exists() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        if abort_task {
            task.abort();
            assert!(task.await.unwrap_err().is_cancelled());
        } else {
            token.cancel();
            assert!(matches!(task.await.unwrap(), Err(rc_vcs::GitError::Cancelled)));
        }
        std::fs::write(dir.path().join("release"), "released").unwrap();
        tokio::time::sleep(Duration::from_millis(600)).await;
        assert!(!dir.path().join("marker").exists(), "A helper kept running after cancellation");
    }
}

#[tokio::test]
async fn clone_preserves_source_and_rejects_reusing_destinations() {
    let source = TempRepo::new().await;
    source.commit_files(&[("App.cs", "class App {}\n")], "source");
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("clone with spaces");
    Git::clone_repository(source.repo.root().to_str().unwrap(), &target).await.unwrap();
    let cloned = Repository::discover(&target).await.unwrap();
    assert_eq!(cloned.resolve("HEAD").await.unwrap(), source.repo.resolve("HEAD").await.unwrap());
    assert_eq!(std::fs::read_to_string(target.join("App.cs")).unwrap(), "class App {}\n");
    assert!(Git::clone_repository(source.repo.root().to_str().unwrap(), &target).await.is_err());
    assert!(Git::clone_repository("--upload-pack=evil", &dir.path().join("unsafe")).await.is_err());
    assert!(Git::clone_repository("ext::command", &dir.path().join("unsafe")).await.is_err());
    assert!(!dir.path().join("unsafe").exists());
}

#[tokio::test]
async fn cherry_pick_supports_success_conflict_continue_and_abort() {
    let t = TempRepo::new().await;
    t.commit_files(&[("App.cs", "original\n")], "base");
    let base = t.repo.resolve("HEAD").await.unwrap();
    t.git(&["checkout", "-qb", "topic"]);
    t.commit_files(&[("App.cs", "topic\n")], "topic");
    let topic = t.repo.resolve("HEAD").await.unwrap();
    t.git(&["checkout", "-q", "main"]);
    t.repo.cherry_pick_commit(&topic).await.unwrap();
    assert_eq!(std::fs::read_to_string(t.path("App.cs")).unwrap(), "topic\n");
    t.git(&["reset", "--hard", &base]);
    t.commit_files(&[("App.cs", "main\n")], "main");
    let main = t.repo.resolve("HEAD").await.unwrap();
    assert!(t.repo.cherry_pick_commit(&topic).await.is_err());
    assert_eq!(t.repo.state().await, rc_vcs::RepoState::CherryPicking);
    assert!(t.repo.cherry_pick_continue().await.is_err());
    t.repo.cherry_pick_abort().await.unwrap();
    assert_eq!(t.repo.resolve("HEAD").await.unwrap(), main);
    assert_eq!(std::fs::read_to_string(t.path("App.cs")).unwrap(), "main\n");
    assert!(t.repo.cherry_pick_commit(&topic).await.is_err());
    t.write("App.cs", "resolved\n");
    t.git(&["add", "App.cs"]);
    t.repo.cherry_pick_continue().await.unwrap();
    assert_eq!(t.repo.state().await, rc_vcs::RepoState::Clean);
    assert_eq!(std::fs::read_to_string(t.path("App.cs")).unwrap(), "resolved\n");
    t.write("dirty.txt", "keep\n");
    assert!(t.repo.cherry_pick_commit(&base).await.is_err());
    assert_eq!(std::fs::read_to_string(t.path("dirty.txt")).unwrap(), "keep\n");
}

#[tokio::test]
async fn worktrees_round_trip_and_refuse_removing_dirty_or_primary_worktrees() {
    let t = TempRepo::new().await;
    t.commit_files(&[("App.cs", "class App {}\n")], "base");
    let dir = tempfile::tempdir().unwrap();
    // Git reports canonical paths (/private/var on macOS).
    let path = dir.path().canonicalize().unwrap().join("linked worktree");
    t.repo.add_worktree(&path, "feature/tools").await.unwrap();
    let list = t.repo.worktrees().await.unwrap();
    assert_eq!(list.len(), 2);
    assert!(list.iter().any(|w| w.path == path && w.branch.as_deref() == Some("feature/tools")));
    std::fs::write(path.join("dirty.txt"), "keep").unwrap();
    assert!(t.repo.remove_worktree(&path).await.is_err());
    assert!(path.join("dirty.txt").exists());
    std::fs::remove_file(path.join("dirty.txt")).unwrap();
    assert!(t.repo.remove_worktree(t.repo.root()).await.is_err());
    t.repo.remove_worktree(&path).await.unwrap();
    assert!(!path.exists());
    assert_eq!(t.repo.worktrees().await.unwrap().len(), 1);
    assert!(t.repo.add_worktree(&dir.path().join("bad"), "--detach").await.is_err());
}

#[tokio::test]
async fn submodule_status_and_update_restore_recorded_commits() {
    let child = TempRepo::new().await;
    child.commit_files(&[("Library.cs", "class Library {}\n")], "library");
    let parent = TempRepo::new().await;
    parent.commit_files(&[("App.cs", "class App {}\n")], "app");
    parent.git(&["-c", "protocol.file.allow=always", "submodule", "add", child.repo.root().to_str().unwrap(), "modules/library"]);
    parent.git(&["commit", "-am", "submodule"]);
    let expected = child.repo.resolve("HEAD").await.unwrap();
    let module = Repository::discover(&parent.path("modules/library")).await.unwrap();
    common::configure(module.root());
    common::git(module.root(), &["commit", "--allow-empty", "-m", "local deviation"]);
    let before = parent.repo.submodules().await.unwrap();
    assert_eq!(before.len(), 1);
    assert_eq!(before[0].path, std::path::Path::new("modules/library"));
    assert_eq!(before[0].state, SubmoduleState::DifferentCommit);
    assert_eq!(before[0].commit, module.resolve("HEAD").await.unwrap());
    let cancelled = rc_vcs::CancelToken::new();
    cancelled.cancel();
    assert!(matches!(parent.repo.update_submodules_cancellable(Some(&cancelled)).await, Err(rc_vcs::GitError::Cancelled)));
    assert_eq!(module.resolve("HEAD").await.unwrap(), before[0].commit);
    parent.repo.update_submodules().await.unwrap();
    assert_eq!(module.resolve("HEAD").await.unwrap(), expected);
    let after = parent.repo.submodules().await.unwrap();
    assert_eq!(after[0].state, SubmoduleState::AtRecordedCommit);
    assert_eq!(after[0].commit, expected);
}

#[tokio::test]
async fn lfs_reports_missing_extension_or_tracks_an_explicit_pattern() {
    let t = TempRepo::new().await;
    t.commit_files(&[("App.cs", "class App {}\n")], "base");
    let available = t.repo.cmd().args(["lfs", "version"]).output().await.is_ok();
    assert!(t.repo.lfs_track("--help").await.is_err());
    if available {
        t.repo.lfs_track("*.bin").await.unwrap();
        assert!(std::fs::read_to_string(t.path(".gitattributes")).unwrap().contains("*.bin filter=lfs"));
        assert!(t.repo.lfs_status().await.unwrap().contains("git-lfs/"));
    } else {
        assert!(t.repo.lfs_status().await.is_err());
        assert!(t.repo.lfs_track("*.bin").await.is_err());
        assert!(t.repo.lfs_pull().await.is_err());
        assert!(!t.path(".gitattributes").exists());
    }
}

#[tokio::test]
#[ignore = "requires git-lfs on PATH; uses only local temporary repositories"]
async fn lfs_pull_downloads_local_objects_and_replaces_pointer_files() {
    let source = TempRepo::new().await;
    source.repo.cmd().args(["lfs", "version"]).output().await.expect("Install git-lfs on the test process PATH");
    // Filter configuration is local to this temporary repository; no hooks or
    // global configuration are installed, and no push or network service is used.
    source.git(&["lfs", "install", "--local", "--skip-repo", "--skip-smudge"]);
    source.repo.lfs_track("*.bin").await.unwrap();
    let payload = "DotCraft LFS payload\0\n".repeat(8192);
    source.commit_files(&[("asset.bin", &payload)], "LFS object");
    let pointer = source.git(&["show", "HEAD:asset.bin"]);
    assert!(pointer.starts_with("version https://git-lfs.github.com/spec/v1\n"));
    let oid = pointer.lines().find_map(|line| line.strip_prefix("oid sha256:")).unwrap();
    assert_eq!(source.read("asset.bin"), payload);

    let destination = tempfile::tempdir().unwrap();
    let checkout = destination.path().join("checkout");
    common::git(destination.path(), &["clone", "--no-checkout", "--", source.repo.root().to_str().unwrap(), checkout.to_str().unwrap()]);
    common::configure(&checkout);
    let source_path = source.repo.git_dir().to_string_lossy().replace('\\', "/");
    let local_url = format!("file:///{}", source_path.trim_start_matches('/'));
    common::git(&checkout, &["config", "lfs.url", &local_url]);
    common::git(&checkout, &["config", "remote.origin.lfsurl", &local_url]);
    common::git(&checkout, &["lfs", "install", "--local", "--skip-repo", "--skip-smudge"]);
    common::git(&checkout, &["checkout", "main"]);
    assert_eq!(std::fs::read_to_string(checkout.join("asset.bin")).unwrap(), pointer);
    let object = checkout.join(".git/lfs/objects").join(&oid[..2]).join(&oid[2..4]).join(oid);
    assert!(!object.exists(), "The clone must begin without the LFS object");

    let repo = Repository::discover(&checkout).await.unwrap();
    repo.lfs_pull().await.unwrap();
    assert_eq!(std::fs::read_to_string(checkout.join("asset.bin")).unwrap(), payload);
    assert_eq!(std::fs::read_to_string(object).unwrap(), payload);
    assert!(repo.status(&Default::default()).await.unwrap().entries.is_empty());
    assert!(repo.lfs_status().await.unwrap().contains("git-lfs/"));
}
