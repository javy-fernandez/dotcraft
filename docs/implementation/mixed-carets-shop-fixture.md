# C-07: Mixed selections and carets in the Shop .NET fixture

The offscreen editor test opens `tests/fixtures/Shop/src/Shop.Core/PriceCalculator.cs`, selects `subtotal` on one line, places a second caret at the end of another line, and sends an actual `(` text event to `EditorView`. It verifies that the selected expression is wrapped, the empty caret receives a pair, both ranges remain correctly placed, and one undo restores the full fixture.

The audit first reproduced data loss: the old command replaced the selected `subtotal` with `()` whenever another caret had no selection. The command now handles each range independently, preserving selected text while placing a paired or unpaired opening character at empty carets according to their context.

Validation: `CARGO_TARGET_DIR=/tmp/dotcraft-editor-carets-target cargo test -p rc-text --lib` and `CARGO_TARGET_DIR=/tmp/dotcraft-editor-carets-target cargo test -p rc-editor --lib --test dotnet_multi_carets`. The `dotnet` SDK is unavailable in this workspace, so the C# project was not compiled. This evidence covers the tested multiple-caret typing scope, without claiming full Rider parity.
