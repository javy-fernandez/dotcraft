# Folder and solution workspace opening (B-01)

Source baseline: `javy-fernandez/dotcraft` `becfb0f`. This adds acceptance coverage for the existing DotCraft app and copies no CrabCraft code or assets.

Opening the `Shop/src` folder creates a folder workspace and discovers a runnable Shop.Cli configuration. Opening `Shop.sln` then loads six projects as a distinct solution workspace; both paths remain in recent history. The related [symlink-tree PR](https://github.com/javy-fernandez/dotcraft/pull/175) hardens folder navigation in `rc-core`.

Validation on Linux: `CARGO_TARGET_DIR=/tmp/dotcraft-zen-target cargo test -p rc-app shop_folder_and_solution_open_as_distinct_desktop_workspaces`. The headless egui app harness uses the real Shop fixtures and renders both workspaces.

SDK build/run, link traversal through the Explorer UI and other platforms remain outside this acceptance slice.
