# Editor tabs on a .NET workspace (A-05)

Source baseline: `javy-fernandez/dotcraft` `becfb0f`. This slice is original DotCraft code and copies no CrabCraft code or assets.

The tab strip activates an open file and closes a selected tab while retaining the remaining file. Custom tab close controls now have accessible names with the document title, using the translated `Close Tab` source string.

Validation on Linux: `CARGO_TARGET_DIR=/tmp/dotcraft-zen-target cargo test -p rc-app dotnet_editor_tabs_activate_and_close_with_accessible_controls`. The egui UI harness loads `Shop.sln`, opens two C# files, activates one through its tab and closes it through the accessible control.

This checks ordinary tabs at a standard desktop width. Pinned/preview tabs, drag-and-drop ordering, compact widths, and native platform chrome remain outside this test.
