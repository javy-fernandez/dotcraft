# C-08: Code folding in the Shop .NET fixture

The offscreen editor test opens `tests/fixtures/Shop/src/Shop.Core/PriceCalculator.cs`, parses its C# fold ranges, and sends Rider Fold, Unfold, Fold All and Unfold All shortcuts through `EditorView`. It checks method folding, caret relocation to the visible fold start, expansion and unchanged buffer text.

The audit found that Fold All immediately unfolded a method when the caret was on its opening-brace line. That line remains visible after folding, so the editor now opens only enclosing folds that would actually hide the caret. The fixture test covers this case.

Validation: run `CARGO_BUILD_JOBS=1 CARGO_TARGET_DIR=/tmp/dotcraft-editor-fold-target cargo test -p rc-editor --test dotnet_folding` and `CARGO_BUILD_JOBS=1 CARGO_TARGET_DIR=/tmp/dotcraft-editor-fold-target cargo test -p rc-editor --lib` separately. The fixture test and 17 library tests passed. The `dotnet` SDK is unavailable in this workspace, so the C# project was not compiled. This evidence covers the tested C# folding shortcuts and does not claim full Rider parity.
