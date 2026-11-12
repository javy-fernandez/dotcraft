# Standalone .NET project workspace acceptance (B-04)

Source baseline: `javy-fernandez/dotcraft` `becfb0f`. This is test coverage for the existing DotCraft app path and copies no CrabCraft code or assets.

Opening a folder containing the standalone `Shop.Cli.csproj` creates a desktop workspace without a solution file. The app discovers a runnable project configuration and renders the workspace UI. The companion [folder-discovery PR](https://github.com/javy-fernandez/dotcraft/pull/169) tightens `rc-core` classification while preserving the folder's session key.

Validation on Linux: `CARGO_TARGET_DIR=/tmp/dotcraft-zen-target cargo test -p rc-app standalone_shop_project_opens_in_the_desktop_workspace`. The headless egui app harness uses the real Shop.Cli project folder and confirms its root and runnable configuration.

The SDK is unavailable in this workspace, so build/run execution and other platforms remain untested.
