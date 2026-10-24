//! DotCraft's code editor widget for egui: per-line galley layout of the visible lines only,
//! a Rider-style gutter, multi-caret editing, folding, syntax and semantic colours, inlay
//! hints, diagnostics and the code-insight popups.

pub mod keymap;
pub mod layout;
pub mod popup;
mod popups_view;
mod select;
pub mod state;
pub mod view;

pub use keymap::{Action, Keymap, KeymapKind};
pub use popup::{Completion, CompletionItem, Hover, Popups, Signature, SignatureHelp};
pub use popups_view::{accept as accept_completion, expand_snippet, render_doc};
pub use state::{BreakpointKind, Diagnostic, EditorEvent, EditorState, GutterIcon, InlayHint, LineChange, Severity};
pub use view::{EditorConfig, EditorOutput, EditorView, run_action};
