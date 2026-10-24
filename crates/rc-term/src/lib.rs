//! DotCraft's terminal: sessions on a pseudo-terminal (`portable-pty`) emulated with
//! `alacritty_terminal`, an egui widget that paints and drives them, and the tool window's
//! tab model.
//!
//! ```ignore
//! let mut terms = rc_term::Terminals::new();
//! terms.open(&rc_term::SpawnSpec::shell(Some(root)), waker)?;
//! // every frame, inside the Terminal tool window:
//! for (id, ev) in terms.poll() { /* titles, exits, OSC 52 clipboard… */ }
//! if let Some(tab) = terms.active() {
//!     let out = rc_term::TerminalView::new(&tab.session, &config).show(ui);
//!     for ev in out.events { if let rc_term::ViewEvent::LinkClicked(link) = ev { /* open */ } }
//! }
//! ```

pub mod colors;
pub mod keys;
pub mod links;
pub mod session;
pub mod tabs;
pub mod view;

pub use alacritty_terminal::vte::ansi::CursorShape;
pub use colors::TermColors;
pub use links::Link;
pub use session::{ExitInfo, Program, SelectKind, SessionEvent, SpawnSpec, TermSession, TermState};
pub use tabs::{TermTab, Terminals};
pub use view::{TermConfig, TermOutput, TerminalView, ViewEvent};
