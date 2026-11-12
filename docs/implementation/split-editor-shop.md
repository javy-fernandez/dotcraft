# Split editor acceptance on a .NET workspace (A-06)

Source baseline: `javy-fernandez/dotcraft` `becfb0f`. This adds acceptance coverage for existing DotCraft native egui split editors; no CrabCraft code or assets are copied.

The editor can show two open C# files in side-by-side or stacked panes. The Unsplit control returns to one pane while keeping both documents open.

Validation on Linux: `CARGO_TARGET_DIR=/tmp/dotcraft-zen-target cargo test -p rc-app split_dotnet_editors_render_and_unsplit_in_both_orientations`. The test invokes the tab split command for real `Shop.sln` source files, then checks both orientations and the Unsplit control in an egui UI harness.

This tests a standard desktop viewport. Divider drag, context-menu pointer interaction, narrow layouts, persistence and other platforms remain outside this acceptance slice.
