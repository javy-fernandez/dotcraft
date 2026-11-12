# F-02: Rebuild a .NET project on Linux

## Behavior

The existing `BuildAction::Rebuild` backend runs `dotnet build <target> --no-incremental`. It supports `--no-restore` when the project has already been restored, streams the started command and SDK output, and returns a successful `BuildResult` only after the SDK exits successfully.

The live acceptance test copies the real `Shop.Core` fixture source into a temporary directory, builds it, corrupts both the generated DLL and its intermediate compiler output, and then invokes `BuildAction::Rebuild` with `no_restore`. It verifies the SDK command, streamed output, exit status, zero diagnostics, and the restored deterministic bytes of both outputs. The source fixture is not modified.

## Validation and provenance

- Fixture: `tests/fixtures/Shop/src/Shop.Core` from this repository.
- SDK: Microsoft's official [dotnet-install script](https://learn.microsoft.com/en-us/dotnet/core/tools/dotnet-install-script), invoked with `--channel 8.0 --install-dir /var/tmp/dotcraft-dotnet --no-path`; installed version **8.0.428** on Linux.
- CLI behavior: Microsoft's [`dotnet build` documentation](https://learn.microsoft.com/en-us/dotnet/core/tools/dotnet-build) documents `--no-incremental` as forcing a clean rebuild of the project and its dependencies.
- `DOTCRAFT_TEST_DOTNET=/var/tmp/dotcraft-dotnet/dotnet DOTNET_CLI_HOME=/var/tmp/dotcraft-rebuild-cli-home DOTNET_CLI_TELEMETRY_OPTOUT=1 CARGO_TARGET_DIR=/var/tmp/dotcraft-rebuild-live-target cargo test -p rc-dotnet --test rebuild_live -- --ignored`: **1 passed**.
- `CARGO_TARGET_DIR=/var/tmp/dotcraft-rebuild-live-target cargo test -p rc-dotnet --lib --quiet`: **132 passed**.

## Scope and limits

This verifies F-02's project rebuild backend on Linux with SDK 8.0.428. The live test is ignored by default because it requires an installed SDK. It does not test a full solution rebuild, projects with NuGet dependencies, Windows/macOS, UI progress presentation, or concurrent rebuilds. An ordinary incremental build can repair a corrupted final DLL by copying an intermediate output, which is why the test corrupts both outputs.
