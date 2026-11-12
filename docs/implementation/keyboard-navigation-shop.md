# Keyboard navigation in the Shop workspace

An offscreen egui acceptance test starts on Welcome with the real `Shop.sln` in Recent Projects. It focuses that accessible row and presses Enter to open the solution, then focuses the Explorer stripe control and presses Space to hide and reopen it. The test checks the workspace and dock state after each action.

Validation: `CARGO_TARGET_DIR=/var/tmp/dotcraft-zen-target CARGO_BUILD_JOBS=2 cargo test -p rc-app keyboard_opens_recent_shop_and_toggles_explorer -- --nocapture` on Linux. The test passed again after temporary disk quota pressure was cleared.

This verifies two keyboard paths. Editor tabs, dialogs, menus, screen readers, and other platforms require further accessibility acceptance. It uses existing DotCraft behavior and the Shop fixture; no CrabCraft source or assets were copied.
