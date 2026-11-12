# F-03: Clean .NET project outputs on Linux

## Behavior

The existing `BuildAction::Clean` backend invokes `dotnet clean <target> -c <configuration>`. It streams a `Started` event, processes SDK output and diagnostics, and returns a successful `BuildResult` after the SDK exits successfully.

The live acceptance test copies the real `Shop.Core` project into a temporary directory and builds both Debug and Release. Cleaning Debug removes its final `bin` DLL and intermediate `obj` DLL while preserving Release. Cleaning Release then removes its outputs. Finally, a build with `--no-restore` regenerates Debug outputs. The source fixture is not modified.

## Validation and provenance

- Fixture: `tests/fixtures/Shop/src/Shop.Core` from this repository.
- SDK: Microsoft's official [dotnet-install script](https://learn.microsoft.com/en-us/dotnet/core/tools/dotnet-install-script), invoked with `--channel 8.0 --install-dir /var/tmp/dotcraft-dotnet --no-path`; installed version **8.0.428** on Linux.
- CLI behavior: Microsoft's [`dotnet clean` documentation](https://learn.microsoft.com/en-us/dotnet/core/tools/dotnet-clean) describes cleaning both final and intermediate outputs and passing the build configuration when needed.
- `DOTCRAFT_TEST_DOTNET=/var/tmp/dotcraft-dotnet/dotnet DOTNET_CLI_HOME=/var/tmp/dotcraft-clean-cli-home DOTNET_CLI_TELEMETRY_OPTOUT=1 CARGO_TARGET_DIR=/var/tmp/dotcraft-clean-live-target cargo test -p rc-dotnet --test clean_live -- --ignored`: **1 passed**.
- `CARGO_TARGET_DIR=/var/tmp/dotcraft-clean-live-target cargo test -p rc-dotnet --lib --quiet`: **132 passed**.

## Scope and limits

This verifies F-03's project Clean backend on Linux with SDK 8.0.428 for Debug and Release configurations. The live test is ignored by default because it requires an installed SDK. It does not test a full solution, multi-target projects, Windows/macOS, or UI progress presentation. Clean removes SDK build outputs; it does not necessarily remove every file under `bin` and `obj`.
