# C-23: Live-template context in the Shop .NET fixture

The offscreen editor tests open `tests/fixtures/Shop/src/Shop.Core/PriceCalculator.cs` and send text and Tab events through `EditorView`. `cw` expands to `Console.WriteLine();` in a C# code line and undo restores the fixture. The same abbreviation remains literal when typed in an inline `//` comment or an ordinary C# string.

The audit found that template expansion checked surrounding punctuation but not whether the caret was in code. The editor now rejects comment and string contexts using the syntax tree when available and a current-line lexical check while parsing catches up.

Validation: run `CARGO_BUILD_JOBS=1 CARGO_TARGET_DIR=/tmp/dotcraft-editor-template-target cargo test -p rc-editor --test dotnet_live_templates` and `CARGO_BUILD_JOBS=1 CARGO_TARGET_DIR=/tmp/dotcraft-editor-template-target cargo test -p rc-editor --lib` separately. The `dotnet` SDK is unavailable in this workspace, so the C# project was not compiled. This evidence covers the tested built-in abbreviation and context guard; the application template popup, custom templates, advanced C# raw/verbatim strings, and lines over 4,096 characters remain outside the tested scope.
