//! Interactive rebase: a plan (the commits from one commit up to HEAD, oldest first, each with
//! an action) is written as git's todo list through `GIT_SEQUENCE_EDITOR`, so git itself does
//! the work and stops on conflicts or `edit` steps exactly like `git rebase -i` in a terminal.
//! New messages (reword, or the message of a squashed group) are applied by an `exec` step that
//! amends the commit with a prepared message file, so no editor is ever opened.

use crate::error::{GitError, Result};
use crate::log::{Commit, LogOptions};
use crate::repo::{RepoState, Repository};
use std::path::PathBuf;

/// What happens to one commit of an interactive rebase.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum RebaseAction {
    #[default]
    Pick,
    /// Pick with a new message.
    Reword,
    /// Pick, then stop so the commit can be amended.
    Edit,
    /// Meld into the previous commit, keeping both messages.
    Squash,
    /// Meld into the previous commit, discarding this message.
    Fixup,
    Drop,
}

impl RebaseAction {
    pub const ALL: [RebaseAction; 6] = [RebaseAction::Pick, RebaseAction::Reword, RebaseAction::Edit, RebaseAction::Squash, RebaseAction::Fixup, RebaseAction::Drop];

    /// The todo keyword (`pick`, `squash`…).
    pub fn keyword(self) -> &'static str {
        match self {
            RebaseAction::Pick | RebaseAction::Reword => "pick",
            RebaseAction::Edit => "edit",
            RebaseAction::Squash => "squash",
            RebaseAction::Fixup => "fixup",
            RebaseAction::Drop => "drop",
        }
    }

    /// Melds into the previous kept commit.
    pub fn melds(self) -> bool {
        matches!(self, RebaseAction::Squash | RebaseAction::Fixup)
    }
}

/// One commit of a [`RebasePlan`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RebaseStep {
    pub commit: Commit,
    pub action: RebaseAction,
    /// The new full message for [`RebaseAction::Reword`]. On the first commit of a squash
    /// group it becomes the message of the whole group.
    pub message: Option<String>,
}

impl RebaseStep {
    /// The commit's original full message (subject and body).
    pub fn original_message(&self) -> String {
        if self.commit.body.trim().is_empty() { self.commit.subject.clone() } else { format!("{}\n\n{}", self.commit.subject, self.commit.body.trim_end()) }
    }
}

/// The commits an interactive rebase rewrites, oldest first, on top of `base`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RebasePlan {
    /// The commit the rewritten ones go on top of; `None` rewrites from the root commit.
    pub base: Option<String>,
    pub steps: Vec<RebaseStep>,
}

/// Why a plan cannot run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanProblem {
    /// Every commit is dropped: there is nothing left to rebase.
    NothingToRebase,
    /// The first kept commit squashes or fixes up into nothing.
    MeldWithoutTarget,
    /// A reword has an empty message.
    EmptyMessage,
}

impl RebasePlan {
    /// Checks the plan before running it.
    pub fn validate(&self) -> Option<PlanProblem> {
        let mut has_target = false;
        for s in &self.steps {
            match s.action {
                RebaseAction::Drop => {}
                a if a.melds() && !has_target => return Some(PlanProblem::MeldWithoutTarget),
                a => {
                    has_target = true;
                    if a == RebaseAction::Reword && s.message.as_deref().is_none_or(|m| m.trim().is_empty()) {
                        return Some(PlanProblem::EmptyMessage);
                    }
                }
            }
        }
        if has_target { None } else { Some(PlanProblem::NothingToRebase) }
    }

    /// Whether the plan changes nothing: every commit picked (or reworded to its own message)
    /// in its original order `original`.
    pub fn is_unchanged(&self, original: &[String]) -> bool {
        let same_message = |s: &RebaseStep| s.message.as_deref().map(str::trim) == Some(s.original_message().trim());
        self.steps.iter().map(|s| &s.commit.hash).eq(original.iter())
            && self.steps.iter().all(|s| s.action == RebaseAction::Pick || s.action == RebaseAction::Reword && same_message(s))
    }

    /// git's todo list for this plan. `amend(i)` returns the `exec` command that gives the
    /// commit just made the new message of step `i`.
    pub fn todo(&self, amend: impl Fn(usize) -> String) -> String {
        let mut out = String::new();
        let mut pending: Option<usize> = None;
        for (i, s) in self.steps.iter().enumerate() {
            if !s.action.melds() && s.action != RebaseAction::Drop
                && let Some(p) = pending.take()
            {
                out.push_str(&format!("exec {}\n", amend(p)));
            }
            let subject: String = s.commit.subject.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
            out.push_str(&format!("{} {} {}\n", s.action.keyword(), s.commit.hash, subject));
            if s.action == RebaseAction::Reword && s.message.as_deref().is_some_and(|m| !m.trim().is_empty()) {
                pending = Some(i);
            }
        }
        if let Some(p) = pending {
            out.push_str(&format!("exec {}\n", amend(p)));
        }
        out
    }
}

/// Why a rebase in progress stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RebaseStop {
    /// Files conflict: resolve them, then continue (or skip the commit).
    Conflicts,
    /// An `edit` step: amend the commit, then continue.
    Edit,
    /// An `exec` step failed.
    ExecFailed,
    /// Anything else (a `break`, a commit that became empty…).
    Other,
}

/// A rebase in progress.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RebaseStatus {
    /// The branch being rebased (`None` for a detached HEAD).
    pub branch: Option<String>,
    pub onto: String,
    /// 1-based number of the current step and the total (todo lines, including `exec`).
    pub step: usize,
    pub total: usize,
    /// The commit git stopped at (hash and subject), when known.
    pub commit: Option<(String, String)>,
    pub stop: RebaseStop,
    pub conflicts: Vec<String>,
}

/// The result of starting or resuming a rebase.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RebaseOutcome {
    Done,
    Stopped(RebaseStatus),
}

/// Quotes a word for `sh` (git runs editors and `exec` lines through a shell, on Windows too).
fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn todo_lines(text: &str) -> impl Iterator<Item = &str> {
    text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#'))
}

impl Repository {
    /// Where the prepared todo and message files live while a rebase runs.
    fn rebase_scratch(&self) -> PathBuf {
        self.git_dir().join("dotcraft-rebase")
    }

    /// The plan to interactively rebase from `from` (included) up to HEAD: every commit picked,
    /// oldest first. Fails when HEAD does not contain `from` or the range has merge commits.
    pub async fn interactive_rebase_plan(&self, from: &str) -> Result<RebasePlan> {
        let from = self.resolve(from).await?;
        let head = self.resolve("HEAD").await?;
        let contained = self.cmd().args(["merge-base", "--is-ancestor", &from, &head]).ok_code(1).output().await?;
        if contained.code != Some(0) {
            return Err(GitError::NotFound(format!("{from} in HEAD")));
        }
        let first = self.log(&LogOptions { revs: vec![from.clone()], max_count: 1, ..Default::default() }).await?;
        let base = first.first().and_then(|c| c.parents.first().cloned());
        let range = match &base {
            Some(b) => format!("{b}..{head}"),
            None => head.clone(),
        };
        let mut commits = self.log(&LogOptions { revs: vec![range], max_count: 10_000, ..Default::default() }).await?;
        if commits.iter().any(Commit::is_merge) {
            return Err(GitError::RebaseMergeCommits);
        }
        commits.reverse();
        Ok(RebasePlan { base, steps: commits.into_iter().map(|commit| RebaseStep { commit, action: RebaseAction::Pick, message: None }).collect() })
    }

    /// Runs `plan` as `git rebase -i`. Stops (leaving the rebase in progress) on conflicts and
    /// `edit` steps; see [`Repository::continue_rebase`], [`Repository::skip_rebase`] and
    /// [`Repository::abort_rebase`].
    pub async fn interactive_rebase(&self, plan: &RebasePlan) -> Result<RebaseOutcome> {
        if let Some(problem) = plan.validate() {
            return Err(GitError::InvalidRebasePlan(format!("{problem:?}")));
        }
        self.ensure_no_conflicts().await?;
        let dir = self.rebase_scratch();
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)?;
        let git = self.git().path().to_string_lossy().replace('\\', "/");
        for (i, s) in plan.steps.iter().enumerate() {
            if let Some(m) = s.message.as_deref().filter(|_| s.action == RebaseAction::Reword) {
                std::fs::write(dir.join(format!("msg-{i}")), m.trim_end().to_string() + "\n")?;
            }
        }
        let todo = plan.todo(|i| {
            let msg = dir.join(format!("msg-{i}")).to_string_lossy().replace('\\', "/");
            format!("{} commit --amend --allow-empty --no-verify --quiet --cleanup=strip -F {}", sh_quote(&git), sh_quote(&msg))
        });
        let todo_path = dir.join("todo");
        std::fs::write(&todo_path, todo)?;
        let editor = format!("cp {}", sh_quote(&todo_path.to_string_lossy().replace('\\', "/")));
        let mut cmd = self.cmd().env("GIT_SEQUENCE_EDITOR", editor).args(["-c", "rebase.abbreviateCommands=false", "rebase", "-i", "--no-autosquash", "--keep-empty"]);
        cmd = match &plan.base {
            Some(base) => cmd.args(["--end-of-options", base]),
            None => cmd.arg("--root"),
        };
        self.finish_rebase_step(cmd.output().await).await
    }

    /// Continues the rebase in progress after conflicts are resolved or a commit was edited.
    /// Conflicted files fail with [`GitError::Conflicts`].
    pub async fn continue_rebase(&self) -> Result<RebaseOutcome> {
        let conflicts = self.conflicted_paths().await?;
        if !conflicts.is_empty() {
            return Err(GitError::Conflicts(conflicts.join("\n")));
        }
        let r = self.cmd().args(["rebase", "--continue"]).output().await;
        self.finish_rebase_step(r).await
    }

    /// Skips the commit the rebase stopped at and continues.
    pub async fn skip_rebase(&self) -> Result<RebaseOutcome> {
        let r = self.cmd().args(["rebase", "--skip"]).output().await;
        self.finish_rebase_step(r).await
    }

    async fn finish_rebase_step(&self, run: Result<crate::git::Output>) -> Result<RebaseOutcome> {
        let rebasing = self.state().await == RepoState::Rebasing;
        match run {
            Ok(_) | Err(GitError::Conflicts(_)) if rebasing => match self.rebase_status().await? {
                Some(st) => Ok(RebaseOutcome::Stopped(st)),
                None => Ok(RebaseOutcome::Done),
            },
            Ok(_) => {
                let _ = std::fs::remove_dir_all(self.rebase_scratch());
                Ok(RebaseOutcome::Done)
            }
            Err(e) => Err(e),
        }
    }

    /// The rebase in progress, if any.
    pub async fn rebase_status(&self) -> Result<Option<RebaseStatus>> {
        let merge = self.git_dir().join("rebase-merge");
        let apply = self.git_dir().join("rebase-apply");
        let read = |p: PathBuf| std::fs::read_to_string(p).unwrap_or_default();
        let conflicts = self.conflicted_paths().await?;
        if merge.is_dir() {
            let done = read(merge.join("done"));
            let todo = read(merge.join("git-rebase-todo"));
            let done_n = todo_lines(&done).count();
            let total = done_n + todo_lines(&todo).count();
            let last = todo_lines(&done).last().unwrap_or("").to_string();
            let mut words = last.splitn(3, ' ');
            let (kw, arg, rest) = (words.next().unwrap_or(""), words.next().unwrap_or(""), words.next().unwrap_or(""));
            let stopped = read(merge.join("stopped-sha")).trim().to_string();
            let hash = if !stopped.is_empty() { Some(stopped) } else if !matches!(kw, "exec" | "x" | "break" | "b" | "") { Some(arg.to_string()) } else { None };
            let commit = match hash {
                Some(h) => {
                    let full = self.resolve(&h).await.unwrap_or(h);
                    let subject = self.cmd().args(["log", "-1", "--format=%s", "--end-of-options", &full]).text().await.map(|s| s.trim().to_string()).unwrap_or_else(|_| rest.to_string());
                    Some((full, subject))
                }
                None => None,
            };
            let stop = if !conflicts.is_empty() {
                RebaseStop::Conflicts
            } else if merge.join("amend").exists() && matches!(kw, "edit" | "e") {
                RebaseStop::Edit
            } else if matches!(kw, "exec" | "x") {
                RebaseStop::ExecFailed
            } else {
                RebaseStop::Other
            };
            let head = read(merge.join("head-name")).trim().to_string();
            return Ok(Some(RebaseStatus {
                branch: head.strip_prefix("refs/heads/").map(str::to_string),
                onto: read(merge.join("onto")).trim().to_string(),
                step: done_n.max(1),
                total: total.max(1),
                commit,
                stop,
                conflicts,
            }));
        }
        if apply.is_dir() {
            let num = |name: &str| read(apply.join(name)).trim().parse::<usize>().unwrap_or(1);
            let head = read(apply.join("head-name")).trim().to_string();
            return Ok(Some(RebaseStatus {
                branch: head.strip_prefix("refs/heads/").map(str::to_string),
                onto: read(apply.join("onto")).trim().to_string(),
                step: num("next"),
                total: num("last"),
                commit: None,
                stop: if conflicts.is_empty() { RebaseStop::Other } else { RebaseStop::Conflicts },
                conflicts,
            }));
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(hash: &str, action: RebaseAction, message: Option<&str>) -> RebaseStep {
        let commit = Commit {
            hash: hash.into(),
            parents: vec![],
            author_name: String::new(),
            author_email: String::new(),
            author_time: 0,
            committer_name: String::new(),
            committer_email: String::new(),
            commit_time: 0,
            refs: vec![],
            head_branch: None,
            subject: format!("s {hash}"),
            body: String::new(),
        };
        RebaseStep { commit, action, message: message.map(str::to_string) }
    }

    #[test]
    fn writes_the_todo_with_amends_after_groups() {
        use RebaseAction::*;
        let plan = RebasePlan {
            base: None,
            steps: vec![step("a", Reword, Some("A")), step("b", Squash, None), step("c", Fixup, None), step("d", Drop, None), step("e", Pick, None), step("f", Reword, Some("F"))],
        };
        assert_eq!(plan.todo(|i| format!("amend {i}")), "pick a s a\nsquash b s b\nfixup c s c\ndrop d s d\nexec amend 0\npick e s e\npick f s f\nexec amend 5\n");
        let edit = RebasePlan { base: None, steps: vec![step("a", Reword, Some("A")), step("b", Edit, None)] };
        assert_eq!(edit.todo(|i| format!("amend {i}")), "pick a s a\nexec amend 0\nedit b s b\n");
    }

    #[test]
    fn validates_plans() {
        use RebaseAction::*;
        let plan = |steps| RebasePlan { base: None, steps };
        assert_eq!(plan(vec![step("a", Drop, None)]).validate(), Some(PlanProblem::NothingToRebase));
        assert_eq!(plan(vec![step("a", Drop, None), step("b", Fixup, None)]).validate(), Some(PlanProblem::MeldWithoutTarget));
        assert_eq!(plan(vec![step("a", Reword, Some("  "))]).validate(), Some(PlanProblem::EmptyMessage));
        assert_eq!(plan(vec![step("a", Drop, None), step("b", Pick, None), step("c", Squash, None)]).validate(), None);
    }

    #[test]
    fn quotes_for_sh() {
        assert_eq!(sh_quote("/a b/it's"), "'/a b/it'\\''s'");
    }
}
