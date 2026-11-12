# Git worktrees in the .NET workspace (I-22)

DotCraft already offers local Git worktree list, add, and remove operations through Repository Tools. This slice adds headless egui acceptance with the real six-project `Shop.sln` fixture. The test creates a linked worktree at a path with spaces on a new branch, lists both worktrees, and checks that the linked directory contains the solution. It verifies the removal confirmation and that Git refuses to remove a dirty linked worktree or the active primary worktree. Finally, it removes the clean linked worktree and confirms the list contains only the primary one.

Validation in this checkout's dedicated Cargo target directory:

- `cargo test -p rc-app repository_tools_manage_shop_worktree_and_refuse_dirty_or_active_removal` — passed (1 real-Git UI test).
- Existing compiled rc-app test `repository_tools_destructive_confirmation_can_be_cancelled` — passed (1 UI test).
- `git diff --check` — passed.

An additional `rc-vcs` backend test build was attempted but stopped by `/tmp` disk quota while compiling dependencies. The existing backend test `worktrees_round_trip_and_refuse_removing_dirty_or_primary_worktrees` is unchanged by this PR; the new UI acceptance independently exercises its behavior through real Git.

This slice uses DotCraft's existing native Rust/egui Repository Tools and Shop fixture. No CrabCraft code or assets are copied. The tested scope is local Linux worktree creation, listing, and safe removal. Locked worktrees, symlinked paths, missing directories, network filesystems, and other platforms remain untested.
