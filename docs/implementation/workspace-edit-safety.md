# Workspace edit safety

Workspace edits containing file creation, renaming, or deletion are rejected before any text edits are applied. The language server receives `applied: false` and a failure reason, and the user sees an error. Code action commands do not execute after a rejected edit. Client capabilities no longer advertise unsupported resource operations.

Validation on November 8, 2026, against original upstream `main` commit `9e849cda99043fce68e8896d64a2608ff78141c8`: `cargo test -p rc-app --locked --offline --target-dir /home/rafael-mendes/Documentos/dotcraft/target/upstream-git --jobs 2 workspace_ -- --nocapture` passed two tests on Linux. Mixed edits containing each resource operation leave both an open buffer and a closed file unchanged; ordinary text-only operations still apply successfully.

This prevents partially applying refactorings that contain unsupported resource operations. Transactional resource edits, global undo, version validation, and broader rollback support remain pending.

The matching `cargo test -p rc-app -p rc-kit --locked --offline --target-dir /home/rafael-mendes/Documentos/dotcraft/target/upstream-git --jobs 2 i18n::tests` run passed all five translation invariant checks. The locale merge preserves every existing upstream translation and adds one English fallback entry. Upstream call/type hierarchy code remains intact.

Provenance: ported from fork implementation `3dd669f404bebac0df582f1ff560958592235e7d` and scope clarification `ff4c8734f38ae41a09b79a36a5ba3dd2f0267f39` (fork PR 10). No external assets or new dependencies are added. This targeted validation does not establish multi-file transactional undo, live Roslyn refactoring coverage, or Windows/macOS execution.

The changed LSP crate also passed all seven unit tests with `cargo test -p rc-lsp --lib --locked --offline --target-dir /home/rafael-mendes/Documentos/dotcraft/target/upstream-git --jobs 2`.
