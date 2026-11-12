# H-16: Code coverage collection on Linux

## Behavior

DotCraft's existing coverage backend builds a test target, runs projects that reference `coverlet.collector` with `dotnet test --no-build --collect "XPlat Code Coverage"`, reads their Cobertura reports, and merges line and branch hits. The Coverage window presents totals by assembly and class and marks covered, partly covered, and uncovered lines in the editor. Projects without the collector are skipped with a visible warning.

The new live acceptance test copies the real `Shop.Core` and `Shop.Core.XunitTests` fixture projects to a temporary directory. It runs the backend with .NET SDK 8.0.428 and verifies successful test execution, a real `coverage.cobertura.xml` file, covered `PriceCalculator.cs` lines, and progress events. The existing app acceptance test runs coverage on the full Shop solution and checks an editor mark and the rendered Coverage window. The generated screenshot is [coverage-linux.png](coverage-linux.png).

## Validation and provenance

- Fixture: `tests/fixtures/Shop` from this repository. The xUnit fixture references `coverlet.collector` **6.0.0** and `Microsoft.NET.Test.Sdk` **17.8.0**. The MSTest fixture lacks that collector; the app displays its skip warning.
- SDK: Microsoft's official [dotnet-install script](https://learn.microsoft.com/en-us/dotnet/core/tools/dotnet-install-script), invoked with `--channel 8.0 --install-dir /var/tmp/dotcraft-dotnet --no-path`; installed version **8.0.428** on Linux. Test packages were resolved from the local cache at `/var/tmp/dotcraft-cli-home/.nuget/packages`.
- Collector semantics: Microsoft's [code coverage guide](https://learn.microsoft.com/en-us/dotnet/core/testing/unit-testing-code-coverage) documents Coverlet and Cobertura reports.
- `DOTCRAFT_TEST_DOTNET=/var/tmp/dotcraft-dotnet/dotnet DOTNET_CLI_HOME=/var/tmp/dotcraft-coverage-cli-home NUGET_PACKAGES=/var/tmp/dotcraft-cli-home/.nuget/packages DOTNET_CLI_TELEMETRY_OPTOUT=1 CARGO_TARGET_DIR=/var/tmp/dotcraft-coverage-live-target cargo test -p rc-dotnet --test coverage_live -- --ignored`: **1 passed**.
- `DOTNET_ROOT=/var/tmp/dotcraft-dotnet DOTNET_CLI_HOME=/var/tmp/dotcraft-coverage-cli-home NUGET_PACKAGES=/var/tmp/dotcraft-cli-home/.nuget/packages DOTNET_CLI_TELEMETRY_OPTOUT=1 CARGO_TARGET_DIR=/var/tmp/dotcraft-coverage-live-target cargo test -p rc-app --bin dotcraft screens_coverage -- --ignored --nocapture`: **1 passed**. The screenshot shows **87% (52/60)** Shop.Core line coverage and line marks in `PriceCalculator.cs`.
- `CARGO_TARGET_DIR=/var/tmp/dotcraft-coverage-live-target cargo test -p rc-dotnet --lib --quiet`: **132 passed**.

## Scope and limits

This verifies H-16 coverage collection and UI presentation for Coverlet-enabled Shop tests on Linux. The live tests are ignored by default because they require an SDK, NuGet packages, and, for the screenshot, a GPU capable of offscreen rendering. The MSTest fixture is skipped because it has no `coverlet.collector` reference. Windows/macOS, other collector formats, and projects using the Microsoft Testing Platform v2 were not tested.
