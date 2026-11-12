# Git fetch, pull, and push from the .NET workspace (I-15)

The Git branches menu can push a new local branch and establish its upstream, fetch a peer's commit, pull it into the workspace, and push a subsequent local commit. A headless egui acceptance test opens the real six-project `Shop.sln` fixture and exercises each menu action against a local bare Git remote. It verifies the remote commit hash and the branch's ahead/behind state after each operation.

The test also creates divergent local and peer C# commits. The UI reports a rejected non-fast-forward push without changing the remote. Fetch and Pull merge both commits, and a final Push brings the remote to the merged head.

Validation in this checkout's dedicated Cargo target directory:

- `cargo test -p rc-app shop_solution_fetch_pull_and_push_from_git_menu` — passed (1 test).
- `cargo test -p rc-vcs fetch_pull_push_with_progress` — passed (1 real-Git backend test).
- `git diff --check` — passed.

This slice uses DotCraft's existing native Rust/egui Git UI and Shop fixture. No CrabCraft code or assets are copied. The tested scope is local bare-remote synchronization on Linux, including upstream setup, ahead/behind updates, and non-fast-forward recovery. Hosted authentication, network failures and cancellation, large repositories, and other platforms remain untested.
