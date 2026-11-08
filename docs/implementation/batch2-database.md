# SQLite inspection and query assistance (batch 2)

This slice implements five native SQLite workflows in the existing Database tool window.
Connections remain read-only and workers remain cancellable. It builds on the first
combined feature batch; it does not replace the upstream HTTP or other tool windows.

| Inventory ID | Implemented and tested scope | Remaining scope |
| --- | --- | --- |
| J-28 | Schema Details explicitly loads columns (including generated/hidden columns), primary-key positions, defaults, index uniqueness/origin/columns, and foreign-key actions for a selected SQLite table or view. | Other engines, trigger editing, object editing, and persistent connections. |
| J-30 | SQL Suggestions inserts local SQLite keywords, loaded table names, and columns from the last inspected table at the caret. Identifiers are quoted; Unicode offsets and token replacement are supported. Comments and quoted text suppress suggestions. | Semantic validation, inspections, joins/alias inference, automatic popup triggers, and other engines. |
| J-32 | Schema Comparison reads two explicitly selected SQLite files and reports added, missing, and changed tables, views, indexes, and triggers with before/after SQL definitions. | SQL normalization, migration generation, concurrent cross-file snapshots, and other engines. |
| J-33 | Relationships renders a scrollable native egui graph from actual declared SQLite foreign keys, including self references, with accessible table buttons and a textual link list. Clicking a table opens its inspection. | Automatic layout optimization, editable diagrams, inferred relations, and other engines. |
| J-34 | Explain Query Plan prepares one read-only SQL statement, displays SQLite's planner rows, and identifies index use without executing that statement. | Measured execution statistics, cost visualization, statement parameters, and other engines. |

## Behavior and resource limits

Opening the Database window or changing tabs does not open another file or issue SQL.
Connect, Inspect Table, Load Relationships, Compare Schemas, Run SQL, and Explain Query
Plan are explicit operations. Paths may be absolute or relative to an open workspace.
The comparison opens the second file independently with read-only SQLite flags; it never
uses ATTACH and never creates missing files. Cancellation and workspace disconnect discard
late replies. SQL and filesystem work stays on the existing worker thread.

The existing defensive connection settings, query bounds, 10-second deadline, and progress
callback are retained. Only four read-only metadata pragmas (`table_xinfo`, `index_list`,
`index_info`, and `foreign_key_list`) are added to the authorizer allowlist. Mutating
pragmas, writes, DDL, transactions, extension loading, and ATTACH remain denied.

Metadata query results retain the existing 500-row/2-MiB limit per grid. Diagram discovery
is limited to 200 tables, 500 foreign-key column links, and 2 MiB, with visible truncation.
Schema comparison requires complete bounded snapshots: if either schema exceeds 500
objects or 2 MiB, it fails explicitly rather than showing an incomplete comparison as
matching. The two schemas are read at separate instants and are compared by exact stored
DDL, so cosmetic SQL differences are reported. Definition previews are capped at 8192
characters. Nothing is written to either database.

Suggestions are computed from already loaded names, with at most 24 choices and the
existing SQL input limit. They are not a SQL parser or proof of query correctness. Column
suggestions come from the inspected table, not alias inference. Suggestion chips wrap and
scroll in compact panels; selecting one never sends a query.

## Validation

On Linux with Rust 1.99.1, in the isolated `/tmp/dotcraft-upstream-attach` checkout:

```sh
CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 cargo test -p rc-app sqlite_ --locked --offline --target-dir /home/rafael-mendes/Documentos/dotcraft/target/upstream-attach --jobs 1 -- --test-threads=1
```

The suite passes 15 tests: seven retained SQLite/backend/UI regressions and eight new
backend/headless regressions. Real temporary SQLite databases verify quoted names,
generated columns, indexes, foreign-key actions, schema additions/removals/changes,
complete-snapshot bounds, graph bounds, indexed query plans, cancellation/deadlines,
Unicode and mid-token suggestions, and unchanged file bytes. The headless workflow clicks
through connect, inspect, graph loading and node navigation, comparison, query plans,
suggestion insertion, and disconnect. It also proves tab changes do not execute work.

No external database service, SDK, or network request is required. Windows/macOS and other
engines were not exercised. New English UI strings are supplied as explicit fallback rows
in `scripts/i18n/batch2-database.tsv`; the batch integrator imports them into the catalogs.

## Provenance

The existing SQLite implementation and retained CrabCraft notice remain in
[sqlite-browser.md](sqlite-browser.md) and [crabcraft-sqlite-MIT.txt](crabcraft-sqlite-MIT.txt).
The new metadata, comparison, completion, graph UI, and planner integration are original
DotCraft changes using the already pinned bundled rusqlite dependency. No new third-party
code, dependencies, artwork, or proprietary assets were added.
