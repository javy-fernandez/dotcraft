//! Text model of the DotCraft editor: a rope with versions, change sets, multi-cursor
//! selections, grouped undo/redo, line endings and UTF-8 ↔ UTF-16 conversion for LSP.
//! No UI code lives here.

pub mod buffer;
pub mod change;
pub mod commands;
pub mod format;
pub mod markup;
pub mod selection;

pub use buffer::{Buffer, BytePoint, Edit, EditKind, LineEnding, Utf16Pos};
pub use change::{Assoc, Change, ChangeSet};
pub use commands::{Edited, Indent, Motion};
pub use selection::{Range, Selection};
pub use ropey::{Rope, RopeSlice};
