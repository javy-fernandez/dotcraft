# Recent Locations .NET source navigation (A-10)

Source baseline: `javy-fernandez/dotcraft` `becfb0f`. This slice is original DotCraft code and copies no CrabCraft code or assets.

The Recent Locations dialog now clamps its selection to the available entries and exposes each path and line number to accessibility tools. Enter opens the selected location in the editor.

Validation on Linux: `CARGO_TARGET_DIR=/tmp/dotcraft-zen-target cargo test -p rc-app recent_locations_selects_and_opens_a_dotnet_source_position`. The egui UI harness loads `Shop.sln`, opens a real C# source file, selects a prior location on line four and confirms the editor moves there.

This covers an open local C# file in one workspace. Closed files, deleted paths, native platform menus and larger histories remain outside this test.
