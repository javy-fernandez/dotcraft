//! Per-workspace session: open editors with their caret, scroll and folds, the active tab,
//! the layout of the tool windows and the expanded explorer nodes. Restored when the
//! workspace is opened again.

use crate::paths;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OpenFile {
    pub path: PathBuf,
    pub line: usize,
    pub col: usize,
    pub scroll_y: f32,
    pub folded: Vec<usize>,
    pub pinned: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolWindowState {
    pub visible: bool,
    /// Width (side windows) or height (bottom windows) in points.
    pub size: f32,
    /// `left`, `right` or `bottom` when moved from its default side.
    pub side: Option<String>,
}

impl Default for ToolWindowState {
    fn default() -> Self {
        ToolWindowState { visible: false, size: 280.0, side: None }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Session {
    pub files: Vec<OpenFile>,
    pub active: Option<usize>,
    /// Tool window id → state.
    pub tool_windows: BTreeMap<String, ToolWindowState>,
    /// Explorer nodes expanded (paths relative to the workspace root, or node keys).
    pub expanded: Vec<String>,
    pub run_configuration: Option<String>,
    /// Breakpoints: file → 0-based lines (with an optional condition).
    pub breakpoints: BTreeMap<PathBuf, Vec<(usize, Option<String>)>>,
    /// Breakpoints that are disabled: file → 0-based lines.
    pub disabled_breakpoints: BTreeMap<PathBuf, Vec<usize>>,
}

impl Session {
    /// File of the session of `workspace` (keyed by a hash of its canonical path).
    pub fn file_for(workspace: &Path) -> PathBuf {
        let canon = workspace.canonicalize().unwrap_or_else(|_| workspace.to_path_buf());
        let name = canon.file_name().and_then(|n| n.to_str()).unwrap_or("workspace");
        paths::sessions_dir().join(format!("{name}-{:016x}.json", fnv(canon.to_string_lossy().as_bytes())))
    }

    pub fn load(workspace: &Path) -> Session {
        std::fs::read_to_string(Self::file_for(workspace)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }

    pub fn save(&self, workspace: &Path) -> anyhow::Result<()> {
        paths::write_atomic(&Self::file_for(workspace), serde_json::to_string_pretty(self)?.as_bytes())?;
        Ok(())
    }
}

/// FNV-1a: a stable hash for file names (std's hasher is not stable across runs).
fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325u64, |h, b| (h ^ *b as u64).wrapping_mul(0x100000001b3))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sessions_round_trip_per_workspace() {
        let dir = tempfile::tempdir().unwrap();
        // SAFETY: tests in this crate do not read DOTCRAFT_HOME concurrently with other values.
        unsafe { std::env::set_var("DOTCRAFT_HOME", dir.path()) };
        let ws = dir.path().join("MySolution");
        std::fs::create_dir_all(&ws).unwrap();
        let mut s = Session::default();
        s.files.push(OpenFile { path: ws.join("Program.cs"), line: 12, col: 4, scroll_y: 120.0, folded: vec![3], pinned: false });
        s.active = Some(0);
        s.save(&ws).unwrap();
        assert_eq!(Session::load(&ws), s);
        assert_eq!(Session::load(&dir.path().join("Other")), Session::default());
    }
}
