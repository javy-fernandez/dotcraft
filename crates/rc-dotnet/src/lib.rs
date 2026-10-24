//! .NET integration: solutions and projects (`.sln`, `.slnx`, `.csproj`), SDK discovery,
//! evaluated MSBuild properties, launch profiles, build/run/test with streamed output and
//! parsed diagnostics, TRX results and NuGet.
//!
//! Long operations run on [`rc_core::Runtime`] and report through an [`rc_core::inbox::Outbox`]
//! (`start_build`, `start_run`, `start_tests`); their handles stop the work (and kill the
//! process tree) on `cancel()`/`stop()` or when dropped. The `async fn` variants take a
//! [`CancelToken`] and a callback instead.

pub mod diagnostics;
pub mod launch;
pub mod msbuild;
pub mod process;
pub mod project;
pub mod sdk;
pub mod solution;

pub use diagnostics::{Diagnostic, Severity};
pub use launch::{LaunchProfile, LaunchSettings};
pub use msbuild::ProjectProperties;
pub use process::{CancelToken, OpHandle, OutputLine, Stream, kill_tree, strip_ansi};
pub use project::ProjectInfo;
pub use sdk::{Dotnet, RuntimeInfo, SdkInfo};
pub use solution::{ProjectRef, Solution, SolutionItem};
