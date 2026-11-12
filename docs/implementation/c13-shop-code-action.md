# C-13 code actions and quick fixes in Shop

## Behavior

DotCraft requests code actions at the caret and shows Roslyn's actions in its context-actions dialog. Selecting a fix with Enter applies a workspace edit to the open editor document. While that dialog is open, the editor releases keyboard focus so Enter does not also insert a newline into source code; focus returns to the editor when the dialog closes.

## Validation and provenance

The live regression opens the real `tests/fixtures/Shop/Shop.sln` and `PriceCalculator.cs` in the app, adds a `StringBuilder` reference without its namespace in the editor buffer, waits for Roslyn's CS0246 diagnostic, invokes Show Context Actions, selects the `System.Text` import fix with Enter, and checks that `using System.Text;` appears and CS0246 clears. It also checks that the original `StringBuilder builder = new();` line is intact, editor focus returns, and the source fixture on disk is unchanged. No CrabCraft code or assets were copied.

The live test passed on Linux with SDK 8.0.428, .NET runtime 10.0.15, and Roslyn 5.13.0-1.26523.5:

`DOTNET_ROOT=/var/tmp/dotcraft-dotnet DOTCRAFT_ROSLYN=<Roslyn-server> CARGO_TARGET_DIR=<dedicated-target> cargo test -p rc-app shop_roslyn_quick_fix_adds_missing_using -- --ignored --nocapture` — 1 passed.

## Scope and limitations

This verifies a Roslyn import quick fix for an open Shop C# document on Linux. It does not cover every code-action kind, multi-file resource operations, or UI discoverability beyond the context-actions dialog.
