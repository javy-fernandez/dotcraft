//! Network operations (fetch, pull, push) with streamed progress and cancellation, and the
//! list of remotes.

use crate::cancel::CancelToken;
use crate::error::Result;
use crate::git::ProgressSink;
use crate::repo::Repository;

/// A configured remote.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Remote {
    pub name: String,
    pub fetch_url: String,
    pub push_url: String,
}

/// Options of [`Repository::fetch`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FetchOptions {
    /// Remote to fetch (`None` = all remotes).
    pub remote: Option<String>,
    /// Delete remote-tracking branches gone from the remote.
    pub prune: bool,
    pub tags: bool,
}

/// Options of [`Repository::pull`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PullOptions {
    pub remote: Option<String>,
    pub branch: Option<String>,
    /// Rebase instead of merge (`Some(false)` forces a merge; `None` uses git config).
    pub rebase: Option<bool>,
    pub ff_only: bool,
}

/// Options of [`Repository::push`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PushOptions {
    /// Remote (default: the branch's push remote / `origin`).
    pub remote: Option<String>,
    /// Refspec, usually a branch name (default: the current branch's upstream).
    pub refspec: Option<String>,
    /// Set the upstream (`-u`).
    pub set_upstream: bool,
    pub force_with_lease: bool,
    pub tags: bool,
    /// Delete `refspec` on the remote (`--delete`).
    pub delete: bool,
}

impl Repository {
    /// Configured remotes.
    pub async fn remotes(&self) -> Result<Vec<Remote>> {
        let out = self.cmd().args(["remote", "-v"]).text().await?;
        let mut remotes: Vec<Remote> = Vec::new();
        for line in out.lines() {
            let mut parts = line.split_whitespace();
            let (Some(name), Some(url), Some(kind)) = (parts.next(), parts.next(), parts.next()) else { continue };
            let i = match remotes.iter().position(|r| r.name == name) {
                Some(i) => i,
                None => {
                    remotes.push(Remote { name: name.to_string(), fetch_url: String::new(), push_url: String::new() });
                    remotes.len() - 1
                }
            };
            if kind == "(push)" {
                remotes[i].push_url = url.to_string();
            } else {
                remotes[i].fetch_url = url.to_string();
            }
        }
        Ok(remotes)
    }

    /// `git fetch`, streaming progress lines.
    pub async fn fetch(&self, opts: &FetchOptions, progress: Option<ProgressSink>, cancel: Option<&CancelToken>) -> Result<()> {
        let mut cmd = self.cmd().args(["fetch", "--progress"]).cancel(cancel);
        if opts.prune {
            cmd = cmd.arg("--prune");
        }
        if opts.tags {
            cmd = cmd.arg("--tags");
        }
        cmd = match &opts.remote {
            Some(r) => cmd.args(["--end-of-options", r]),
            None => cmd.arg("--all"),
        };
        cmd.streamed(progress).await.map(drop)
    }

    /// `git pull`, streaming progress lines. Conflicts fail with `Conflicts`.
    pub async fn pull(&self, opts: &PullOptions, progress: Option<ProgressSink>, cancel: Option<&CancelToken>) -> Result<()> {
        let mut cmd = self.cmd().args(["pull", "--progress", "--no-edit"]).cancel(cancel);
        match opts.rebase {
            Some(true) => cmd = cmd.arg("--rebase"),
            Some(false) => cmd = cmd.arg("--no-rebase"),
            None => {}
        }
        if opts.ff_only {
            cmd = cmd.arg("--ff-only");
        }
        if let Some(r) = &opts.remote {
            cmd = cmd.args(["--end-of-options", r]).args(&opts.branch);
        }
        cmd.streamed(progress).await.map(drop)
    }

    /// `git push`, streaming progress lines. Rejections fail with `PushRejected`, a branch
    /// without upstream with `NoUpstream`.
    pub async fn push(&self, opts: &PushOptions, progress: Option<ProgressSink>, cancel: Option<&CancelToken>) -> Result<()> {
        let mut cmd = self.cmd().args(["push", "--progress", "--porcelain"]).cancel(cancel);
        if opts.set_upstream {
            cmd = cmd.arg("--set-upstream");
        }
        if opts.force_with_lease {
            cmd = cmd.arg("--force-with-lease");
        }
        if opts.tags {
            cmd = cmd.arg("--follow-tags");
        }
        if opts.delete {
            cmd = cmd.arg("--delete");
        }
        if opts.remote.is_some() || opts.refspec.is_some() {
            cmd = cmd.arg("--end-of-options").arg(opts.remote.as_deref().unwrap_or("origin")).args(&opts.refspec);
        }
        cmd.streamed(progress).await.map(drop)
    }
}
