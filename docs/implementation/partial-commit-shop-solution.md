# Partial Git commit in the .NET workspace (I-21)

Repository Tools now lists numbered unstaged text hunks for a repository-relative file, stages one chosen hunk using DotCraft's existing `rc-vcs` patch backend, and offers an explicit Commit staged action. Commit staged uses the Git index only; it does not add unstaged files or editor buffers. Both Stage hunk and Commit staged refuse to run while an editor document is unsaved. The hunk preview is limited to 100 hunks and four changed lines per hunk, and tool output is capped at 65,536 characters.

A headless egui acceptance test opens the real six-project `Shop.sln` fixture. It makes two separated changes to `Order.cs`, lists and stages only the first hunk, and checks the index contains only that hunk. It then verifies an unsaved `PriceCalculator.cs` editor buffer blocks Commit staged. After saving, it commits the index and checks that HEAD contains only the selected Order hunk; the second Order hunk and PriceCalculator edit remain in the worktree. A second Commit staged with no staged changes reports an error and does not create another commit.

Validation in this checkout's dedicated Cargo target directory:

- `cargo test -p rc-app repository_tools_commit_only_selected_shop_csharp_hunk` — passed (1 real-Git UI test).
- `cargo test -p rc-app repository_tools_destructive_confirmation_can_be_cancelled` — passed (1 existing UI test).
- `cargo test -p rc-vcs stage_unstage_and_discard_hunks_and_lines` — passed (1 existing real-Git backend test).
- `git diff --check` — passed.

This slice uses DotCraft's existing native Rust/egui UI, Shop fixture, and hunk staging backend. No CrabCraft code or assets are copied. The tested scope is local Linux text-hunk partial commits. Line-level selection is available only in the backend; binary files, renames, concurrent index changes, and other platforms remain untested.
