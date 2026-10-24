//! Git integration of DotCraft through the `git` CLI (never libgit2): repository discovery,
//! status, diffs, log, blame and write operations as async functions meant to run on
//! `rc_core::Runtime`, plus in-process line diffs for the gutter and the side-by-side viewer.
//! No UI code lives here.

pub mod gutter;
pub mod side_by_side;
pub mod textdiff;

pub use gutter::{GutterMark, LineChange, LineChangeKind, LineChanges};
pub use side_by_side::{Row, RowKind, SideBySide, inline_changes};
