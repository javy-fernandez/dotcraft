# Light and dark themes in a .NET workspace (A-02)

Source baseline: `javy-fernandez/dotcraft` `becfb0f`. This slice adds acceptance coverage for existing DotCraft native egui themes and copies no CrabCraft code or assets.

The application applies light and dark theme choices to the live egui context without closing the workspace. The test switches from light to dark and back while a real six-project `Shop.sln` remains open.

Validation on Linux: `CARGO_TARGET_DIR=/tmp/dotcraft-zen-target cargo test -p rc-app dotnet_workspace_switches_between_light_and_dark_themes`. The headless app harness verifies the active egui visual mode after each switch.

This checks explicit theme choices in a standard viewport. System-theme notifications, native menu/chrome appearance, accessibility contrast and other platforms remain untested.
