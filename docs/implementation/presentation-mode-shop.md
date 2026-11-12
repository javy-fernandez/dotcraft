# Presentation mode with Shop.sln

The View menu toggles Presentation Mode. It enlarges the editor font to at least 24 points, requests fullscreen, and gives the editor the full work area. The dock layout stays in memory and is available again after exit. Changing the configured font while presenting updates the enlarged size; exit restores the configured size.

An offscreen egui test opens the real `Shop.sln` workspace and a `Shop.Core` C# file, uses the View menu to enter and exit, changes font settings while active, and verifies the editor font and preserved dock layout.

Validation: `CARGO_TARGET_DIR=/var/tmp/dotcraft-zen-target CARGO_BUILD_JOBS=2 cargo test -p rc-app presentation_mode_scales_shop_editor_and_preserves_dock -- --nocapture` on Linux.

OS fullscreen behavior, display scaling, and other platforms are not exercised by the offscreen harness. This implementation uses DotCraft code and the Shop fixture; no CrabCraft source or assets were copied.
