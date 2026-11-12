# F-05: Streamed .NET build output on Linux

## Behavior

The existing build backend sends a `Started` event, then line-oriented `Output` events from the SDK while the process runs, followed by `Finished` with its result. Output lines retain their stdout/stderr stream and newline status.

The live acceptance test copies the real `Shop.Core` fixture to a temporary directory and inserts a probe MSBuild target into the copy. The target prints a begin marker, sleeps for two seconds, and prints an end marker before compilation. The test receives the begin marker while the build handle is still active, then verifies marker order, stdout/newline metadata, a successful SDK result, and `Finished` as the final event. This distinguishes live streaming from collecting all output after process exit. The source fixture is not modified.

## Validation and provenance

- Fixture: `tests/fixtures/Shop/src/Shop.Core` from this repository. The probe target is generated only in a temporary copy; no external source code is copied into DotCraft.
- SDK: Microsoft's official [dotnet-install script](https://learn.microsoft.com/en-us/dotnet/core/tools/dotnet-install-script), invoked with `--channel 8.0 --install-dir /var/tmp/dotcraft-dotnet --no-path`; installed version **8.0.428** on Linux.
- CLI: Microsoft's [`dotnet build` documentation](https://learn.microsoft.com/en-us/dotnet/core/tools/dotnet-build) documents the SDK build command and its output.
- `DOTCRAFT_TEST_DOTNET=/var/tmp/dotcraft-dotnet/dotnet DOTNET_CLI_HOME=/var/tmp/dotcraft-stream-cli-home DOTNET_CLI_TELEMETRY_OPTOUT=1 CARGO_TARGET_DIR=/var/tmp/dotcraft-build-stream-live-target cargo test -p rc-dotnet --test build_stream_live -- --ignored`: **1 passed**.
- `CARGO_TARGET_DIR=/var/tmp/dotcraft-build-stream-live-target cargo test -p rc-dotnet --lib --quiet`: **132 passed**.

## Scope and limits

This verifies F-05's backend streaming from a real SDK build on Linux. The live test is ignored by default because it requires a local .NET 8 SDK and the `sleep` command. It does not test UI presentation, arbitrary terminal control sequences, very large output, Windows/macOS, or concurrent builds. Existing unit tests cover output splitting and ANSI handling separately.
