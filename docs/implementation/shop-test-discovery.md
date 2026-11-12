# Shop test discovery on Linux

The H-04 acceptance test invokes the existing `discover_tests` backend against the real Shop.sln with official .NET SDK 8.0.428. Discovery builds the solution, evaluates each test project's target assembly, and asks VSTest for fully qualified test names. The result contains at least ten tests across Shop.Core.XunitTests, Shop.Core.NunitTests and Shop.Core.MSTests, each with a class, method, project and existing assembly path.

Validation: `PATH=/var/tmp/dotcraft-dotnet:$PATH DOTNET_ROOT=/var/tmp/dotcraft-dotnet DOTNET_CLI_HOME=/var/tmp/dotcraft-cli-home CARGO_TARGET_DIR=/var/tmp/dotcraft-zen-target CARGO_BUILD_JOBS=2 cargo test -p rc-dotnet discovers_shop_xunit_nunit_and_mstest_with_real_sdk -- --ignored --nocapture` (1 passed on Linux). The test is ignored by default because it needs a local SDK and restored test packages. The normal `rc-dotnet` library suite has 132 tests; this command runs the live acceptance separately.

The evidence covers backend discovery on .NET 8 Linux. App test-tree rendering, discovery cancellation, changed projects, other frameworks and other platforms remain untested. This slice uses existing DotCraft code and the repository Shop fixture; it copies no CrabCraft code or assets.
