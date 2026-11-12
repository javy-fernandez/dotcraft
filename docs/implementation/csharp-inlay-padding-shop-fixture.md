# C-09: Inlay hint padding in the Shop .NET fixture

The editor tests use `tests/fixtures/Shop/src/Shop.Core/PriceCalculator.cs` at the `ThrowIfNull(order)` call. The layout test checks the rendered hint text with both `pad_left` and `pad_right`, then with neither flag. An offscreen `EditorView` test sends a right-arrow key event and verifies that the virtual hint does not change the logical caret position or the buffer text.

The audit found that the editor added requested padding spaces and then trimmed them away before layout. The hint now retains those spaces while the layout's thin-space markers remain unchanged.

Validation: run `CARGO_BUILD_JOBS=1 CARGO_TARGET_DIR=/tmp/dotcraft-editor-inlay-target cargo test -p rc-editor --test dotnet_inlay_hints` and `CARGO_BUILD_JOBS=1 CARGO_TARGET_DIR=/tmp/dotcraft-editor-inlay-target cargo test -p rc-editor --lib` separately. The `dotnet` SDK is unavailable in this workspace, so the C# project was not compiled. This evidence covers hint spacing and caret navigation in the tested editor scope, without claiming full Rider parity.
