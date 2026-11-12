# Notification status and expiry (A-15)

Source baseline: `javy-fernandez/dotcraft` `becfb0f`. This slice adds acceptance coverage for existing native egui notifications and copies no CrabCraft code or assets.

The status bar presents the newest notification. Informational messages expire after eight seconds; errors remain for fifteen seconds. The test controls the egui clock so both paths are checked deterministically.

Validation on Linux: `CARGO_TARGET_DIR=/tmp/dotcraft-zen-target cargo test -p rc-app dotnet_workspace_notifications_show_and_expire_by_severity`. The headless UI harness loads real `Shop.sln`, checks that an error is visible and verifies severity-based expiry.

This covers transient status-bar notices in a normal workspace. Notification history, compact layouts, other platforms and Zen Mode behavior are outside this acceptance slice; Zen Mode expiry is separately covered in [PR 161](https://github.com/javy-fernandez/dotcraft/pull/161).
