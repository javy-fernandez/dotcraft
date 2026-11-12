# B-09: Live .NET runtime discovery on Linux

## Behavior

The runtime-list parser now accepts whitespace between runtime name and version, including tabs and repeated spaces, and ignores malformed lines with extra tokens. `Dotnet::list_runtimes` is exercised against a real local .NET installation. The real Shop.Api project targets `net8.0` and its web SDK requires the ASP.NET Core runtime; the discovered installation contains both `Microsoft.NETCore.App` and `Microsoft.AspNetCore.App` 8.0.34 with existing version directories.

The SDK and runtimes were installed only in `/var/tmp/dotcraft-dotnet` using Microsoft's [non-admin `dotnet-install.sh` script](https://learn.microsoft.com/en-us/dotnet/core/tools/dotnet-install-script), with SDK version **8.0.428**. No third-party code or assets were copied.

## Validation

- `CARGO_TARGET_DIR=/var/tmp/dotcraft-runtime-discovery-target cargo test -p rc-dotnet --test runtime_discovery_live`: one parser test passed; the live test is ignored by default because it requires a local runtime.
- `DOTCRAFT_TEST_DOTNET=/var/tmp/dotcraft-dotnet/dotnet DOTCRAFT_TEST_RUNTIME_VERSION=8.0.34 CARGO_TARGET_DIR=/var/tmp/dotcraft-runtime-discovery-target cargo test -p rc-dotnet --test runtime_discovery_live -- --ignored`: one live test passed against Shop.Api and actual runtime directories.
- `CARGO_TARGET_DIR=/var/tmp/dotcraft-runtime-discovery-target cargo test -p rc-dotnet --lib`: 132 tests passed.

## Scope

B-09 can be marked complete for tested Linux backend runtime discovery and target matching. Windows/macOS installations, desktop runtime presentation and launching a web server were not exercised; these limits do not establish cross-platform or UI parity.
