# Code Vision: Roslyn reference lenses

## Behavior

The C# editor requests `textDocument/codeLens` after edits settle. It resolves lenses that arrive without a command, keeps results only for the matching document version, and displays the returned reference labels above declarations. Clicking a label opens the existing usages view. **Show Code Vision** in Editor settings disables requests and clears visible labels.

The label uses the configured editor font size and theme colors. It is drawn at the right edge of the row above a declaration so line numbers, selection, and caret mapping do not move.

## Validation

- `cargo check -p rc-app --locked --offline` with compact profiles passed.
- `cargo test -p rc-lsp reference_code_lens_is_advertised_and_enabled` verifies the Roslyn capability and setting.
- No live Roslyn session was available for a visual or navigation smoke test.

## Provenance and remaining work

This is an original implementation using the existing LSP client. The setting name follows the [official C# extension configuration](https://github.com/dotnet/vscode-csharp/blob/main/package.json). No CrabCraft code or assets were copied.

Roslyn's reference lenses provide usage counts. Implementation and override counts, a dedicated row that never overlaps source text, and live visual validation remain for issue #117. This PR references the issue without closing it.
