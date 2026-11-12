# H-22: Memory snapshots of a running .NET process on Linux

## Behavior

DotCraft's existing snapshot backend connects to a running .NET process through EventPipe, triggers a full GC heap walk, and groups live objects by type. The result includes object counts and shallow byte sizes, sorted by size. The Monitoring window's Memory tab shows these types; it captures the running application process rather than the build host.

The new live test copies the real `Shop.Api` and `Shop.Core` fixtures into a temporary directory, builds Shop.Api with .NET SDK 8.0.428, starts it on an ephemeral localhost port, and snapshots that process ID. It checks a nonempty heap, `System.String` objects, sorted type sizes, internally consistent totals, and that Shop.Api remains alive through capture. A child-process guard stops it after the test.

The existing offscreen app test clicks **Get Memory Snapshot** while Shop.Api is running, confirms more than 1,000 objects including `System.String` and `Microsoft.AspNetCore` types, excludes `Microsoft.Build` types, and renders the [Memory tab screenshot](memory-snapshot-linux.png). The screenshot was generated in this PR's isolated checkout and visually inspected.

## Validation and provenance

- Fixtures: `tests/fixtures/Shop/src/Shop.Api` and `Shop.Core` from this repository. The backend test builds a temporary copy; the UI test runs the repository fixture and creates only ignored build outputs.
- SDK: Microsoft's official [dotnet-install script](https://learn.microsoft.com/en-us/dotnet/core/tools/dotnet-install-script), invoked with `--channel 8.0 --install-dir /var/tmp/dotcraft-dotnet --no-path`; installed version **8.0.428** on Linux. Shop.Api's NuGet packages were resolved from `/var/tmp/dotcraft-cli-home/.nuget/packages`.
- Runtime semantics: Microsoft's [GC dump documentation](https://learn.microsoft.com/en-us/dotnet/core/diagnostics/dotnet-gcdump) describes EventPipe heap collection and its full-GC pause. DotCraft implements the EventPipe client directly in Rust; this test did not install or call the external `dotnet-gcdump` tool.
- `DOTCRAFT_TEST_DOTNET=/var/tmp/dotcraft-dotnet/dotnet DOTNET_ROOT=/var/tmp/dotcraft-dotnet DOTNET_CLI_HOME=/var/tmp/dotcraft-memory-cli-home NUGET_PACKAGES=/var/tmp/dotcraft-cli-home/.nuget/packages DOTNET_CLI_TELEMETRY_OPTOUT=1 CARGO_TARGET_DIR=/var/tmp/dotcraft-memory-snapshot-live-target cargo test -p rc-dotnet --test memory_snapshot_live -- --ignored --nocapture`: **1 passed**.
- `DOTNET_ROOT=/var/tmp/dotcraft-dotnet DOTNET_CLI_HOME=/var/tmp/dotcraft-memory-cli-home NUGET_PACKAGES=/var/tmp/dotcraft-cli-home/.nuget/packages DOTNET_CLI_TELEMETRY_OPTOUT=1 CARGO_TARGET_DIR=/var/tmp/dotcraft-memory-snapshot-live-target cargo test -p rc-app --bin dotcraft screens_profiler -- --ignored --nocapture`: **1 passed** with offscreen Vulkan, including the memory snapshot assertions and screenshot.
- `CARGO_TARGET_DIR=/var/tmp/dotcraft-memory-snapshot-live-target cargo test -p rc-dotnet --lib --quiet`: **132 passed**.

## Scope and limits

This verifies H-22's live heap capture and UI presentation for Shop.Api on Linux. It reports shallow sizes and counts, without object reference paths or retained sizes. Capturing the heap triggers a full GC that can pause the target process. The live tests are ignored by default because they need a local SDK, NuGet packages, and an offscreen-capable GPU for the UI test. Other runtimes, large heaps, Windows/macOS, and memory diffing were not exercised.
