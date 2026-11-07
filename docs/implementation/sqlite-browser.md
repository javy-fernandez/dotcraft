# Native read-only SQLite browser

With a workspace open, select **Database** from a tool window stripe. Enter an existing SQLite file and click **Connect**. Relative paths resolve against the current workspace; absolute paths can point outside it. Connecting loads the list of user tables and views and their CREATE statements. Opening a workspace or merely entering a file path does not open a database or execute a query.

Choose a table and click **Browse Table** to view its first rows. Expand **Table Schema** for the selected table or view definition. **Refresh Schema** reloads metadata. The SQL console runs exactly one statement per **Run SQL** action. Connections accept reads only; write statements, transactions, ATTACH, PRAGMA commands, and extension loading are refused. Query parameters and persistent transactions are outside this slice.

## Worker and display bounds

All database path checks, connections, schema reads, and query execution run on a named worker thread. The UI polls a wakeable inbox without blocking. **Cancel Query** interrupts SQLite through its progress hook. **Disconnect**, workspace close, and dropping the database controller cancel pending work and discard late results. Cancellation of a database operation does not cancel an unrelated build, HTTP request, or debugger session.

Each operation uses a fresh read-only connection and a 150 ms lock timeout. SQL execution has a 10 second deadline checked through SQLite progress hooks. A connection session retains the selected file and schema in memory, rather than maintaining a SQL transaction across requests. External file changes may therefore appear on the next request; refresh schema after external DDL changes.

Results retain at most 500 rows and 2 MiB of cell payload and column names; truncation is explicit. SQL is limited to 64 KiB, statements to 256 columns, SQLite values to 1 MiB, and the schema list to 200 tables and views. Schema definitions retain up to 8,192 characters. SQLite expression, compound SELECT, bytecode, attached database, and worker thread limits also apply. A malformed file or unsupported query returns an error rather than panicking.

Result rows are virtualized with fixed heights. Columns can be resized and scrolled horizontally. Cells display one clipped line; hover previews retain up to 4,096 text characters or the first 32 BLOB bytes in hexadecimal. SQL NULL, integers, real values, text, and BLOB values remain distinct in the result model. Invalid UTF-8 text is refused. Browsing does not impose a sort order; only the displayed bounded prefix is available in this slice.

## Validation

`cargo test -p rc-app --offline sqlite_` exercises real temporary SQLite files and the native headless interface:

- Schema and view discovery, CREATE previews, Unicode, NULL, integer, and BLOB values, with unchanged database file bytes after reads.
- Rejection of writes, DDL, ATTACH, extension loading, transactions, multiple statements, and invalid table names.
- Row, byte, and SQL input limits; missing files are never created.
- Mid-query cancellation, deadlines, and discarded results after disconnect.
- Explicit Connect, Browse Table, Run SQL, Cancel Query, and Disconnect controls.
- Virtualized result rendering and access to the final row through scrolling.

These local SQLite tests establish this slice's behavior. They do not establish SQL editing assistance, other database engines, or Rider parity.

## Reuse and licensing

The worker bounds, identifier quoting, defensive connection configuration, progress-hook cancellation, and native result-grid approach are adapted selectively from [CrabCraft](https://github.com/RafaelMendes1407/CrabCraft), commit `475ade7b5ef0020c953f68a51f1b897380c1ea06`, particularly `crates/rc-app/src/database_core.rs` and `database.rs`. The new controller and UI omit that implementation's PostgreSQL, write, cell editing, import/export, and project trust integrations. No project code or external SQL CLI is started.

CrabCraft code is MIT OR Apache-2.0. Its [MIT notice](crabcraft-sqlite-MIT.txt) is retained alongside DotCraft's existing licenses. The pinned dependency `rusqlite` 0.40.1 uses bundled SQLite through `libsqlite3-sys`; rusqlite and libsqlite3-sys retain their published dependency licenses, and SQLite is public domain. No third-party UI assets were added.

## Remaining work

Writes and cell editing, paging beyond the bounded prefix, indexes and triggers as separate schema nodes, schema diagrams, SQL language assistance, query parameters, import/export, encrypted databases, PostgreSQL, and other engines remain separate features. Connection paths and query text are not saved to disk or automatically restored. No write controls are shown.
