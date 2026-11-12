# Code Vision: Roslyn declaration lenses

## Behavior

The C# editor requests `textDocument/codeLens` after edits settle. It resolves lenses that arrive without a command and displays the returned reference labels above declarations. It also asks Roslyn for `textDocument/implementation` results for document symbols. Type and interface labels count implementations; virtual and abstract members label those results as overrides. Each result is kept only for the matching document version. Clicking a label opens the existing usages or implementations view. **Show Code Vision** in Editor settings disables requests and clears visible labels.

Code Vision labels have a dedicated row before each declaration, with separate clickable labels for each count. Source text, carets, selections, and gutter marks share the same remapped source rows, so labels cannot cover code. Labels use the configured editor font size and theme colors.

## Validation

- Focused editor tests check that label rows precede declarations and do not duplicate source rows.
- Focused app tests check implementation-location deduplication and override classification.
- `cargo check -p rc-app --locked --offline` with compact profiles and `cargo test -p rc-lsp reference_code_lens_is_advertised_and_enabled` validate integration and Roslyn capability.

## Provenance and remaining work

This is an original implementation using the existing LSP client. The setting name follows the [official C# extension configuration](https://github.com/dotnet/vscode-csharp/blob/main/package.json). No CrabCraft code or assets were copied.

At most 64 implementation requests are issued per document refresh to bound Roslyn work. Override classification relies on declaration text and can misclassify split modifier lines or interface implementations. No live Roslyn visual or navigation smoke test was available, so this slice references #117 without closing it.
