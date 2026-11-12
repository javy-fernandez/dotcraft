# C-04 signature help: active argument in the Shop fixture

## Behavior

The signature popup now selects the parameter under the editor caret. Moving across a top-level comma updates the highlight immediately, even before a new language-server response arrives. Commas inside nested calls, brackets, quoted strings, and comments do not advance the outer call's argument index. Moving beyond the closing parenthesis leaves the server's existing close behavior intact.

## Validation and provenance

The test uses `tests/fixtures/Shop/src/Shop.Core/PriceCalculator.cs`, including the `Math.Round(taxable * TaxRate, 2, MidpointRounding.AwayFromZero)` call. It drives right-arrow events through `EditorView` and checks the active parameter. A second test adds a nested `Math.Max` call to the fixture source and checks comma handling, quoted commas, and the closing parenthesis. No CrabCraft code or assets were copied.

Commands: `cargo test -p rc-editor --test signature_help_shop` and `cargo test -p rc-editor --lib` with a dedicated Cargo target directory.

## Remaining limitations

The tests supply signature metadata directly to the editor. They do not start Roslyn, verify the `textDocument/signatureHelp` response mapping, or cover the app popup request lifecycle. The local argument scanner does not parse every C# literal form, generic type argument, or named argument mapping; the broader C-04 feature remains partial.
