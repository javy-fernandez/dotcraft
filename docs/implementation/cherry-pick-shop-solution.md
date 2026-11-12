# Cherry-pick in the .NET workspace (I-17)

The Git log now offers Cherry-pick for a selected commit. DotCraft resolves the revision to a commit hash before invoking Git, runs the operation away from the UI thread, and supports cancellation through the existing operation handle. A conflict refreshes Git state; the branches menu then offers Abort Cherry-pick, which restores the pre-operation HEAD and worktree.

A headless egui acceptance test opens the real six-project `Shop.sln` fixture. It selects a commit from a topic branch, cherry-picks it onto main, and checks the resulting C# file and commit subject. It then selects a conflicting C# commit, verifies `CherryPicking` state and an error, aborts from the branches menu, and checks that HEAD and the main branch's file content are preserved.

Validation in this checkout's dedicated Cargo target directory:

- `cargo test -p rc-app cherry_pick_shop_commit_and_abort_conflict_from_git_ui` — passed (1 real-Git UI test).
- `cargo test -p rc-app branches_popup_filters_and_shows_branch_actions` — passed (1 existing UI test).
- `cargo test -p rc-vcs --test branches` — passed (5 real-Git backend tests, including apply, conflict/abort, and a pre-cancelled operation).
- `git diff --check` — passed.

This slice uses DotCraft's existing native Rust/egui Git UI and Shop fixture. No CrabCraft code or assets are copied. The tested scope is local Linux cherry-pick and conflict abort. Conflict resolution and continuation, cancellation of a long-running Git process, merge commits, and other platforms remain untested.
