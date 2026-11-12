# C-10: C# comments in the Shop .NET fixture

The offscreen editor test opens `tests/fixtures/Shop/src/Shop.Core/PriceCalculator.cs` and sends Rider keymap shortcuts to `EditorView`. It selects two C# statements, toggles `//` comments on and off, and checks undo. It also selects the `subtotal - discount` expression, toggles `/* ... */` on and off, checks that the delimiters stay selected for the second toggle, and checks undo.

The test exposed nested block comments on the second shortcut. The editor had kept only the original expression selected after adding the delimiters. It now keeps the full commented range selected so the next shortcut unwraps it.

Validation: `CARGO_TARGET_DIR=/tmp/dotcraft-editor-comments-target cargo test -p rc-editor --lib --test dotnet_comments` passes 17 library tests and two fixture acceptance tests. The `dotnet` SDK is unavailable in this workspace, so the C# project was not compiled. This evidence covers the tested editor shortcuts; it does not establish complete Rider parity.
