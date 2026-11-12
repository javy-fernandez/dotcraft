# Shop coverage report in the native app

The H-18 acceptance test runs real Shop.sln coverage with official .NET SDK 8.0.428 and Coverlet 6.0.0 on Linux. It verifies that the report contains a partially covered Shop.Core assembly and a covered PriceCalculator class. In the native egui Coverage window, it collapses and reopens Shop.Core through the accessibility tree and verifies the PriceCalculator class row is visible again.

Validation: `PATH=/path/to/dotnet:$PATH DOTNET_ROOT=/path/to/dotnet CARGO_TARGET_DIR=/path/to/dedicated-target cargo test -p rc-app shop_coverage_report_groups_assembly_and_classes -- --ignored --nocapture` (1 passed on Linux). The test is ignored by default because it requires an installed SDK and restored Coverlet packages.

This verifies a local .NET 8 assembly/class report and its expand/collapse UI. Branch metrics, exports, multiple assemblies, filtering, MTP v2 and other platforms remain untested. The implementation uses original DotCraft code and the repository Shop fixture; no CrabCraft code or assets were copied.
