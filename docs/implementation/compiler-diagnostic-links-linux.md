# F-06: Compiler diagnostic links on Linux

## Behavior

The build backend parses a compiler diagnostic into a source path, 1-based line and column, project path, severity, code, and message. It emits the diagnostic during the build and retains it in the final result. The Build tool window's clickable diagnostic row converts that source position to the editor's 0-based coordinates and opens the file there. A diagnostic without a source file has no editor target.

This change extracts the Build row's existing coordinate conversion into a small helper so it can be tested alongside editor navigation. It does not change the visual row or the test-results UI.

## Validation and provenance

- Fixture: `tests/fixtures/Shop/src/Shop.Core`. The live test copies its project and C# sources to a temporary directory, inserts an undefined name into `PriceCalculator.cs`, and compiles the copy. The repository fixture is not modified.
- SDK: Microsoft's official [dotnet-install script](https://learn.microsoft.com/en-us/dotnet/core/tools/dotnet-install-script), invoked with `--channel 8.0 --install-dir /var/tmp/dotcraft-dotnet --no-path`; installed version **8.0.428** on Linux.
- `DOTCRAFT_TEST_DOTNET=/var/tmp/dotcraft-dotnet/dotnet DOTNET_CLI_HOME=/var/tmp/dotcraft-diagnostics-cli-home DOTNET_CLI_TELEMETRY_OPTOUT=1 CARGO_TARGET_DIR=/var/tmp/dotcraft-diagnostics-links-target cargo test -p rc-dotnet --test compiler_diagnostic_links_live -- --ignored`: **1 passed**, including a real CS0103 diagnostic, exact file/project, line 1-based, column 1-based, and streamed event.
- `CARGO_TARGET_DIR=/var/tmp/dotcraft-diagnostics-links-target cargo test -p rc-app --bin dotcraft compiler_diagnostic_ --quiet`: **2 passed**, covering 1-based to 0-based conversion and opening a file with the caret at the expected line and column.
- `CARGO_TARGET_DIR=/var/tmp/dotcraft-diagnostics-links-target cargo test -p rc-dotnet --lib --quiet`: **132 passed**.

## Scope and limits

This verifies F-06's compiler diagnostic data and Build row navigation target on Linux with SDK 8.0.428. The live test is ignored by default because it requires a local SDK. The app test invokes the same navigation target used by the clicked row; it does not simulate a pointer click in a rendered Build window. It does not cover Windows/macOS path formats or every diagnostic type.
