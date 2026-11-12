# C-11: C# switch-case indentation in the Shop .NET fixture

The offscreen editor test opens `tests/fixtures/Shop/src/Shop.Core/PriceCalculator.cs` and sends Enter through `EditorView` at the end of `case "SAVE10":` and `default:`. It checks that the new line starts one level deeper than the label and that undo restores the fixture. A unit guard verifies that comment text, ordinary labels, and a ternary colon do not take this special path.

The audit found that the generic newline command only added a level after `{`, leaving a statement after a C# `case` label aligned with the label. The editor now inserts the case-label newline and indentation in one undoable edit.

Validation: `CARGO_TARGET_DIR=/tmp/dotcraft-editor-indent-target cargo test -p rc-editor --lib --test dotnet_case_indent`. The `dotnet` SDK is unavailable in this workspace, so the C# project was not compiled. This evidence covers the tested C# switch-label indentation scope and does not claim full Rider parity.
