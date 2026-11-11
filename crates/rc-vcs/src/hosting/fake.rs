//! Canned GitHub and Azure DevOps services for [`MockServer`](super::mock::MockServer): one
//! repository each, a few pull requests with files, reviews and conversations. Used by the REST
//! client tests and the app's window tests and captures.

use super::mock::Request;

/// The token the fake services accept.
pub const TOKEN: &str = "test-token-123";

/// The text of `src/Shop.Core/PriceCalculator.cs` before and after the fake pull requests.
pub const BEFORE: &str = "namespace Shop.Core;\n\npublic class PriceCalculator\n{\n    public decimal Total(decimal price, int quantity) => price * quantity;\n}\n";
pub const AFTER: &str = "namespace Shop.Core;\n\npublic class PriceCalculator\n{\n    public decimal Total(decimal price, int quantity, decimal discount = 0)\n    {\n        var total = price * quantity;\n        return total - total * discount;\n    }\n}\n";

fn split(path: &str) -> (&str, &str) {
    path.split_once('?').unwrap_or((path, ""))
}

fn json_string(s: &str) -> String {
    serde_json::Value::String(s.to_string()).to_string()
}

fn gh_pr(n: u64, title: &str, head: &str, draft: bool, author: &str) -> String {
    format!(
        r#"{{"number":{n},"title":"{title}","body":"Adds a **discount** to the totals.\n\n- Keeps the old overload\n- Tests included","user":{{"login":"{author}"}},"head":{{"ref":"{head}","sha":"head{n}"}},"base":{{"ref":"main","sha":"main{n}"}},"state":"open","draft":{draft},"created_at":"2026-11-0{}T10:00:00Z","html_url":"https://github.com/acme/shop/pull/{n}","requested_reviewers":[{{"login":"octo"}}],"merged_at":null,"mergeable":true}}"#,
        n % 9 + 1
    )
}

/// A fake GitHub for `acme/shop` (API root = the server's base URL), signed in as `octo`.
pub fn github(req: &Request) -> (u16, String) {
    if req.authorization.as_deref() != Some(&format!("Bearer {TOKEN}")) {
        return (401, r#"{"message":"Bad credentials"}"#.into());
    }
    let (path, query) = split(&req.path);
    match (req.method.as_str(), path) {
        ("GET", "/user") => (200, r#"{"login":"octo","name":"Octo Cat"}"#.into()),
        ("GET", "/repos/acme/shop") => (200, r#"{"default_branch":"main","permissions":{"push":true,"admin":false}}"#.into()),
        ("GET", "/repos/acme/shop/pulls") => (200, format!("[{},{},{}]", gh_pr(42, "Discounts in PriceCalculator", "feature/discounts", false, "mona"), gh_pr(41, "Cache product images", "feature/image-cache", false, "hubot"), gh_pr(40, "WIP: new checkout flow", "feature/checkout", true, "octo"))),
        ("GET", "/repos/acme/shop/pulls/42") => (200, gh_pr(42, "Discounts in PriceCalculator", "feature/discounts", false, "mona")),
        ("GET", "/repos/acme/shop/pulls/42/files") => (200, r#"[{"filename":"src/Shop.Core/PriceCalculator.cs","status":"modified","additions":5,"deletions":1},{"filename":"tests/Shop.Core.Tests/DiscountTests.cs","status":"added","additions":24,"deletions":0},{"filename":"src/Shop.Core/Pricing.cs","previous_filename":"src/Shop.Core/Prices.cs","status":"renamed","additions":0,"deletions":0}]"#.into()),
        ("GET", "/repos/acme/shop/pulls/42/reviews") => (200, r#"[{"id":900,"user":{"login":"hubot"},"state":"APPROVED","body":"","submitted_at":"2026-11-03T09:00:00Z"},{"id":901,"user":{"login":"octo"},"state":"COMMENTED","body":"Looks good overall.","submitted_at":"2026-11-03T11:00:00Z"}]"#.into()),
        ("GET", "/repos/acme/shop/issues/42/comments") => (200, r#"[{"id":5,"user":{"login":"mona"},"body":"Ready for review.","created_at":"2026-11-03T08:00:00Z"}]"#.into()),
        ("GET", "/repos/acme/shop/pulls/42/comments") => (200, r#"[{"id":7,"user":{"login":"hubot"},"body":"Should the discount be clamped to [0, 1]?","path":"src/Shop.Core/PriceCalculator.cs","line":8,"created_at":"2026-11-03T10:00:00Z"},{"id":8,"in_reply_to_id":7,"user":{"login":"mona"},"body":"Good point, done.","path":"src/Shop.Core/PriceCalculator.cs","line":8,"created_at":"2026-11-03T10:30:00Z"}]"#.into()),
        ("GET", p) if p.starts_with("/repos/acme/shop/compare/") => (200, r#"{"merge_base_commit":{"sha":"base42"}}"#.into()),
        ("GET", "/repos/acme/shop/contents/src/Shop.Core/PriceCalculator.cs") if query.contains("ref=base42") => (200, BEFORE.into()),
        ("GET", "/repos/acme/shop/contents/src/Shop.Core/PriceCalculator.cs") if query.contains("ref=head42") => (200, AFTER.into()),
        ("POST", "/repos/acme/shop/issues/42/comments") | ("POST", "/repos/acme/shop/pulls/42/comments/7/replies") => (201, r#"{"id":99}"#.into()),
        ("POST", "/repos/acme/shop/pulls/42/reviews") => (200, r#"{"id":902}"#.into()),
        ("PUT", "/repos/acme/shop/pulls/42/merge") => (200, r#"{"merged":true,"message":"Pull Request successfully merged"}"#.into()),
        ("PUT", "/repos/acme/shop/pulls/41/merge") => (405, r#"{"message":"Pull Request is not mergeable"}"#.into()),
        ("POST", "/repos/acme/shop/pulls") => {
            let v: serde_json::Value = serde_json::from_str(&req.body).unwrap_or_default();
            let title = v.get("title").and_then(|t| t.as_str()).unwrap_or_default();
            let head = v.get("head").and_then(|t| t.as_str()).unwrap_or_default();
            (201, gh_pr(43, title, head, v.get("draft").and_then(|d| d.as_bool()).unwrap_or(false), "octo"))
        }
        _ => (404, r#"{"message":"Not Found"}"#.into()),
    }
}

/// The Azure DevOps identity of the fake user.
pub const AZURE_ME: &str = "11111111-2222-3333-4444-555555555555";

fn az_pr(id: u64, title: &str, src: &str, draft: bool) -> String {
    format!(
        r#"{{"pullRequestId":{id},"title":"{title}","description":"Moves the settings page to the new layout.","createdBy":{{"displayName":"Ana García","id":"aaaa"}},"sourceRefName":"refs/heads/{src}","targetRefName":"refs/heads/main","status":"active","isDraft":{draft},"creationDate":"2026-11-02T12:00:00Z","reviewers":[{{"displayName":"Luis Pérez","vote":-5,"id":"bbbb"}},{{"displayName":"Javier","vote":0,"id":"{AZURE_ME}"}}],"lastMergeSourceCommit":{{"commitId":"src{id}"}},"lastMergeTargetCommit":{{"commitId":"tgt{id}"}},"mergeStatus":"succeeded"}}"#
    )
}

/// A fake Azure DevOps for `contoso/Shop/_git/Shop.Web` (API root = base + `/contoso`).
pub fn azure(req: &Request) -> (u16, String) {
    // Basic base64(":test-token-123"); Azure answers bad tokens with 203 and a sign-in page.
    if req.authorization.as_deref() != Some("Basic OnRlc3QtdG9rZW4tMTIz") {
        return (203, "<html><body>Sign in</body></html>".into());
    }
    let (path, query) = split(&req.path);
    let repo = "/contoso/Shop/_apis/git/repositories/Shop.Web";
    let Some(rest) = path.strip_prefix(repo) else {
        return match (req.method.as_str(), path) {
            ("GET", "/contoso/_apis/connectionData") => (200, format!(r#"{{"authenticatedUser":{{"id":"{AZURE_ME}","providerDisplayName":"Javier"}}}}"#)),
            _ => (404, r#"{"message":"not found"}"#.into()),
        };
    };
    match (req.method.as_str(), rest) {
        ("GET", "") => (200, r#"{"id":"r1","name":"Shop.Web","defaultBranch":"refs/heads/main"}"#.into()),
        ("GET", "/pullrequests") => (200, format!(r#"{{"value":[{},{}],"count":2}}"#, az_pr(7, "New settings layout", "feature/settings", false), az_pr(6, "Upgrade to .NET 10", "chore/net10", true))),
        ("GET", "/pullrequests/7") => (200, az_pr(7, "New settings layout", "feature/settings", false)),
        ("GET", "/pullRequests/7/iterations") => (200, r#"{"value":[{"id":1,"sourceRefCommit":{"commitId":"it1"},"commonRefCommit":{"commitId":"old1"}},{"id":2,"sourceRefCommit":{"commitId":"src7"},"commonRefCommit":{"commitId":"base7"}}]}"#.into()),
        ("GET", "/pullRequests/7/iterations/2/changes") => (200, r#"{"changeEntries":[{"changeType":"edit","item":{"path":"/src/Shop.Core/PriceCalculator.cs","gitObjectType":"blob"}},{"changeType":"add","item":{"path":"/src/Shop.Web/Pages/Settings.razor","gitObjectType":"blob"}},{"changeType":"rename, edit","item":{"path":"/src/Shop.Web/Layout.cs","gitObjectType":"blob"},"originalPath":"/src/Shop.Web/OldLayout.cs"},{"changeType":"add","item":{"path":"/src/Shop.Web/Pages","gitObjectType":"tree"}}]}"#.into()),
        ("GET", "/pullRequests/7/threads") => (200, r#"{"value":[{"id":10,"status":"active","threadContext":null,"comments":[{"id":1,"parentCommentId":0,"author":{"displayName":"Luis Pérez"},"content":"Please add a screenshot.","publishedDate":"2026-11-02T13:00:00Z","commentType":"text"},{"id":2,"parentCommentId":1,"author":{"displayName":"Ana García"},"content":"Added.","publishedDate":"2026-11-02T14:00:00Z","commentType":"text"}]},{"id":11,"status":"fixed","threadContext":{"filePath":"/src/Shop.Core/PriceCalculator.cs","rightFileStart":{"line":6,"offset":1}},"comments":[{"id":1,"parentCommentId":0,"author":{"displayName":"Luis Pérez"},"content":"Rename this variable.","publishedDate":"2026-11-02T15:00:00Z","commentType":"text"}]},{"id":12,"comments":[{"id":1,"author":{"displayName":"Microsoft.VisualStudio.Services.TFS"},"content":"Ana García updated the pull request","commentType":"system"}]}]}"#.into()),
        ("GET", "/items") if query.contains("PriceCalculator.cs") && query.contains("version=base7") => (200, format!(r#"{{"path":"/src/Shop.Core/PriceCalculator.cs","content":{}}}"#, json_string(BEFORE))),
        ("GET", "/items") if query.contains("PriceCalculator.cs") && query.contains("version=src7") => (200, format!(r#"{{"path":"/src/Shop.Core/PriceCalculator.cs","content":{}}}"#, json_string(AFTER))),
        ("POST", "/pullRequests/7/threads") => (200, r#"{"id":13}"#.into()),
        ("POST", "/pullRequests/7/threads/10/comments") => (200, r#"{"id":3}"#.into()),
        ("PUT", p) if p == format!("/pullRequests/7/reviewers/{AZURE_ME}") => (200, r#"{"vote":10}"#.into()),
        ("PATCH", "/pullrequests/7") => (200, az_pr(7, "New settings layout", "feature/settings", false).replace(r#""status":"active""#, r#""status":"completed""#)),
        ("POST", "/pullrequests") => {
            let v: serde_json::Value = serde_json::from_str(&req.body).unwrap_or_default();
            let title = v.get("title").and_then(|t| t.as_str()).unwrap_or_default();
            let src = v.get("sourceRefName").and_then(|t| t.as_str()).unwrap_or_default().trim_start_matches("refs/heads/").to_string();
            (201, az_pr(8, title, &src, false))
        }
        _ => (404, r#"{"message":"not found"}"#.into()),
    }
}

/// The fake GitHub repository, its API at `base`.
pub fn github_repo(base: &str) -> super::HostRepo {
    let mut r = super::HostRepo::parse("https://github.com/acme/shop.git").expect("GitHub URL");
    r.api = base.to_string();
    r
}

/// The fake Azure DevOps repository, its API at `base`.
pub fn azure_repo(base: &str) -> super::HostRepo {
    let mut r = super::HostRepo::parse("https://contoso@dev.azure.com/contoso/Shop/_git/Shop.Web").expect("Azure DevOps URL");
    r.api = format!("{base}/contoso");
    r
}
