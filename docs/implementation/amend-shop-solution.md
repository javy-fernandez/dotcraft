# Amend and save-before-commit in the .NET workspace (I-12)

The Commit window saves open editor buffers immediately before dispatching Commit or Amend. Previously, `Vcs::commit` then selected paths from its cached status, which could predate that save. A newly saved C# edit could therefore be omitted from an amended commit. The operation now reads fresh Git status on its background task before choosing unexcluded paths. An empty selection still reports `NothingToCommit`, while a message-only Amend remains possible when the worktree is clean.

A headless egui acceptance test opens the real six-project `Shop.sln` fixture, edits a C# buffer without saving it, excludes a second modified C# file, and uses the Amend checkbox and Commit button. It confirms HEAD changes without adding another commit, the editor edit appears in HEAD, the excluded file stays outside HEAD, and the editor is saved. The test then makes another unsaved C# edit and verifies a regular Commit includes it as a second commit.

Validation in this checkout's dedicated Cargo target directory:

- `cargo test -p rc-app amend_saves_csharp_buffer_and_rewrites_only_selected_commit_paths` — passed (1 test).
- `cargo test -p rc-app git_changes_marks_diff_commit_branch_and_blame` — passed (1 existing app test).
- `cargo test -p rc-vcs commit` — passed (2 real-Git backend tests).
- `git diff --check` — passed.

This slice uses DotCraft's existing native Rust/egui Commit window and Shop fixture. No CrabCraft code or assets are copied. The tested scope is local Linux commit amendment and save-before-commit for C# files. Interactive rebase is tracked separately as I-16; signed commits, protected remote histories, and other platforms remain untested.
