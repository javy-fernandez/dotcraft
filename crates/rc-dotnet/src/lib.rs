//! .NET integration: solutions and projects (`.sln`, `.slnx`, `.csproj`), and — in later
//! modules — SDK discovery, build/run/test, MSBuild and TRX parsing and NuGet.

pub mod project;
pub mod solution;

pub use project::ProjectInfo;
pub use solution::{ProjectRef, Solution, SolutionItem};
