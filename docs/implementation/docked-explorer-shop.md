# Docked Explorer in a .NET solution (A-04)

Source baseline: `javy-fernandez/dotcraft` `becfb0f`. This adds acceptance coverage for the existing native egui dock implementation; no CrabCraft code or assets are copied.

With real `Shop.sln` open, the Explorer stripe button hides and reopens its tool window. Moving Explorer from the left to right dock preserves its visibility and round-trips through the workspace dock session map.

Validation on Linux: `CARGO_TARGET_DIR=/tmp/dotcraft-zen-target cargo test -p rc-app explorer_dock_toggles_and_moves_with_shop_solution_open`. The headless egui UI harness exercises the stripe control and validates dock state and serialization.

This tests one tool window at a standard viewport. Pointer drag, context-menu Move To, compact stripe overflow, multi-window layouts and other platforms remain outside this acceptance slice.
