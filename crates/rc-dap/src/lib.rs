//! Debugging .NET programs through the Debug Adapter Protocol with netcoredbg.
//!
//! - [`netcoredbg`]: finding or downloading the adapter (pinned release).
//! - [`client`]: the DAP client over stdio, non-blocking ([`Pending`] polled per frame).
//! - [`session`]: [`DebugSession`], the state machine the UI owns: build, launch/attach,
//!   handshake, breakpoints, stops, frames, variables, watches, evaluation.
//! - [`launch`]: what to debug ([`DebugTarget`]) and the adapter's launch arguments.
//! - [`breakpoints`]: breakpoints by file and line, hit conditions and logpoints.

pub mod breakpoints;
pub mod client;
pub mod launch;
pub mod netcoredbg;
pub mod protocol;
pub mod session;

pub use breakpoints::{BreakpointSpec, BreakpointState, Breakpoints};
pub use client::{DapClient, DapError, Incoming, Pending};
pub use launch::{DebugOptions, DebugTarget, LaunchConfig};
pub use netcoredbg::{AdapterCommand, DownloadEvent};
pub use protocol::{EvalContext, ExceptionInfo, Scope, StackFrame, Thread, Variable};
pub use session::{DebugEvent, DebugSession, EvalValue, Failure, OutputCategory, SessionState, StopInfo, StopReason, ThreadFrames, Watch, WatchValue};
