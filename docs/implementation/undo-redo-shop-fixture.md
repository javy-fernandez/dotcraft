# C-12: Undo and redo in the Shop .NET fixture

The offscreen editor test opens `tests/fixtures/Shop/src/Shop.Core/PriceCalculator.cs`, sends an egui text event, then sends Rider keymap undo and redo shortcuts through `EditorView`. It checks text and caret restoration, diagnostic and inlay offsets, and clearing of stale search highlights. A new edit after undo must invalidate redo.

The audit found that regular edits moved diagnostic and inlay offsets while undo and redo left them stale. History edits now remap these anchors in application order and clear transient search highlights. The fixture regression exercises this behavior through the editor event path.

Validation: `CARGO_TARGET_DIR=/tmp/dotcraft-editor-undo-target cargo test -p rc-editor --lib --test dotnet_undo_redo`. The `dotnet` SDK is unavailable in this workspace, so the C# project was not compiled. This evidence covers the tested editor history behavior and does not claim full Rider parity.
