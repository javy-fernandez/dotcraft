# Recent Files filter and selection (A-09)

Source baseline: `javy-fernandez/dotcraft` `becfb0f`. This is original DotCraft code and copies no CrabCraft code or assets.

The Recent Files dialog keeps the selected row inside the filtered list. An old selection beyond the new result count now falls back to the last available row, so Enter opens the visible result. Rows also expose their file names and selection state to accessibility tooling.

Validation on Linux: `CARGO_TARGET_DIR=/tmp/dotcraft-zen-target cargo test -p rc-app recent_files_filter_clamps_selection_and_opens_a_dotnet_file`. The egui UI harness opens three C# files from the real `Shop.sln` fixture, filters down to one, checks the accessible row and bounded selection, then opens it with Enter.

This covers recently opened local files in a .NET workspace. Missing/deleted files, cross-platform native menus, and very large recent lists remain outside this test.
