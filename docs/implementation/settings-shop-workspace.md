# Settings dialog on a .NET workspace (A-11)

Source baseline: `javy-fernandez/dotcraft` `becfb0f`. This adds acceptance coverage for the existing native egui settings dialog; no CrabCraft code or assets are copied.

With a real six-project `Shop.sln` open, the Editor page applies a tab-color choice and saves it to DotCraft settings. Reopening the dialog and cancelling another change leaves both the running and persisted values unchanged. The workspace remains open throughout.

Validation on Linux: `CARGO_TARGET_DIR=/tmp/dotcraft-zen-target cargo test -p rc-app settings_apply_and_cancel_with_shop_solution_open`. The headless egui UI harness clicks the dialog controls and reloads settings from its isolated test home.

This tests one Editor setting at a standard viewport. Other settings pages, compact layout, OS integrations and other platforms have separate or pending acceptance coverage.
