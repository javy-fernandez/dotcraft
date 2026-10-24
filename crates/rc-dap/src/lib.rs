//! Debugging .NET programs through the Debug Adapter Protocol with netcoredbg.
//!
//! - [`client`]: the DAP client over stdio, non-blocking ([`Pending`] polled per frame).

pub mod client;
pub mod protocol;

pub use client::{DapClient, DapError, Incoming, Pending};
pub use protocol::{EvalContext, ExceptionInfo, Scope, StackFrame, Thread, Variable};
