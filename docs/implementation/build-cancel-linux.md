# F-04: Cancel a running .NET build on Linux

## Behavior

The existing build backend returns a cancellable `OpHandle`. Cancelling it while `dotnet build` is running ends the operation with `BuildResult.cancelled = true` and `success = false`. The final event is `Finished`; a later build of the same project can succeed.

The live acceptance test copies the real `Shop.Core` fixture into a temporary directory and inserts a probe MSBuild target into the copy. The target starts a 30-second child sleep, prints a marker, and waits. The test cancels upon receiving that marker, checks completion within ten seconds, confirms the target's end marker and compiled DLL are absent, and then builds the same copied project successfully after restoring its original project file. The source fixture is not modified.

## Validation and provenance

- Fixture: `tests/fixtures/Shop/src/Shop.Core` from this repository. The probe target exists only in the temporary copy; no external source code is copied into DotCraft.
- SDK: Microsoft's official [dotnet-install script](https://learn.microsoft.com/en-us/dotnet/core/tools/dotnet-install-script), invoked with `--channel 8.0 --install-dir /var/tmp/dotcraft-dotnet --no-path`; installed version **8.0.428** on Linux.
- CLI: Microsoft's [`dotnet build` documentation](https://learn.microsoft.com/en-us/dotnet/core/tools/dotnet-build) describes the command exercised by the test.
- `DOTCRAFT_TEST_DOTNET=/var/tmp/dotcraft-dotnet/dotnet DOTNET_CLI_HOME=/var/tmp/dotcraft-cancel-cli-home DOTNET_CLI_TELEMETRY_OPTOUT=1 CARGO_TARGET_DIR=/var/tmp/dotcraft-build-cancel-live-target cargo test -p rc-dotnet --test build_cancel_live -- --ignored`: **1 passed**.
- `CARGO_TARGET_DIR=/var/tmp/dotcraft-build-cancel-live-target cargo test -p rc-dotnet --lib --quiet`: **132 passed**.

## Scope and limits

This verifies F-04's backend cancellation and subsequent build recovery with a real SDK process on Linux. The live test is ignored by default because it requires a local .NET 8 SDK and the `sleep` command. It does not directly inspect every descendant process after cancellation, nor does it test Windows/macOS process termination or UI cancel controls. Existing process unit tests cover cancellation tokens and process behavior separately.
