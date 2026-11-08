# Workflow improvements: batch 2

This batch implements twenty additional scoped improvements on the consolidated
[upstream PR 108](https://github.com/javy-fernandez/dotcraft/pull/108) baseline. Delivery
is one new proposal from `RafaelMendes1407:feat/workflow-batch-02` to original
`javy-fernandez/dotcraft:main`. It does not update PR 108's source branch.

The baseline is remote commit `6cabba5bf8da8473172ddb5c8cc14dc0e93c9764`, whose Git tree
is identical to local validation commit `335d53d`. Until PR 108 merges, the new
proposal's diff against original `main` also includes that baseline. Review only
this batch using the [incremental comparison](https://github.com/RafaelMendes1407/dotcraft/compare/6cabba5bf8da8473172ddb5c8cc14dc0e93c9764...feat/workflow-batch-02).
After PR 108 merges, update this branch onto the resulting original main before final
review; a squash/rebase merge does not preserve the fork baseline ancestry automatically.
No default branch was merged and no release was published.

## Twenty selected scopes

| ID | Delivered behavior | Evidence |
| --- | --- | --- |
| C-18 | EditorConfig save-time whitespace, final newline, UTF-8 BOM and LF/CRLF rules | [Editor](batch2-editor.md) |
| C-23 | Workspace custom C# snippets: edit, preview, persist, insert, Tab expansion | [Editor](batch2-editor.md) |
| C-24 | Six syntax-guarded C# postfix templates for identifier/member paths | [Editor](batch2-editor.md) |
| D-16 | Cancellable indexed filename-family navigation | [Editor](batch2-editor.md) |
| D-17 | Persistent workspace favorite files | [Editor](batch2-editor.md) |
| I-09 | Explicit clone into a new directory and open the result | [Git](batch2-git.md) |
| I-17 | Single-commit cherry-pick with continuation and confirmed abort | [Git](batch2-git.md) |
| I-22 | List, create and safely remove linked worktrees | [Git](batch2-git.md) |
| I-23 | Recursive submodule status and recorded-commit update | [Git](batch2-git.md) |
| I-24 | Optional Git LFS status, pattern tracking and pull controls | [Git](batch2-git.md) |
| J-05 | Upstream HTTP Client cancellation and request cleanup | [Web](batch2-web.md) |
| J-06 | Background environment discovery, selection and refresh | [Web](batch2-web.md) |
| J-09 | Endpoint search, method filtering and grouping | [Web](batch2-web.md) |
| J-12 | Static method/route conflict warnings | [Web](batch2-web.md) |
| J-24 | OpenAPI 3 JSON import into a new HTTP request file | [Web](batch2-web.md) |
| J-28 | SQLite columns, indexes and foreign-key inspection | [Database](batch2-database.md) |
| J-30 | Local SQL keyword/table/column suggestions | [Database](batch2-database.md) |
| J-32 | Read-only comparison of complete SQLite schema snapshots | [Database](batch2-database.md) |
| J-33 | Native declared-foreign-key diagram with table navigation | [Database](batch2-database.md) |
| J-34 | SQLite EXPLAIN QUERY PLAN for a single read-only statement | [Database](batch2-database.md) |

Broader feature families remain partial in FEATURES.md. Each evidence note names
unsupported behavior; these bounded implementations do not establish Rider parity.
The implementation uses native Rust/egui. New worker operations are cancellable;
existing synchronous document/session persistence is reused by the editor slice.

## Review and provenance

Four independent work streams used isolated source checkouts and unique Cargo targets:
editor/integration, Git, database, and web. Database and Git slices received reciprocal
read-only reviews; the database reviewer also found the explicit-Save gap in the editor,
which was corrected and covered by an action-level regression with read-only protection.
A local JSON-RPC peer verifies that save-time edits reach the C# language server before
the save notification, with Unicode ranges, initialization gating and repeated-save checks.
Worktree removal confirmation explicitly states that ignored files are deleted.
Git LFS 3.8.1 tracking/status and actual local `file://` object download/hydration were
validated using a checksum-verified official binary installed only under `/tmp`.
All retained copyright notices and MIT OR Apache-2.0 licensing remain. No new external
code/assets were copied; the Git process-group guard declares the already locked libc
package directly. Existing reuse provenance remains in the earlier evidence notes.

English UI source strings use `tr`/`trf`. Four TSV manifests provide explicit English
fallbacks for seven locales. Integration preserves all existing translation values.
FEATURES.md is the status record and its generated counts are refreshed.

## Combined validation

Fresh combined results on Linux / Rust 1.99.1:

| Check | Result |
| --- | --- |
| All workspace test targets | Compiled successfully |
| Library tests across 11 crates | 256 passed |
| Selected application/backend/headless tests | 101 passed |
| Real-Git repository/tool regressions | 24 passed; optional LFS transfer explicitly exercised separately |
| Web grammar fixtures | 3 passed |
| Solution-filter screen regressions | 2 passed |
| Git LFS 3.8.1 tracking/status and local download | 2 passed, including one default-ignored transfer case |
| Locale/catalog and inventory checks | 116 new keys per locale, no existing translation changes; exactly 20 updated feature IDs |

This covers 387 distinct passing tests. The LFS tracking test runs both without and
with the optional extension, so its two executions count as one test in that total.
Runtime integration modules and GPU captures are excluded from the application selection.
The existing unused-import warning in `rc-kit/src/theme.rs` remains unchanged.

```sh
cargo test --workspace --no-run --locked --offline --target-dir /home/rafael-mendes/Documentos/dotcraft/target/upstream-integration --jobs 1
cargo test --workspace --lib --locked --offline --target-dir /home/rafael-mendes/Documentos/dotcraft/target/upstream-integration --jobs 1 -- --test-threads=1
cargo test --workspace --bins --locked --offline --target-dir /home/rafael-mendes/Documentos/dotcraft/target/upstream-integration --jobs 1 -- --skip insight_tests:: --skip runner_tests:: --skip debugger_tests:: --skip screens:: --test-threads=1
cargo test --workspace --test repo --test tools --test web --locked --offline --target-dir /home/rafael-mendes/Documentos/dotcraft/target/upstream-integration --jobs 1 -- --test-threads=1
cargo test --workspace --bins solution_filter_ --locked --offline --target-dir /home/rafael-mendes/Documentos/dotcraft/target/upstream-integration --jobs 1 -- --test-threads=1
env PATH=/tmp/dotcraft-git-lfs-v3.8.1:/home/rafael-mendes/.cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin cargo test --workspace --test tools lfs_ --locked --offline --target-dir /home/rafael-mendes/Documentos/dotcraft/target/upstream-integration --jobs 1 -- --include-ignored --nocapture
python3 scripts/features_summary.py
git diff --check
```

The .NET SDK/runtime suite, GPU acceptance, Windows/macOS execution and remote-service
authentication/transfers are not claimed. Historical live .NET results in earlier notes
remain historical and are not fresh validation of this batch.
