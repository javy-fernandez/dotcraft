# Shop test result tree on Linux

The H-05 acceptance test opens the real Shop.sln, runs all tests with official .NET SDK 8.0.428, and paints the native egui Tests window. Thirteen xUnit, NUnit and MSTest cases pass. The accessibility tree contains the three test classes, and clicking the visible `SampleKeepsTheCustomer` row selects its fully qualified result in app state.

Validation: `PATH=/var/tmp/dotcraft-dotnet:$PATH DOTNET_ROOT=/var/tmp/dotcraft-dotnet DOTNET_CLI_HOME=/var/tmp/dotcraft-cli-home CARGO_TARGET_DIR=/var/tmp/dotcraft-zen-target CARGO_BUILD_JOBS=2 cargo test -p rc-app shop_test_results_render_and_select_in_the_tests_window -- --ignored --nocapture` (1 passed on Linux). The test is ignored by default because it requires the SDK and restored Shop packages.

This verifies successful local .NET 8 result grouping and click selection. Failed/skipped cases, rerun, keyboard navigation, large result sets, and other platforms remain untested. The code uses the original DotCraft implementation and repository Shop fixture; no CrabCraft code or assets were copied.
