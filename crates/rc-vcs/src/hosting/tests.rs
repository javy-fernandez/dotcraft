use super::fake::{self, TOKEN};
use super::mock::MockServer;
use super::secrets::{MemoryStore, SecretStore};
use super::*;

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap()
}

#[test]
fn parses_github_remotes() {
    for url in ["https://github.com/acme/shop.git", "https://user@github.com/acme/shop", "git@github.com:acme/shop.git", "ssh://git@github.com/acme/shop.git", "ssh://git@ssh.github.com:443/acme/shop.git"] {
        let r = HostRepo::parse(url).unwrap_or_else(|| panic!("{url}"));
        assert_eq!((r.kind, r.owner.as_str(), r.name.as_str(), r.api.as_str(), r.host.as_str()), (HostKind::GitHub, "acme", "shop", "https://api.github.com", "github.com"), "{url}");
        assert_eq!(r.display_name(), "acme/shop");
        assert_eq!(r.account(), "github:github.com");
    }
    let e = HostRepo::parse("git@github.example.com:team/app.git").unwrap();
    assert_eq!((e.api.as_str(), e.web.as_str()), ("https://github.example.com/api/v3", "https://github.example.com"));
    assert_eq!(HostRepo::parse("https://gitlab.com/acme/shop.git"), None);
    assert_eq!(HostRepo::parse("/srv/repos/shop.git"), None);
    assert_eq!(HostRepo::parse("C:/repos/shop"), None);
    assert_eq!(HostRepo::parse("https://github.com/acme"), None);
}

#[test]
fn parses_azure_devops_remotes() {
    for url in [
        "https://contoso@dev.azure.com/contoso/My%20Shop/_git/Shop.Web",
        "https://dev.azure.com/contoso/My%20Shop/_git/Shop.Web",
        "git@ssh.dev.azure.com:v3/contoso/My%20Shop/Shop.Web",
        "https://contoso.visualstudio.com/My%20Shop/_git/Shop.Web",
        "https://contoso.visualstudio.com/DefaultCollection/My%20Shop/_git/Shop.Web",
        "contoso@vs-ssh.visualstudio.com:v3/contoso/My%20Shop/Shop.Web",
    ] {
        let r = HostRepo::parse(url).unwrap_or_else(|| panic!("{url}"));
        assert_eq!((r.kind, r.owner.as_str(), r.project.as_str(), r.name.as_str()), (HostKind::AzureDevOps, "contoso", "My Shop", "Shop.Web"), "{url}");
        assert_eq!(r.api, "https://dev.azure.com/contoso");
        assert_eq!(r.account(), "azure:dev.azure.com/contoso");
        assert_eq!(r.pulls_page(), "https://dev.azure.com/contoso/My%20Shop/_git/Shop.Web/pullrequests");
    }
    // A repository named like its project.
    let r = HostRepo::parse("https://dev.azure.com/contoso/_git/Tools").unwrap();
    assert_eq!((r.project.as_str(), r.display_name().as_str()), ("Tools", "contoso/Tools"));
}

#[test]
fn encodes_urls() {
    assert_eq!(enc("My Shop/ä"), "My%20Shop%2F%C3%A4");
    assert_eq!(enc_path("src/My File.cs"), "src/My%20File.cs");
    assert_eq!(percent_decode("My%20Shop"), "My Shop");
}

#[test]
fn memory_store_round_trip() {
    let s = MemoryStore::default();
    assert_eq!(s.get("github:github.com"), None);
    s.set("github:github.com", "abc").unwrap();
    assert_eq!(s.get("github:github.com").as_deref(), Some("abc"));
    s.delete("github:github.com").unwrap();
    s.delete("github:github.com").unwrap();
    assert_eq!(s.get("github:github.com"), None);
}

#[test]
fn github_client_against_a_mock_server() {
    let server = MockServer::start(fake::github);
    let c = HostClient::new(fake::github_repo(&server.base), TOKEN).unwrap();
    rt().block_on(async {
        let me = c.viewer().await.unwrap();
        assert_eq!((me.id.as_str(), me.name.as_str(), me.default_branch.as_str(), me.can_push), ("octo", "Octo Cat", "main", true));
        let list = c.list().await.unwrap();
        assert_eq!(list.iter().map(|p| p.id).collect::<Vec<_>>(), [42, 41, 40]);
        assert_eq!(list[2].state, PrState::Draft);
        assert_eq!((list[0].source_branch.as_str(), list[0].target_branch.as_str(), list[0].author.as_str()), ("feature/discounts", "main", "mona"));

        let d = c.details(42, &me).await.unwrap();
        assert_eq!(d.files.len(), 3);
        assert_eq!(d.files[2].change, FileChange::Renamed);
        assert_eq!(d.files[2].old_path.as_deref(), Some("src/Shop.Core/Prices.cs"));
        assert_eq!((d.base_commit.as_str(), d.head_commit.as_str()), ("base42", "head42"));
        // Approved by hubot; octo only commented (and was a requested reviewer).
        assert!(d.pr.reviewers.contains(&Reviewer { name: "hubot".into(), vote: Vote::Approved }));
        assert!(d.pr.reviewers.contains(&Reviewer { name: "octo".into(), vote: Vote::Commented }));
        // Conversation comment, review comment thread with its reply, review summary.
        assert_eq!(d.threads.len(), 3);
        let line = d.threads.iter().find(|t| t.path.is_some()).unwrap();
        assert_eq!((line.id.as_str(), line.line, line.comments.len()), ("line:7", Some(8), 2));
        assert_eq!(d.permissions, Permissions { can_review: true, can_merge: true });

        assert_eq!(c.file_text("src/Shop.Core/PriceCalculator.cs", &d.base_commit).await.unwrap().as_deref(), Some(fake::BEFORE));
        assert_eq!(c.file_text("src/Shop.Core/PriceCalculator.cs", &d.head_commit).await.unwrap().as_deref(), Some(fake::AFTER));
        assert_eq!(c.file_text("tests/Shop.Core.Tests/DiscountTests.cs", &d.base_commit).await.unwrap(), None);

        c.comment(42, "Nice").await.unwrap();
        c.reply(42, line, "Thanks").await.unwrap();
        let general = d.threads.iter().find(|t| t.id.starts_with("issue:")).unwrap();
        c.reply(42, general, "Agreed").await.unwrap();
        c.review(42, Review::Approve, "", &me).await.unwrap();
        c.review(42, Review::RequestChanges, "Please add tests", &me).await.unwrap();
        c.merge(&d.pr, MergeMethod::Squash).await.unwrap();
        let err = c.merge(&list[1], MergeMethod::Merge).await.unwrap_err();
        assert_eq!(err, HostError::Rejected("Pull Request is not mergeable".into()));
        let created = c.create(&NewPullRequest { title: "Tidy up".into(), description: "Body".into(), source: "feature/tidy".into(), target: "main".into(), draft: true }).await.unwrap();
        assert_eq!((created.id, created.title.as_str(), created.source_branch.as_str(), created.state), (43, "Tidy up", "feature/tidy", PrState::Draft));
    });
    let seen = server.seen();
    assert!(seen.iter().all(|r| r.authorization.as_deref() == Some(&*format!("Bearer {TOKEN}"))));
    let body = |m: &str, p: &str| -> serde_json::Value { serde_json::from_str(&seen.iter().rfind(|r| r.method == m && r.path == p).unwrap().body).unwrap() };
    assert_eq!(body("POST", "/repos/acme/shop/issues/42/comments")["body"], "> @mona wrote:\n> Ready for review.\n\nAgreed");
    assert_eq!(body("POST", "/repos/acme/shop/pulls/42/comments/7/replies")["body"], "Thanks");
    assert_eq!(body("POST", "/repos/acme/shop/pulls/42/reviews"), serde_json::json!({ "event": "REQUEST_CHANGES", "body": "Please add tests" }));
    assert_eq!(body("PUT", "/repos/acme/shop/pulls/42/merge"), serde_json::json!({ "merge_method": "squash", "sha": "head42" }));
    assert_eq!(body("POST", "/repos/acme/shop/pulls"), serde_json::json!({ "title": "Tidy up", "body": "Body", "head": "feature/tidy", "base": "main", "draft": true }));
}

#[test]
fn github_bad_token_is_unauthorized_and_never_echoed() {
    let server = MockServer::start(|r| (401, format!(r#"{{"message":"Bad credentials for {}"}}"#, r.authorization.clone().unwrap_or_default())));
    let c = HostClient::new(fake::github_repo(&server.base), "secret-value").unwrap();
    let err = rt().block_on(c.viewer()).unwrap_err();
    assert!(matches!(&err, HostError::Unauthorized(m) if !m.contains("secret-value") && m.contains("***")), "{err:?}");
    assert!(!format!("{c:?}").contains("secret-value"));
}

#[test]
fn azure_client_against_a_mock_server() {
    let server = MockServer::start(fake::azure);
    let c = HostClient::new(fake::azure_repo(&server.base), TOKEN).unwrap();
    rt().block_on(async {
        let me = c.viewer().await.unwrap();
        assert_eq!((me.id.as_str(), me.name.as_str(), me.default_branch.as_str()), (fake::AZURE_ME, "Javier", "main"));
        let list = c.list().await.unwrap();
        assert_eq!(list.iter().map(|p| (p.id, p.state)).collect::<Vec<_>>(), [(7, PrState::Open), (6, PrState::Draft)]);
        assert_eq!(list[0].source_branch, "feature/settings");
        assert_eq!(list[0].web_url, "https://dev.azure.com/contoso/Shop/_git/Shop.Web/pullrequest/7");
        assert_eq!(list[0].reviewers[0], Reviewer { name: "Luis Pérez".into(), vote: Vote::ChangesRequested });

        let d = c.details(7, &me).await.unwrap();
        // Folders are skipped; paths lose their leading slash.
        assert_eq!(d.files.iter().map(|f| (f.path.as_str(), f.change)).collect::<Vec<_>>(), [("src/Shop.Core/PriceCalculator.cs", FileChange::Modified), ("src/Shop.Web/Pages/Settings.razor", FileChange::Added), ("src/Shop.Web/Layout.cs", FileChange::Renamed)]);
        assert_eq!(d.files[2].old_path.as_deref(), Some("src/Shop.Web/OldLayout.cs"));
        assert_eq!((d.base_commit.as_str(), d.head_commit.as_str()), ("base7", "src7"));
        // System threads are hidden.
        assert_eq!(d.threads.len(), 2);
        assert_eq!(d.threads[0].comments.len(), 2);
        assert_eq!((d.threads[1].path.as_deref(), d.threads[1].line, d.threads[1].resolved), (Some("src/Shop.Core/PriceCalculator.cs"), Some(6), true));
        assert!(d.permissions.can_merge && d.permissions.can_review);

        assert_eq!(c.file_text("src/Shop.Core/PriceCalculator.cs", "base7").await.unwrap().as_deref(), Some(fake::BEFORE));
        assert_eq!(c.file_text("src/Shop.Core/PriceCalculator.cs", "src7").await.unwrap().as_deref(), Some(fake::AFTER));
        assert_eq!(c.file_text("src/Shop.Web/Pages/Settings.razor", "base7").await.unwrap(), None);

        c.comment(7, "Looks good").await.unwrap();
        c.reply(7, &d.threads[0], "Thanks").await.unwrap();
        c.review(7, Review::Approve, "", &me).await.unwrap();
        c.merge(&d.pr, MergeMethod::Squash).await.unwrap();
        let created = c.create(&NewPullRequest { title: "Fix typo".into(), description: String::new(), source: "fix/typo".into(), target: "main".into(), draft: false }).await.unwrap();
        assert_eq!((created.id, created.source_branch.as_str()), (8, "fix/typo"));
    });
    let seen = server.seen();
    let body = |m: &str, p: &str| -> serde_json::Value { serde_json::from_str(&seen.iter().rfind(|r| r.method == m && r.path.starts_with(p)).unwrap().body).unwrap() };
    let repo = "/contoso/Shop/_apis/git/repositories/Shop.Web";
    assert_eq!(body("POST", &format!("{repo}/pullRequests/7/threads?"))["comments"][0]["content"], "Looks good");
    assert_eq!(body("POST", &format!("{repo}/pullRequests/7/threads/10/comments")), serde_json::json!({ "parentCommentId": 1, "content": "Thanks", "commentType": 1 }));
    assert_eq!(body("PUT", &format!("{repo}/pullRequests/7/reviewers/")), serde_json::json!({ "vote": 10 }));
    let patch = body("PATCH", &format!("{repo}/pullrequests/7"));
    assert_eq!((patch["status"].as_str(), patch["completionOptions"]["mergeStrategy"].as_str(), patch["lastMergeSourceCommit"]["commitId"].as_str()), (Some("completed"), Some("squash"), Some("src7")));
    assert_eq!(body("POST", &format!("{repo}/pullrequests?"))["sourceRefName"], "refs/heads/fix/typo");
}

#[test]
fn azure_sign_in_page_means_a_bad_token() {
    let server = MockServer::start(fake::azure);
    let c = HostClient::new(fake::azure_repo(&server.base), "wrong").unwrap();
    assert!(matches!(rt().block_on(c.list()), Err(HostError::Unauthorized(_))));
}
