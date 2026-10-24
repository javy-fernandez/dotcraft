//! Search without UI: the workspace file index (walked with `ignore`, kept up to date from
//! file-system events), IntelliJ-like fuzzy ranking for Go to File and other lists, Find in
//! Files streamed through an [`rc_core::inbox::Outbox`], Replace in Files and single-buffer search.

pub mod files;
pub mod find;
pub mod fuzzy;
pub mod index;
pub mod mask;
pub mod query;
pub mod replace;

pub use files::{Boost, FileMatch, FileQuery, FileSearchRequest, FileSearchResults, FileSearchWorker, search_files};
pub use find::{FileMatches, FindEvent, FindOptions, FindSummary, Finder, LineMatch, SearchHandle, find_in_files};
pub use fuzzy::{FuzzyMatch, FuzzyMatcher, FuzzyPattern, MatchKind, rank, score};
pub use index::{FileEntry, FileIndex, FileKind, IndexSnapshot, ScanHandle};
pub use mask::{FileMask, MaskError};
pub use query::{Query, QueryError, QueryOptions};
pub use replace::{ReplaceError, ReplaceOutcome, ReplaceTarget, TextEdit, compute_edits, replace_in_file, replace_in_files, replace_text};
