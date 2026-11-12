# B-04: opening a folder with one standalone .NET project

## Behavior

When a folder contains exactly one top-level `.csproj`, `.fsproj`, or `.vbproj` and no solution file, `Workspace::open` recognizes the project. This lets the existing application project path receive the project without a separate folder scan. The folder path remains the session and recent-entry key, so existing folder sessions can still be restored. Opening the project file directly continues to use the project file as its key.

If a folder has one solution, the solution still takes precedence. Folders with multiple projects or multiple solutions remain plain folders.

## Validation

- `CARGO_TARGET_DIR=/tmp/dotcraft-core-project-folder-target cargo test -p rc-core` — 13 passed on Linux.
- Real minimal .NET project files and C# source verify folder and direct-file opening, project name, root, and persistence key for C#, F#, and Visual Basic project extensions.
- Multiple-project and multiple-solution fixtures verify ambiguous-folder behavior. A solution-plus-project fixture verifies solution precedence.
- `git diff --check` passed.

## Provenance and limits

This extends DotCraft's existing workspace discovery at upstream commit `becfb0fe316203df2d8d7c1e99c6a54bd37cd13b`; no external source or assets were copied. The application UI and actual SDK build/run flow were not exercised. Only top-level project files are considered for automatic selection; nested projects continue through existing folder discovery. This is a tested B-04 workspace-discovery scope, not full Rider parity.
