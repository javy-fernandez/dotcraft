//! Explicit Git maintenance requests; no command strings or shell interpolation.
use rc_kit::i18n::tr;
use rc_vcs::{GitError, Repository};
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub enum ToolAction {
    Clone { source: String, destination: PathBuf },
    CherryPick(String),
    ContinueCherryPick,
    AbortCherryPick,
    ListWorktrees,
    AddWorktree { destination: PathBuf, branch: String },
    RemoveWorktree(PathBuf),
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
    pub report: String,
    pub confirmation: Option<ToolAction>,
    pub cloned: Option<PathBuf>,
}

impl ToolAction {
    pub fn changes_worktree(&self) -> bool {
        matches!(self, Self::CherryPick(_) | Self::ContinueCherryPick | Self::AbortCherryPick | Self::UpdateSubmodules | Self::LfsPull | Self::LfsTrack(_) | Self::RemoveWorktree(_))
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
