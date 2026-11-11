//! GitHub REST API v3 (also GitHub Enterprise Server at `/api/v3`).

use super::*;
use serde_json::{Value, json};

fn repo_url(c: &HostClient) -> String {
    format!("{}/repos/{}/{}", c.repo.api, enc(&c.repo.owner), enc(&c.repo.name))
}

pub(super) async fn viewer(c: &HostClient) -> HostResult<Viewer> {
    let user = c.get(&format!("{}/user", c.repo.api)).await?;
    let repo = c.get(&repo_url(c)).await?;
    let login = s(&user, "login");
    let name = user.get("name").and_then(|n| n.as_str()).filter(|n| !n.is_empty()).map(str::to_string).unwrap_or_else(|| login.clone());
    Ok(Viewer {
        id: login,
        name,
        default_branch: s(&repo, "default_branch"),
        can_push: repo.pointer("/permissions/push").and_then(|p| p.as_bool()).unwrap_or(false),
    })
}

fn parse_pr(v: &Value) -> PullRequest {
    let state = if v.get("merged_at").is_some_and(|m| !m.is_null()) || v.get("merged").and_then(|m| m.as_bool()) == Some(true) {
        PrState::Merged
    } else if s(v, "state") == "closed" {
        PrState::Closed
    } else if v.get("draft").and_then(|d| d.as_bool()) == Some(true) {
        PrState::Draft
    } else {
        PrState::Open
    };
    let reviewers = v
        .get("requested_reviewers")
        .and_then(|r| r.as_array())
        .map(|a| a.iter().map(|r| Reviewer { name: s(r, "login"), vote: Vote::None }).collect())
        .unwrap_or_default();
    PullRequest {
        id: v.get("number").and_then(|n| n.as_u64()).unwrap_or_default(),
        title: s(v, "title"),
        description: s(v, "body"),
        author: v.pointer("/user/login").and_then(|l| l.as_str()).unwrap_or_default().to_string(),
        author_id: v.pointer("/user/login").and_then(|l| l.as_str()).unwrap_or_default().to_string(),
        source_branch: v.pointer("/head/ref").and_then(|l| l.as_str()).unwrap_or_default().to_string(),
        target_branch: v.pointer("/base/ref").and_then(|l| l.as_str()).unwrap_or_default().to_string(),
        state,
        created: s(v, "created_at"),
        web_url: s(v, "html_url"),
        head_sha: v.pointer("/head/sha").and_then(|l| l.as_str()).unwrap_or_default().to_string(),
        base_sha: v.pointer("/base/sha").and_then(|l| l.as_str()).unwrap_or_default().to_string(),
        reviewers,
        mergeable: v.get("mergeable").and_then(|m| m.as_bool()),
    }
}

pub(super) async fn list(c: &HostClient) -> HostResult<Vec<PullRequest>> {
    let v = c.get(&format!("{}/pulls?state=open&sort=created&direction=desc&per_page=100", repo_url(c))).await?;
    let a = v.as_array().ok_or_else(|| HostError::Parse("expected a list of pull requests".into()))?;
    Ok(a.iter().map(parse_pr).collect())
}

fn parse_file(v: &Value) -> PrFile {
    let change = match s(v, "status").as_str() {
        "added" => FileChange::Added,
        "removed" => FileChange::Deleted,
        "renamed" | "copied" => FileChange::Renamed,
        _ => FileChange::Modified,
    };
    PrFile {
        path: s(v, "filename"),
        old_path: v.get("previous_filename").and_then(|p| p.as_str()).map(str::to_string),
        change,
        additions: v.get("additions").and_then(|n| n.as_u64()).map(|n| n as u32),
        deletions: v.get("deletions").and_then(|n| n.as_u64()).map(|n| n as u32),
    }
}

fn comment_of(v: &Value) -> Comment {
    Comment { id: v.get("id").map(|i| i.to_string()).unwrap_or_default(), author: v.pointer("/user/login").and_then(|l| l.as_str()).unwrap_or_default().to_string(), body: s(v, "body"), created: s(v, "created_at") }
}

/// Issue comments, review summaries and review comments (grouped by their first comment) as
/// conversations in time order.
fn threads(issue_comments: &Value, reviews: &Value, review_comments: &Value) -> Vec<Thread> {
    let mut out: Vec<Thread> = Vec::new();
    for c in issue_comments.as_array().into_iter().flatten() {
        let comment = comment_of(c);
        out.push(Thread { id: format!("issue:{}", comment.id), path: None, line: None, resolved: false, comments: vec![comment] });
    }
    for r in reviews.as_array().into_iter().flatten() {
        if s(r, "body").trim().is_empty() {
            continue;
        }
        let comment = Comment { id: r.get("id").map(|i| i.to_string()).unwrap_or_default(), author: r.pointer("/user/login").and_then(|l| l.as_str()).unwrap_or_default().to_string(), body: s(r, "body"), created: s(r, "submitted_at") };
        out.push(Thread { id: format!("review:{}", comment.id), path: None, line: None, resolved: false, comments: vec![comment] });
    }
    let mut roots: Vec<Thread> = Vec::new();
    for c in review_comments.as_array().into_iter().flatten() {
        let comment = comment_of(c);
        match c.get("in_reply_to_id").and_then(|i| i.as_u64()) {
            Some(parent) => {
                let id = format!("line:{parent}");
                match roots.iter_mut().find(|t| t.id == id) {
                    Some(t) => t.comments.push(comment),
                    None => roots.push(Thread { id, path: Some(s(c, "path")), line: line_of(c), resolved: false, comments: vec![comment] }),
                }
            }
            None => {
                let id = format!("line:{}", comment.id);
                match roots.iter_mut().find(|t| t.id == id) {
                    Some(t) => t.comments.insert(0, comment),
                    None => roots.push(Thread { id, path: Some(s(c, "path")), line: line_of(c), resolved: false, comments: vec![comment] }),
                }
            }
        }
    }
    out.extend(roots);
    out.sort_by(|a, b| a.comments.first().map(|c| c.created.as_str()).cmp(&b.comments.first().map(|c| c.created.as_str())));
    out
}

fn line_of(c: &Value) -> Option<u32> {
    c.get("line").and_then(|l| l.as_u64()).or_else(|| c.get("original_line").and_then(|l| l.as_u64())).map(|l| l as u32)
}

/// Each reviewer's latest decisive review (approved / changes requested; comments otherwise).
fn votes(pr: &mut PullRequest, reviews: &Value) {
    for r in reviews.as_array().into_iter().flatten() {
        let name = r.pointer("/user/login").and_then(|l| l.as_str()).unwrap_or_default().to_string();
        let vote = match s(r, "state").as_str() {
            "APPROVED" => Vote::Approved,
            "CHANGES_REQUESTED" => Vote::ChangesRequested,
            "DISMISSED" => Vote::None,
            _ => Vote::Commented,
        };
        match pr.reviewers.iter_mut().find(|x| x.name == name) {
            // A later comment does not undo an approval or a change request.
            Some(x) if vote == Vote::Commented && x.vote != Vote::None => {}
            Some(x) => x.vote = vote,
            None => pr.reviewers.push(Reviewer { name, vote }),
        }
    }
}

pub(super) async fn details(c: &HostClient, id: u64, viewer: &Viewer) -> HostResult<PrDetails> {
    let base = repo_url(c);
    let pr = c.get(&format!("{base}/pulls/{id}")).await?;
    let files = c.get(&format!("{base}/pulls/{id}/files?per_page=100")).await?;
    let reviews = c.get(&format!("{base}/pulls/{id}/reviews?per_page=100")).await?;
    let issue_comments = c.get(&format!("{base}/issues/{id}/comments?per_page=100")).await?;
    let review_comments = c.get(&format!("{base}/pulls/{id}/comments?per_page=100")).await?;
    let mut pr = parse_pr(&pr);
    votes(&mut pr, &reviews);
    // The diff GitHub shows is against the merge base, not the target's tip.
    let base_commit = match c.get(&format!("{base}/compare/{}...{}?per_page=1", enc(&pr.base_sha), enc(&pr.head_sha))).await {
        Ok(cmp) => cmp.pointer("/merge_base_commit/sha").and_then(|s| s.as_str()).map(str::to_string).unwrap_or_else(|| pr.base_sha.clone()),
        Err(_) => pr.base_sha.clone(),
    };
    let open = pr.is_open();
    let permissions = Permissions { can_review: open && pr.author_id != viewer.id, can_merge: open && pr.state != PrState::Draft && viewer.can_push && pr.mergeable != Some(false) };
    Ok(PrDetails {
        files: files.as_array().into_iter().flatten().map(parse_file).collect(),
        threads: threads(&issue_comments, &reviews, &review_comments),
        base_commit,
        head_commit: pr.head_sha.clone(),
        permissions,
        pr,
    })
}

pub(super) async fn file_text(c: &HostClient, path: &str, commit: &str) -> HostResult<Option<String>> {
    let url = format!("{}/contents/{}?ref={}", repo_url(c), enc_path(path), enc(commit));
    let body = c.send(c.request(reqwest::Method::GET, &url).header("Accept", "application/vnd.github.raw")).await?;
    Ok(Some(body))
}

pub(super) async fn comment(c: &HostClient, id: u64, body: &str) -> HostResult<()> {
    c.post(&format!("{}/issues/{id}/comments", repo_url(c)), &json!({ "body": body })).await.map(drop)
}

pub(super) async fn reply(c: &HostClient, id: u64, thread: &Thread, body: &str) -> HostResult<()> {
    match thread.id.strip_prefix("line:") {
        Some(root) => c.post(&format!("{}/pulls/{id}/comments/{root}/replies", repo_url(c)), &json!({ "body": body })).await.map(drop),
        // Conversation comments have no replies on GitHub: answer with a new comment quoting it.
        None => {
            let quoted = thread.comments.last().map(|q| format!("> @{} wrote:\n> {}\n\n", q.author, q.body.lines().collect::<Vec<_>>().join("\n> "))).unwrap_or_default();
            comment(c, id, &format!("{quoted}{body}")).await
        }
    }
}

pub(super) async fn review(c: &HostClient, id: u64, review: Review, body: &str) -> HostResult<()> {
    let event = match review {
        Review::Approve => "APPROVE",
        Review::RequestChanges => "REQUEST_CHANGES",
    };
    let mut payload = json!({ "event": event });
    if !body.trim().is_empty() {
        payload["body"] = json!(body);
    }
    c.post(&format!("{}/pulls/{id}/reviews", repo_url(c)), &payload).await.map(drop)
}

pub(super) async fn merge(c: &HostClient, pr: &PullRequest, method: MergeMethod) -> HostResult<()> {
    let m = match method {
        MergeMethod::Merge => "merge",
        MergeMethod::Squash => "squash",
        MergeMethod::Rebase => "rebase",
    };
    let payload = json!({ "merge_method": m, "sha": pr.head_sha });
    c.json(c.request(reqwest::Method::PUT, &format!("{}/pulls/{}/merge", repo_url(c), pr.id)).json(&payload)).await.map(drop)
}

pub(super) async fn create(c: &HostClient, new: &NewPullRequest) -> HostResult<PullRequest> {
    let payload = json!({ "title": new.title, "body": new.description, "head": new.source, "base": new.target, "draft": new.draft });
    c.post(&format!("{}/pulls", repo_url(c)), &payload).await.map(|v| parse_pr(&v))
}
