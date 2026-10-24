//! Core services of DotCraft without UI: the shared async runtime, inboxes that wake the UI,
//! settings and per-workspace sessions, the workspace's file tree and file watching.

pub mod inbox;
pub mod paths;
pub mod runtime;
pub mod session;
pub mod settings;
pub mod watch;
pub mod workspace;

pub use inbox::{Inbox, Waker};
pub use runtime::Runtime;
pub use session::Session;
pub use settings::Settings;
pub use workspace::{FileNode, Workspace, WorkspaceKind};
