# C-05 quick documentation popup in the Shop fixture

## Behavior

The editor reports the F1 quick-documentation action at a C# symbol. A documentation popup anchored near the bottom or right edge now opens toward the viewport interior so its content stays visible; Escape closes it.

## Validation and provenance

The acceptance test loads `tests/fixtures/Shop/src/Shop.Core/PriceCalculator.cs`, positions the caret on `PriceBreakdown`, sends F1 through `EditorView`, supplies the summary from that C# file as documentation, inspects the painted egui text at all four viewport corners, and closes the popup with Escape. No CrabCraft code or assets were copied.

Commands: `cargo test -p rc-editor --test quick_doc_shop` and `cargo test -p rc-editor --lib` with a dedicated Cargo target directory.

## Remaining limitations

The test supplies the documentation to the editor popup directly. It does not start Roslyn or verify the app's `textDocument/hover` request and response mapping. Large documents may still exceed the viewport height; C-05 remains partial until the live application flow is accepted.
