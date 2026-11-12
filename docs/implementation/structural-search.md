# C# structural search and replace

## Behavior

- Code › Structural Search and Replace opens a native egui dialog. The query is parsed as a C# expression, statement, member, or type declaration. `$name$` placeholders capture syntax nodes, including nested expressions; repeated names must capture the same source text.
- Each placeholder can require a node category (identifier, expression, statement, or type) and a text regex. A replacement template substitutes captured source text for `$name$`.
- Search scans indexed `.cs` files on the background runtime, honoring the workspace file index and stopping when canceled. Results show source and replacement previews, with individual selection. The scan skips files larger than 2 MiB and caps results at 1,000.
- Replace selected checks all affected files against the versions read during the scan, rejects overlapping edits, then writes changed files atomically. Modified open documents block replacement.

## Validation

- Four `rc-search` tests cover expression, statement, member, and type-declaration shape matching, constraints, repeated captures, and replacement rendering.
- Focused `rc-app` tests cover conflict rejection, multiple edits in one file, and the headless dialog applying a selected result.
- `cargo check -p rc-app --locked --offline` passed on Linux with the existing `open_files::push` dead-code warning.

## Provenance and limits

The matcher and UI are original DotCraft code using the existing tree-sitter C# parser and workspace index. No CrabCraft code or assets were copied. MIT OR Apache-2.0 licensing applies.

Node constraints inspect syntax kinds and source text; they do not resolve symbols or types through Roslyn. The pattern parser handles C# constructs accepted by the tree-sitter grammar but does not model Rider's full structural-search language. A multi-file replacement preflights all source versions, but the file system cannot make all writes atomic together if a later write fails. Windows and macOS UI runs remain untested.
