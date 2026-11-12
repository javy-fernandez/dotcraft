# Shop launch profile selection on Linux

This slice validates F-08 for the native app's project run-configuration picker. The acceptance test copies the real `Shop.Api.csproj` and `launchSettings.json` into a temporary workspace, adds a second `Project` profile, and opens that project in DotCraft. It checks that both project profiles appear, the unsupported IIS Express profile is omitted, and selecting each profile from the toolbar produces a `RunRequest` with the matching profile name.

The toolbar's configuration button receives an accessible name so the selection can be driven through egui's accessibility tree. This small label change matches [PR 227](https://github.com/javy-fernandez/dotcraft/pull/227), which tests the same control for F-07; Git can coalesce the identical line when the PRs are applied.

Validation: `CARGO_TARGET_DIR=/var/tmp/dotcraft-zen-target CARGO_BUILD_JOBS=2 cargo test -p rc-app shop_launch_profiles_can_be_selected_from_the_toolbar -- --nocapture` (1 passed, Linux). The test uses the real Shop project files but does not launch the ASP.NET server. Live process behavior with an alternate profile, solution workspace selection, session restoration, other runtimes and platforms remain untested.

Source: existing DotCraft launch settings and runner code on original `main`. No CrabCraft code or assets were copied.
