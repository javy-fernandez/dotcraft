//! Explicit Git maintenance requests; no command strings or shell interpolation.
use rc_kit::i18n::tr;
use rc_vcs::{CommitOptions, DiffOptions, DiffSpec, GitError, Repository};
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Debug)]
pub enum ToolAction {
    Clone { source: String, destination: PathBuf },
    CherryPick(String),
    ContinueCherryPick,
    AbortCherryPick,
    ListWorktrees,
    AddWorktree { destination: PathBuf, branch: String },
    RemoveWorktree(PathBuf),
    ListUnstagedHunks(String),
    StageHunk { path: String, number: String },
    CommitStaged(String),
    SubmoduleStatus,
    UpdateSubmodules,
    LfsStatus,
    LfsTrack(String),
    LfsPull,
}

#[derive(Default)]
pub struct RepositoryTools {
    pub visible: bool,
    pub source: String,
    pub destination: String,
    pub worktree_path: String,
    pub revision: String,
    pub branch: String,
    pub pattern: String,
    pub partial_path: String,
    pub partial_hunk: String,
    pub partial_message: String,
    pub report: String,
    pub confirmation: Option<ToolAction>,
    pub cloned: Option<PathBuf>,
}

impl ToolAction {
    pub fn changes_worktree(&self) -> bool {
        matches!(self, Self::CherryPick(_) | Self::ContinueCherryPick | Self::AbortCherryPick | Self::UpdateSubmodules | Self::LfsPull | Self::LfsTrack(_) | Self::RemoveWorktree(_) | Self::StageHunk { .. } | Self::CommitStaged(_))
    }

    pub async fn run(self, repo: Option<Repository>) -> Result<(String, Option<PathBuf>), GitError> {
        if let Self::Clone { source, destination } = self {
            rc_vcs::Git::clone_repository(&source, &destination).await?;
            return Ok((destination.display().to_string(), Some(destination)));
        }
        let repo = repo.ok_or_else(|| GitError::NotARepository(PathBuf::new()))?;
        let report = match self {
            Self::Clone { .. } => unreachable!(),
            Self::CherryPick(revision) => {
                repo.cherry_pick_commit(&revision).await?;
                repo.resolve("HEAD").await?
            }
            Self::ContinueCherryPick => {
                repo.cherry_pick_continue().await?;
                repo.resolve("HEAD").await?
            }
            Self::AbortCherryPick => {
                repo.cherry_pick_abort().await?;
                repo.resolve("HEAD").await?
            }
            Self::ListWorktrees => repo
                .worktrees()
                .await?
                .into_iter()
                .map(|w| format!("{}\n  {}  {}  {}\n", w.path.display(), w.head, w.branch.as_deref().unwrap_or("HEAD"), if w.locked { tr("Locked") } else { "" }))
                .collect::<Vec<_>>()
                .join("\n"),
            Self::AddWorktree { destination, branch } => {
                repo.add_worktree(&destination, &branch).await?;
                destination.display().to_string()
            }
            Self::RemoveWorktree(path) => {
                repo.remove_worktree(&path).await?;
                path.display().to_string()
            }
            Self::ListUnstagedHunks(path) => {
                let file = unstaged_file(&repo, &path).await?;
                if file.binary || file.hunks.is_empty() {
                    return Err(GitError::Parse("Choose a text file with unstaged hunks".into()));
                }
                file.hunks.iter().enumerate().take(100).map(|(index, hunk)| {
                    let preview = hunk.lines.iter().filter(|line| line.kind != rc_vcs::DiffLineKind::Context).take(4)
                        .map(|line| format!("{} {}", if line.kind == rc_vcs::DiffLineKind::Added { '+' } else { '-' }, line.text.chars().take(120).collect::<String>()))
                        .collect::<Vec<_>>().join("\n");
                    format!("{}: @@ -{},{} +{},{} @@ {}\n{}", index + 1, hunk.old_start, hunk.old_lines, hunk.new_start, hunk.new_lines, hunk.section, preview)
                }).collect::<Vec<_>>().join("\n\n")
            }
            Self::StageHunk { path, number } => {
                let file = unstaged_file(&repo, &path).await?;
                if file.binary {
                    return Err(GitError::Parse("Binary files cannot be staged by hunk".into()));
                }
                let index = number.trim().parse::<usize>().ok().and_then(|n| n.checked_sub(1)).ok_or_else(|| GitError::Parse("Enter a positive hunk number".into()))?;
                repo.stage_hunk(&file, index).await?;
                format!("Staged hunk {} of {}", index + 1, file.path())
            }
            Self::CommitStaged(message) => {
                let message = message.trim();
                if message.is_empty() {
                    return Err(GitError::Parse("Enter a commit message".into()));
                }
                repo.ensure_no_conflicts().await?;
                let status = repo.status(&Default::default()).await?;
                if status.staged().next().is_none() {
                    return Err(GitError::NothingToCommit);
                }
                repo.commit(&CommitOptions { message: message.into(), ..Default::default() }).await?
            }
            Self::SubmoduleStatus => repo.submodule_status().await?,
            Self::UpdateSubmodules => {
                repo.update_submodules().await?;
                repo.submodule_status().await?
            }
            Self::LfsStatus => repo.lfs_status().await?,
            Self::LfsTrack(pattern) => {
                repo.lfs_track(&pattern).await?;
                repo.lfs_status().await?
            }
            Self::LfsPull => {
                repo.lfs_pull().await?;
                repo.lfs_status().await?
            }
        };
        Ok((report.chars().take(65_536).collect(), None))
    }
}

async fn unstaged_file(repo: &Repository, path: &str) -> Result<rc_vcs::FileDiff, GitError> {
    let path = path.trim();
    let p = Path::new(path);
    if path.is_empty() || path.starts_with('-') || p.components().any(|part| !matches!(part, Component::Normal(_))) {
        return Err(GitError::Parse("Choose a repository-relative file path".into()));
    }
    let files = repo.diff(&DiffSpec::WorktreeVsIndex, &DiffOptions { paths: vec![path.into()], ..Default::default() }).await?;
    files.into_iter().find(|file| file.path() == path).ok_or_else(|| GitError::NotFound(path.into()))
}
