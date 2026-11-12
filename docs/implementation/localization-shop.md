# Live localization in the Shop workspace

An offscreen egui acceptance test opens the six-project `Shop.sln`, finds the Explorer control in English, and applies Portuguese, Spanish, and English language settings in sequence. It verifies that the control's accessible name is relabeled each time while the workspace remains open. The selected locale remains in app settings.

Validation: `CARGO_TARGET_DIR=/var/tmp/dotcraft-zen-target CARGO_BUILD_JOBS=2 cargo test -p rc-app language_switch_relabels_open_shop_workspace -- --nocapture` on Linux.

This is scoped to live relabeling of the Explorer control. Other UI surfaces, locale completeness, right-to-left layout, native menus, and other platforms are not covered. It uses existing DotCraft translations and the Shop fixture; no CrabCraft source or assets were copied.
