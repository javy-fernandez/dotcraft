# C-25: Surround With in the Shop .NET fixture

The offscreen editor test opens `tests/fixtures/Shop/src/Shop.Core/PriceCalculator.cs`, selects two C# statements, sends the Rider Surround With shortcut through `EditorView`, and applies the built-in `if` template as the application would. It verifies indentation, snippet-field selection, and undo restoring the fixture.

The audit found that applying a template to a read-only editor could create a snippet session even though the text replacement was refused. `insert_snippet` now returns before creating fields in a read-only document. A second fixture test verifies unchanged text, selection, and snippet state.

Validation: run `CARGO_BUILD_JOBS=1 CARGO_TARGET_DIR=/tmp/dotcraft-editor-surround-target cargo test -p rc-editor --test dotnet_surround` and `CARGO_BUILD_JOBS=1 CARGO_TARGET_DIR=/tmp/dotcraft-editor-surround-target cargo test -p rc-editor --lib` separately. The `dotnet` SDK is unavailable in this workspace, so the C# project was not compiled. This evidence covers one built-in template and the editor action path; the application popup flow and other templates remain outside the tested scope.
