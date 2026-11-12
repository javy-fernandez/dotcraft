# Shelve and restore changes in a .NET solution (I-11)

DotCraft's Git stash view saves tracked and untracked changes and can apply, pop, or drop a stash. This slice adds headless UI acceptance with the real `Shop.sln` fixture. The test edits a C# document in the editor, creates an untracked C# file, then uses the Git tool window to stash both changes. It confirms the editor content was saved before stashing, the worktree returned to its base state, and Pop required confirmation before restoring both files.

The existing implementation uses DotCraft's native Rust/egui UI and `rc-vcs` Git CLI backend. No CrabCraft code or assets are copied.

Validation in this checkout's dedicated Cargo target directory:

- `cargo test -p rc-app shelves_save_and_restore_shop_csharp_changes_from_the_git_window` — passed (1 test).
- `cargo test -p rc-app stash_actions_refresh_and_keep_the_selected_entry` — passed (1 test).
- `cargo test -p rc-vcs stash` — passed (1 real-Git backend test).

The existing backend and app tests also cover stash identity, apply/pop/drop, index options, and stale selection. This slice does not copy CrabCraft code or assets; the Shop fixture and Git UI were already present in DotCraft.

The tested scope is local Git shelves for a .NET workspace on Linux. Remote synchronization, multiple simultaneous worktrees, merge conflicts during restore, and other platforms remain outside this acceptance test.
