# Code Vision: Roslyn declaration lenses

## Behavior

The C# editor requests `textDocument/codeLens` after edits settle. It resolves lenses that arrive without a command and displays the returned reference labels above declarations. It also asks Roslyn for `textDocument/implementation` results for document symbols. Type and interface labels count implementations; virtual, abstract, and override members label those results as overrides. The current declaration is excluded from implementation counts. Each result is kept only for the matching document version. Clicking a label opens the existing usages or implementations view. **Show Code Vision** in Editor settings disables requests and clears visible labels.

Code Vision labels have a dedicated row before each declaration position, including when multiple declarations share a source line. Each count has a separate clickable label. Source text, carets, selections, and gutter marks share the same remapped source rows, so labels cannot cover code. Labels use the configured editor font size and theme colors.

## Validation

- Editor tests check that label rows precede declarations, including separate rows for declarations on one source line.
- App tests check implementation-location deduplication, excluding the current declaration, and syntax-tree override classification across split modifier lines.
- An opt-in Linux test with Roslyn 5.13 and .NET 10 checks live reference, implementation, and override counts; disabling and re-enabling the setting; invalidation after an edit; and navigation by clicking a Code Vision label. Its offscreen capture was inspected for overlap.
- `cargo check -p rc-app --locked --offline` with compact profiles and `cargo test -p rc-lsp reference_code_lens_is_advertised_and_enabled` validate integration and Roslyn capability.

## Provenance and limits

This is an original implementation using the existing LSP client. The setting name follows the [official C# extension configuration](https://github.com/dotnet/vscode-csharp/blob/main/package.json). No CrabCraft code or assets were copied.

Implementation requests are queued and sent at most eight at a time so all declarations can eventually receive counts without overwhelming Roslyn. Override classification reads C# syntax-tree modifier nodes after the document parse catches up; Roslyn supplies the target locations. A live visual check was run on Linux; Windows and macOS remain untested.
