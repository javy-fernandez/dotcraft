# Shop test gutter outcomes on Linux

The H-06 acceptance test opens the real `PriceCalculatorTests.cs` inside Shop.sln. Before a test run, the `SampleOrderGetsFreeShipping` method has a Run Test gutter icon. After DotCraft runs all Shop tests with official .NET SDK 8.0.428, the runner records its passed outcome and the native egui editor changes that same line to the Test Passed gutter icon.

Validation: `PATH=/var/tmp/dotcraft-dotnet:$PATH DOTNET_ROOT=/var/tmp/dotcraft-dotnet DOTNET_CLI_HOME=/var/tmp/dotcraft-cli-home CARGO_TARGET_DIR=/var/tmp/dotcraft-zen-target CARGO_BUILD_JOBS=2 cargo test -p rc-app shop_test_gutter_reflects_live_test_outcomes -- --ignored --nocapture` (1 passed on Linux). The test is ignored by default because it needs the SDK and restored Shop packages.

This verifies a passed xUnit method in a local .NET 8 workspace. Failed/skipped icons, parameterized cases, click-to-run, other test frameworks' gutters and other platforms remain untested. The implementation is existing original DotCraft code and the repository Shop fixture; no CrabCraft code or assets were copied.
