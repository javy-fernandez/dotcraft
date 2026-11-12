# Shop test output on Linux

The H-08 acceptance test runs all tests from real Shop.sln with official .NET SDK 8.0.428 and verifies that the native Tests console contains streamed output from the xUnit, NUnit and MSTest projects, each successful VSTest run, and DotCraft's final `Tests passed: 13` message. It opens the Tests window in the egui harness and confirms that the console remains available beside the result tree.

Validation: `PATH=/var/tmp/dotcraft-dotnet:$PATH DOTNET_ROOT=/var/tmp/dotcraft-dotnet DOTNET_CLI_HOME=/var/tmp/dotcraft-cli-home CARGO_TARGET_DIR=/var/tmp/dotcraft-zen-target CARGO_BUILD_JOBS=2 cargo test -p rc-app shop_test_output_streams_into_the_tests_console -- --ignored --nocapture` (1 passed on Linux). The test is ignored by default because it requires the SDK and restored Shop packages.

This covers successful .NET 8 local test output. Failure details, per-test stdout, very large streams, cancellation, and other platforms remain untested. The implementation is existing original DotCraft code using the repository Shop fixture; no CrabCraft code or assets were copied.
