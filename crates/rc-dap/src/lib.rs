//! Debugging .NET programs through the Debug Adapter Protocol with netcoredbg.
//!
//! - [`netcoredbg`]: finding or downloading the adapter (pinned release).
//! - [`client`]: the DAP client over stdio, non-blocking ([`Pending`] polled per frame).

pub mod client;
pub mod netcoredbg;
pub mod protocol;

pub use client::{DapClient, DapError, Incoming, Pending};
pub use netcoredbg::{AdapterCommand, DownloadEvent};
pub use protocol::{EvalContext, ExceptionInfo, Scope, StackFrame, Thread, Variable};
