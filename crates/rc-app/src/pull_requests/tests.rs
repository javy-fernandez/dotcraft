//! The pull request manager and window against local mock servers (never the real services).

use super::*;
use egui_kittest::kittest::Queryable;
use rc_vcs::hosting::fake::{self, TOKEN};
use rc_vcs::hosting::mock::MockServer;
use std::time::{Duration, Instant};

fn manager(store: Arc<dyn SecretStore>) -> PullRequests {
    PullRequests::with_store(Waker::noop(), store)
}

/// Drains events until `done`; returns the outputs seen meanwhile.
fn pump(m: &mut PullRequests, what: &str, done: impl Fn(&PullRequests) -> bool) -> Vec<Output> {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut outs = Vec::new();
    loop {
        outs.extend(m.drain());
        if done(m) {
            return outs;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what} (error: {:?})", m.error);
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn notices(outs: &[Output]) -> Vec<String> {
    outs.iter().filter_map(|o| if let Output::Notice(n, _) = o { Some(n.clone()) } else { None }).collect()
}

#[test]
fn github_sign_in_review_merge_and_create() {
    let server = MockServer::start(fake::github);
    let store: Arc<dyn SecretStore> = Arc::new(MemoryStore::default());
    let mut m = manager(store.clone());
    m.use_repo_for_test(PathBuf::from("/repo"), fake::github_repo(&server.base));
    assert!(!m.connecting && !m.signed_in(), "no stored token: the sign-in form shows");

    // A wrong token is refused and not stored.
    m.token_input = "wrong".into();
    m.sign_in();
    pump(&mut m, "refusal", |m| !m.connecting);
    assert!(!m.signed_in() && m.error.is_some());
    assert_eq!(store.get("github:github.com"), None);

    m.token_input = TOKEN.into();
    m.sign_in();
    pump(&mut m, "list", |m| m.signed_in() && !m.loading && !m.list.is_empty());
    assert_eq!(store.get("github:github.com").as_deref(), Some(TOKEN));
    assert!(m.token_input.is_empty() && m.error.is_none());
    m.filter = "#41".into();
    assert_eq!(m.visible().iter().map(|p| p.id).collect::<Vec<_>>(), [41]);
    m.filter = "checkout".into();
    assert_eq!(m.visible().iter().map(|p| p.id).collect::<Vec<_>>(), [40]);
    m.filter.clear();

    m.select(42);
    pump(&mut m, "details", |m| m.details.is_some());
    let file = m.details.as_ref().unwrap().files[0].clone();
    m.open_diff(&file);
    let outs = pump_until_diff(&mut m);
    let Some(Output::Diff(title, path, left, right)) = outs.into_iter().find(|o| matches!(o, Output::Diff(..))) else { panic!("no diff") };
    assert_eq!((title.as_str(), path.as_str(), left.as_str(), right.as_str()), ("src/Shop.Core/PriceCalculator.cs (#42)", "src/Shop.Core/PriceCalculator.cs", fake::BEFORE, fake::AFTER));

    m.new_comment = "Nice work".into();
    m.add_comment();
    pump(&mut m, "comment", |m| m.busy.is_none());
    assert!(m.new_comment.is_empty());
    m.reply = Some(("line:7".into(), "Thanks!".into()));
    m.send_reply();
    pump(&mut m, "reply", |m| m.busy.is_none());
    assert!(m.reply.is_none());
    m.review(Review::Approve);
    let outs = pump(&mut m, "review", |m| m.busy.is_none());
    assert_eq!(notices(&outs), ["Approved #42"]);
    m.merge_method = MergeMethod::Squash;
    m.merge();
    let outs = pump(&mut m, "merge", |m| m.busy.is_none());
    assert_eq!(notices(&outs), ["Merged #42"]);

    // New pull request from a branch already on the remote.
    let branch = |name: &str, kind| rc_vcs::Branch { name: name.into(), full_ref: String::new(), kind, target: String::new(), is_head: false, upstream: None, ahead: 0, behind: 0, upstream_gone: false, commit_time: 0, subject: String::new() };
    let branches = [branch("feature/tidy-up", rc_vcs::BranchKind::Local), branch("main", rc_vcs::BranchKind::Local), branch("origin/main", rc_vcs::BranchKind::Remote), branch("origin/develop", rc_vcs::BranchKind::Remote)];
    m.open_create(Some("feature/tidy-up".into()), &branches);
    let f = m.create.as_ref().unwrap();
    assert_eq!((f.title.as_str(), f.target.as_str(), f.targets.clone(), f.unpushed), ("Tidy up", "main", vec!["main".to_string(), "develop".to_string()], true));
    let mut pushed = branches.to_vec();
    pushed.push(branch("origin/feature/tidy-up", rc_vcs::BranchKind::Remote));
    m.check_pushed(&pushed);
    assert!(!m.create.as_ref().unwrap().unpushed);
    m.create_pr();
    let outs = pump(&mut m, "create", |m| m.create.is_none());
    assert_eq!(notices(&outs), ["Created pull request #43"]);
    assert_eq!(m.selected, Some(43));
    let created = server.seen().into_iter().rfind(|r| r.method == "POST" && r.path == "/repos/acme/shop/pulls").unwrap();
    let body: serde_json::Value = serde_json::from_str(&created.body).unwrap();
    assert_eq!((body["head"].as_str(), body["base"].as_str(), body["title"].as_str()), (Some("feature/tidy-up"), Some("main"), Some("Tidy up")));

    m.sign_out().unwrap();
    assert_eq!(store.get("github:github.com"), None);
    assert!(!m.signed_in() && m.repo.is_some() && m.list.is_empty());
}

fn pump_until_diff(m: &mut PullRequests) -> Vec<Output> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let outs = m.drain();
        if outs.iter().any(|o| matches!(o, Output::Diff(..))) {
            return outs;
        }
        assert!(Instant::now() < deadline, "no diff");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn azure_stored_token_connects_without_asking() {
    let server = MockServer::start(fake::azure);
    let store: Arc<dyn SecretStore> = Arc::new(MemoryStore::default());
    store.set("azure:dev.azure.com/contoso", TOKEN).unwrap();
    let mut m = manager(store);
    m.use_repo_for_test(PathBuf::from("/repo"), fake::azure_repo(&server.base));
    assert!(m.connecting);
    pump(&mut m, "list", |m| m.signed_in() && !m.loading && !m.list.is_empty());
    assert_eq!(m.viewer.as_ref().unwrap().name, "Javier");
    m.select(7);
    pump(&mut m, "details", |m| m.details.is_some());
    assert_eq!(m.details.as_ref().unwrap().threads.len(), 2);
    let file = m.details.as_ref().unwrap().files[0].clone();
    m.open_diff(&file);
    let outs = pump_until_diff(&mut m);
    assert!(outs.iter().any(|o| matches!(o, Output::Diff(_, _, l, r) if l == fake::BEFORE && r == fake::AFTER)));
    // An added file has no left side and is never asked for.
    let added = m.details.as_ref().unwrap().files[1].clone();
    m.open_diff(&added);
    let outs = pump_until_diff(&mut m);
    assert!(outs.iter().any(|o| matches!(o, Output::Diff(_, _, l, _) if l.is_empty())));
    m.review(Review::Approve);
    let outs = pump(&mut m, "vote", |m| m.busy.is_none());
    assert_eq!(notices(&outs), ["Approved #7"]);
}

#[test]
fn window_lists_opens_and_approves() {
    let _home = crate::screens::HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = tempfile::tempdir().unwrap();
    let server = MockServer::start(fake::github);
    let mut app = crate::screens::test_app(home.path(), true);
    app.settings.roslyn_path = Some("/nonexistent".into());
    app.open_workspace(&crate::screens::fixtures().join("Shop/Shop.sln"));
    app.pull_requests.use_repo_for_test(PathBuf::from("/repo"), fake::github_repo(&server.base));
    app.pull_requests.token_input = TOKEN.into();
    app.dock.show(crate::toolwindow::ToolWindow::PullRequests);
    app.dock.sizes.insert(crate::toolwindow::ToolWindow::PullRequests, 520.0);
    let mut h = crate::screens::harness(app, egui::vec2(1280.0, 800.0));
    h.get_by_label("Sign In").click();
    settle(&mut h, "list", |a| !a.pull_requests.list.is_empty() && !a.pull_requests.loading);
    h.get_by_label("#42 Discounts in PriceCalculator").click();
    settle(&mut h, "details", |a| a.pull_requests.details.is_some());
    h.get_by_label("Approve").click();
    settle(&mut h, "review", |a| a.pull_requests.busy.is_none() && a.notices.iter().any(|n| n.text == "Approved #42"));
    h.get_by_label("PriceCalculator.cs").click();
    settle(&mut h, "diff", |a| a.vcs.diff.is_some());
    assert!(h.state().vcs.diff.as_ref().unwrap().title.starts_with("src/Shop.Core/PriceCalculator.cs"));
    assert!(server.seen().iter().any(|r| r.method == "POST" && r.path == "/repos/acme/shop/pulls/42/reviews"));
}

fn settle(h: &mut egui_kittest::Harness<'_, crate::app::DotCraftApp>, what: &str, done: impl Fn(&crate::app::DotCraftApp) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done(h.state()) {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        h.run_steps(2);
        std::thread::sleep(Duration::from_millis(5));
    }
    h.run_steps(2);
}
