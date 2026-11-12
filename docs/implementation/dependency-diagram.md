# Project and package dependency diagram

## Behavior

- The Dependencies tool window renders projects and NuGet packages as cards in a scrollable node-and-edge diagram. Directed lines run from each project to its referenced projects and packages. Shared packages use one card with incoming lines from every referencing project.
- Project cards open their project file. Package cards focus the exact package for one referencing project in the existing NuGet tool window. Refresh rebuilds the graph after project-file edits.
- Loading project XML happens on a background task. The tool window shows a loading state while the graph is read and ignores responses from older loads and refreshes.
- The tool window is available from the stripe and View > Tool Windows, and its visibility and position persist with the existing dock session.

## Validation

- `CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0 cargo check -p rc-app --locked --offline --target-dir /home/rafael-mendes/Documentos/dotcraft/target/issue-129 --jobs 2` passed on Linux. The existing `open_files::push` dead-code warning remains.
- The dependency graph unit test validates project references, shared-package deduplication, and directed edges. The layout test places referenced projects and their package in successive columns.
- Both focused `rc-app` diagram tests passed, covering layout and stale-refresh responses.

## Provenance and limits

This is original DotCraft code using its existing egui theme, project parser, NuGet window, and background runtime. No third-party code or assets were copied. MIT OR Apache-2.0 licensing applies.

The graph reads direct references from project files. It does not resolve transitive NuGet dependencies, conditional MSBuild items, or version overrides from central package management. Package clicks select the first incoming project reference. Very large graphs may require scrolling in both directions.
