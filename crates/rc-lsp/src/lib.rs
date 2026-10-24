//! Language Server Protocol client: JSON-RPC over stdio with cancellation, incremental
//! document sync, semantic token decoding, and the Roslyn C# server adapter.

pub mod roslyn;
pub mod rpc;
pub mod semantic;
pub mod session;
pub mod sync;

pub use lsp_types;
pub use rpc::{Client, Incoming, Pending, RpcError};
pub use session::{LanguageSession, LspEvent, ServerState};
pub use sync::{path_to_uri, uri_to_path};
