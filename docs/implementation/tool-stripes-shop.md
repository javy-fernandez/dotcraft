# Tool window stripes in a .NET solution (A-03)

Source baseline: `javy-fernandez/dotcraft` `becfb0f`. This adds acceptance coverage for existing DotCraft native egui stripes; no CrabCraft code or assets are copied.

With real `Shop.sln` open, the left stripe toggles a bottom tool window (Problems) and the right stripe toggles Notifications. The existing overflow-scroll test verifies that the last window remains reachable in a compact stripe.

Validation on Linux: `CARGO_TARGET_DIR=/tmp/dotcraft-zen-target cargo test -p rc-app shop_workspace_stripes_open_bottom_and_right_windows` and `cargo test -p rc-app overflowing_stripes_scroll_to_and_toggle_the_last_tool`. The headless egui harness checks both side mappings and toggle state.

This covers normal and compact stripes. Pointer context menus, moving windows through the stripe menu, native platform chrome and other platforms remain outside this acceptance slice.
