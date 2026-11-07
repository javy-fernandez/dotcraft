# Git stash management

Open the Git tool window and select Stashes. Create a stash with a message, optionally include untracked files or keep staged changes, and apply, pop, or drop existing stashes. Restoring staged changes is optional. Pop and drop require confirmation; Cancel leaves the repository unchanged.

Operations execute asynchronously and refresh the stash list and working-tree status. Modified editor documents are saved first, and worktree-changing actions stop if any document remains modified. Selected stash hashes are checked against a fresh list before using backend indices. Concurrent app Git operations are serialized.

Validation: `cargo test -p rc-app --offline vcs_tests` passed two tests. Coverage includes real-Git push, list, apply, pop, drop, untracked files, stale indices, missing entries, and headless confirmation/cancellation interactions.

The numeric-index backend still permits an external Git process to change the stash list between lookup and mutation. This completes the stash UI scope; persistent changelists and a separate patch-shelf format remain pending.
