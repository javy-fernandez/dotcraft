# C-06: Automatic brackets in the Shop .NET fixture

The offscreen editor test opens `tests/fixtures/Shop/src/Shop.Core/PriceCalculator.cs` through `EditorState` and sends actual egui text events to `EditorView`. It checks paired parentheses in C# code, typing over the closing parenthesis, wrapping a selected expression, and undo. It also verifies that typing `(` at the end of the fixture's inline `//` comment inserts only `(`. The latter behavior was fixed by checking the current C# syntax context, with an inline-comment fallback while parsing catches up.

Run `CARGO_BUILD_JOBS=2 CARGO_TARGET_DIR=/tmp/dotcraft-editor-brackets-target cargo test -p rc-editor --test dotnet_brackets` for the fixture acceptance test. The `dotnet` SDK is unavailable in this workspace, so this run does not compile the fixture's C# project. Other languages and advanced C# lexical forms were not covered by this test.
