# Deleted sides in Git conflicts

Selecting a deleted side of a modify/delete conflict now removes the file and stages the resolution. Selecting the modified side restores that side and stages it. Every selected path must be conflicted before any changes begin; an empty selection is a no-op. Literal pathspecs protect special filenames.

Implementation: `Repository::resolve_with` in `crates/rc-vcs/src/ops.rs`, adapted from CrabCraft's deletion-aware approach with additional validation. Both projects use MIT OR Apache-2.0.

Validation: `cargo test -p rc-vcs --offline` passed 21 unit tests and 18 real-Git integration tests. The regression covers both deletion directions, both chosen sides, invalid mixed selections, special filenames, and preservation of unrelated edits.

This completes a backend conflict-resolution gap. It does not complete the backlog's visual merge editor. Resolution uses multiple Git commands and is not transactional if a later command fails.
