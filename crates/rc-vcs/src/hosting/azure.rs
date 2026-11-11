//! Azure DevOps Repos REST API 7.1 (`dev.azure.com/{organization}`).

use super::*;
use serde_json::{Value, json};

const V: &str = "api-version=7.1";

fn repo_url(c: &HostClient) -> String {
    format!("{}/{}/_apis/git/repositories/{}", c.repo.api, enc(&c.repo.project), enc(&c.repo.name))
}

fn short_ref(r: &str) -> String {
    r.strip_prefix("refs/heads/").unwrap_or(r).to_string()
}

pub(super) async fn viewer(c: &HostClient) -> HostResult<Viewer> {
    let data = c.get(&format!("{}/_apis/connectionData", c.repo.api)).await?;
    let repo = c.get(&format!("{}?{V}", repo_url(c))).await?;
    let user = data.get("authenticatedUser").cloned().unwrap_or(Value::Null);
    let id = s(&user, "id");
    if id.is_empty() || id == "00000000-0000-0000-0000-000000000000" {
        return Err(HostError::Unauthorized("anonymous access".into()));
    }
    let name = [s(&user, "providerDisplayName"), s(&user, "customDisplayName")].into_iter().find(|n| !n.is_empty()).unwrap_or_else(|| id.clone());
    Ok(Viewer { id, name, default_branch: short_ref(&s(&repo, "defaultBranch")), can_push: true })
}

fn vote_of(n: i64) -> Vote {
    match n {
        10 => Vote::Approved,
        5 => Vote::ApprovedWithSuggestions,
        -5 => Vote::ChangesRequested,
        -10 => Vote::Rejected,
        _ => Vote::None,
    }
}

fn web_url(c: &HostClient, id: u64) -> String {
    format!("{}/{}/_git/{}/pullrequest/{id}", c.repo.web, enc(&c.repo.project), enc(&c.repo.name))
}

fn parse_pr(c: &HostClient, v: &Value) -> PullRequest {
    let id = v.get("pullRequestId").and_then(|n| n.as_u64()).unwrap_or_default();
    let state = match s(v, "status").as_str() {
        "completed" => PrState::Merged,
        "abandoned" => PrState::Closed,
        _ if v.get("isDraft").and_then(|d| d.as_bool()) == Some(true) => PrState::Draft,
        _ => PrState::Open,
    };
    let reviewers = v
        .get("reviewers")
        .and_then(|r| r.as_array())
        .map(|a| a.iter().map(|r| Reviewer { name: s(r, "displayName"), vote: vote_of(r.get("vote").and_then(|n| n.as_i64()).unwrap_or(0)) }).collect())
        .unwrap_or_default();
    let mergeable = match s(v, "mergeStatus").as_str() {
        "succeeded" => Some(true),
        "conflicts" | "failure" | "rejectedByPolicy" => Some(false),
        _ => None,
    };
    PullRequest {
        id,
        title: s(v, "title"),
        description: s(v, "description"),
        author: v.pointer("/createdBy/displayName").and_then(|l| l.as_str()).unwrap_or_default().to_string(),
        author_id: v.pointer("/createdBy/id").and_then(|l| l.as_str()).unwrap_or_default().to_string(),
        source_branch: short_ref(&s(v, "sourceRefName")),
        target_branch: short_ref(&s(v, "targetRefName")),
        state,
        created: s(v, "creationDate"),
        web_url: web_url(c, id),
        head_sha: v.pointer("/lastMergeSourceCommit/commitId").and_then(|l| l.as_str()).unwrap_or_default().to_string(),
        base_sha: v.pointer("/lastMergeTargetCommit/commitId").and_then(|l| l.as_str()).unwrap_or_default().to_string(),
        reviewers,
        mergeable,
    }
}

pub(super) async fn list(c: &HostClient) -> HostResult<Vec<PullRequest>> {
    let v = c.get(&format!("{}/pullrequests?searchCriteria.status=active&$top=100&{V}", repo_url(c))).await?;
    let a = v.get("value").and_then(|a| a.as_array()).ok_or_else(|| HostError::Parse("expected a list of pull requests".into()))?;
    Ok(a.iter().map(|p| parse_pr(c, p)).collect())
}

fn parse_change(v: &Value) -> Option<PrFile> {
    let item = v.get("item")?;
    if s(item, "gitObjectType") == "tree" || item.get("isFolder").and_then(|f| f.as_bool()) == Some(true) {
        return None;
    }
    let kind = s(v, "changeType");
    let path = s(item, "path").trim_start_matches('/').to_string();
    let change = if kind.contains("rename") {
        FileChange::Renamed
    } else if kind.contains("add") {
        FileChange::Added
    } else if kind.contains("delete") {
        FileChange::Deleted
    } else {
        FileChange::Modified
    };
    let old_path = v.get("originalPath").and_then(|p| p.as_str()).map(|p| p.trim_start_matches('/').to_string()).or_else(|| v.get("sourceServerItem").and_then(|p| p.as_str()).filter(|_| change == FileChange::Renamed).map(|p| p.trim_start_matches('/').to_string()));
    Some(PrFile { path, old_path, change, additions: None, deletions: None })
}

fn parse_threads(v: &Value) -> Vec<Thread> {
    let mut out = Vec::new();
    for t in v.get("value").and_then(|a| a.as_array()).into_iter().flatten() {
        if t.get("isDeleted").and_then(|d| d.as_bool()) == Some(true) {
            continue;
        }
        let comments: Vec<Comment> = t
            .get("comments")
            .and_then(|a| a.as_array())
            .into_iter()
            .flatten()
            .filter(|c| s(c, "commentType") != "system" && c.get("isDeleted").and_then(|d| d.as_bool()) != Some(true))
            .map(|c| Comment { id: c.get("id").map(|i| i.to_string()).unwrap_or_default(), author: c.pointer("/author/displayName").and_then(|l| l.as_str()).unwrap_or_default().to_string(), body: s(c, "content"), created: s(c, "publishedDate") })
            .collect();
        if comments.is_empty() {
            continue;
        }
        let ctx = t.get("threadContext").filter(|c| !c.is_null());
        let path = ctx.map(|c| s(c, "filePath").trim_start_matches('/').to_string()).filter(|p| !p.is_empty());
        let line = ctx.and_then(|c| c.pointer("/rightFileStart/line").or_else(|| c.pointer("/leftFileStart/line"))).and_then(|l| l.as_u64()).map(|l| l as u32);
        let resolved = matches!(s(t, "status").as_str(), "fixed" | "closed" | "wontFix" | "byDesign");
        out.push(Thread { id: t.get("id").map(|i| i.to_string()).unwrap_or_default(), path, line, resolved, comments });
    }
    out
}

pub(super) async fn details(c: &HostClient, id: u64, _viewer: &Viewer) -> HostResult<PrDetails> {
    let base = repo_url(c);
    let pr = c.get(&format!("{base}/pullrequests/{id}?{V}")).await?;
    let pr = parse_pr(c, &pr);
    let iterations = c.get(&format!("{base}/pullRequests/{id}/iterations?{V}")).await?;
    let last = iterations.get("value").and_then(|a| a.as_array()).and_then(|a| a.last()).cloned().unwrap_or(Value::Null);
    let (mut files, mut base_commit, mut head_commit) = (Vec::new(), pr.base_sha.clone(), pr.head_sha.clone());
    if let Some(it) = last.get("id").and_then(|i| i.as_u64()) {
        // Iteration changes against the common ancestor (`$compareTo=0`), as the web shows them.
        let changes = c.get(&format!("{base}/pullRequests/{id}/iterations/{it}/changes?$top=2000&$compareTo=0&{V}")).await?;
        files = changes.get("changeEntries").and_then(|a| a.as_array()).into_iter().flatten().filter_map(parse_change).collect();
        if let Some(b) = last.pointer("/commonRefCommit/commitId").and_then(|x| x.as_str()) {
            base_commit = b.to_string();
        }
        if let Some(h) = last.pointer("/sourceRefCommit/commitId").and_then(|x| x.as_str()) {
            head_commit = h.to_string();
        }
    }
    let threads = parse_threads(&c.get(&format!("{base}/pullRequests/{id}/threads?{V}")).await?);
    let open = pr.is_open();
    let permissions = Permissions { can_review: open, can_merge: open && pr.state != PrState::Draft && pr.mergeable != Some(false) };
    Ok(PrDetails { pr, files, threads, base_commit, head_commit, permissions })
}

pub(super) async fn file_text(c: &HostClient, path: &str, commit: &str) -> HostResult<Option<String>> {
    let url = format!("{}/items?path={}&versionDescriptor.version={}&versionDescriptor.versionType=commit&includeContent=true&{V}", repo_url(c), enc(&format!("/{path}")), enc(commit));
    let v = c.get(&url).await?;
    Ok(v.get("content").and_then(|t| t.as_str()).map(str::to_string))
}

pub(super) async fn comment(c: &HostClient, id: u64, body: &str) -> HostResult<()> {
    let payload = json!({ "comments": [{ "parentCommentId": 0, "content": body, "commentType": 1 }], "status": 1 });
    c.post(&format!("{}/pullRequests/{id}/threads?{V}", repo_url(c)), &payload).await.map(drop)
}

pub(super) async fn reply(c: &HostClient, id: u64, thread: &Thread, body: &str) -> HostResult<()> {
    let parent: u64 = thread.comments.first().and_then(|c| c.id.parse().ok()).unwrap_or(1);
    let payload = json!({ "parentCommentId": parent, "content": body, "commentType": 1 });
    c.post(&format!("{}/pullRequests/{id}/threads/{}/comments?{V}", repo_url(c), enc(&thread.id)), &payload).await.map(drop)
}

pub(super) async fn review(c: &HostClient, id: u64, review: Review, body: &str, viewer: &Viewer) -> HostResult<()> {
    let vote = match review {
        Review::Approve => 10,
        Review::RequestChanges => -5,
    };
    let url = format!("{}/pullRequests/{id}/reviewers/{}?{V}", repo_url(c), enc(&viewer.id));
    c.json(c.request(reqwest::Method::PUT, &url).json(&json!({ "vote": vote }))).await?;
    if !body.trim().is_empty() {
        comment(c, id, body).await?;
    }
    Ok(())
}

pub(super) async fn merge(c: &HostClient, pr: &PullRequest, method: MergeMethod) -> HostResult<()> {
    let strategy = match method {
        MergeMethod::Merge => "noFastForward",
        MergeMethod::Squash => "squash",
        MergeMethod::Rebase => "rebase",
    };
    let payload = json!({
        "status": "completed",
        "lastMergeSourceCommit": { "commitId": pr.head_sha },
        "completionOptions": { "mergeStrategy": strategy },
    });
    let url = format!("{}/pullrequests/{}?{V}", repo_url(c), pr.id);
    c.json(c.request(reqwest::Method::PATCH, &url).json(&payload)).await.map(drop)
}

pub(super) async fn create(c: &HostClient, new: &NewPullRequest) -> HostResult<PullRequest> {
    let payload = json!({
        "sourceRefName": format!("refs/heads/{}", new.source),
        "targetRefName": format!("refs/heads/{}", new.target),
        "title": new.title,
        "description": new.description,
        "isDraft": new.draft,
    });
    c.post(&format!("{}/pullrequests?{V}", repo_url(c)), &payload).await.map(|v| parse_pr(c, &v))
}
