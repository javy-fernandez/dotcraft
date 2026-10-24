//! Search without UI: the workspace file index (walked with `ignore`, kept up to date from
//! file-system events) and IntelliJ-like fuzzy ranking for Go to File and other lists.

pub mod files;
pub mod fuzzy;
pub mod index;

pub use files::{FileMatch, FileQuery, FileSearchRequest, FileSearchResults, FileSearchWorker, search_files};
pub use fuzzy::{FuzzyMatch, FuzzyMatcher, FuzzyPattern, MatchKind, rank, score};
pub use index::{FileEntry, FileIndex, FileKind, IndexSnapshot, ScanHandle};
