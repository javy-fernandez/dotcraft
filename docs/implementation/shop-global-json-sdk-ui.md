# Project SDK selection for Shop.Core on Linux

The B-20 acceptance test opens a temporary copied Shop.Core project in the native egui SDK chooser using official .NET SDK 8.0.428. It selects the installed .NET 8 SDK, saves `global.json`, verifies the selected version through the real CLI, and builds the project successfully. The earlier real SDK regression in PR #5 validates version switching and preservation of unrelated `global.json` policies; the existing settings test validates returning to the settings draft and cancellation.

Validation: `PATH=/path/to/dotnet:$PATH DOTNET_ROOT=/path/to/dotnet CARGO_TARGET_DIR=/path/to/dedicated-target cargo test -p rc-app shop_sdk_chooser_writes_global_json_and_builds_with_selected_sdk -- --ignored --nocapture` (1 passed on Linux). The test is ignored by default because it requires a local SDK and restored Shop packages.

This verifies project-local SDK pinning and a successful .NET 8 build from the native chooser. SDK installation, ancestor policy display, language-server restart, Windows/macOS and other runtime versions remain untested. The implementation uses original DotCraft code and its Shop fixture; no CrabCraft code or assets were copied.
