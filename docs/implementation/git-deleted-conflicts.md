# Deleted sides in Git conflicts

Selecting a deleted side of a modify/delete conflict now removes the file and stages the resolution. Selecting the modified side restores that side and stages it. Every selected path must be conflicted before any changes begin; an empty selection is a no-op. Literal pathspecs protect special filenames.

Implementation: `Repository::resolve_with` in `crates/rc-vcs/src/ops.rs`, adapted from CrabCraft's deletion-aware approach with additional validation. Both projects use MIT OR Apache-2.0.

Validation on November 8, 2026, against original upstream `main` commit `9e849cda99043fce68e8896d64a2608ff78141c8`: `cargo test -p rc-vcs --locked --target-dir /home/rafael-mendes/Documentos/dotcraft/target/upstream-git --jobs 2` passed 21 unit tests and 18 real-Git integration tests on Linux. The regression covers both deletion directions, both chosen sides, invalid mixed selections, special filenames, and preservation of unrelated edits.

This completes a backend conflict-resolution gap. It does not complete the backlog's visual merge editor. Resolution uses multiple Git commands and is not transactional if a later command fails.

Provenance: this port cherry-picks fork implementation commit `94aa39642060b81c54a56bf5a4abbfe73f79d423` and retained-notice commit `11b657e2f7dad4b5b7048ac09a48a195057faa5a`. The reused CrabCraft approach remains covered by [the retained MIT notice](crabcraft-MIT.txt). No images or other assets are added.
