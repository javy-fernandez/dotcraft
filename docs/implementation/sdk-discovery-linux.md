# B-08: Live .NET SDK discovery on Linux

## Behavior

`Dotnet::locate` now treats an existing extensionless explicit host file as a file even when its name is not exactly `dotnet`. The existing candidate order remains explicit path, `DOTNET_ROOT`, `PATH`, default locations, then the user installation. A live test exercises both explicit and `PATH` discovery, SDK listing and selected version reporting. The discovered host builds a temporary copy of the real Shop.Core project.

The live SDK was installed only in `/var/tmp/dotcraft-dotnet` with Microsoft's [non-admin `dotnet-install.sh` script](https://learn.microsoft.com/en-us/dotnet/core/tools/dotnet-install-script), using `--channel 8.0 --install-dir /var/tmp/dotcraft-dotnet --no-path`. The resolved version was **8.0.428**, with `Microsoft.NETCore.App` and `Microsoft.AspNetCore.App` **8.0.34**. No third-party code or assets were copied.

## Validation

- `CARGO_TARGET_DIR=/var/tmp/dotcraft-sdk-discovery-target cargo test -p rc-dotnet --test sdk_discovery_live`: one unit test passed; the live test is ignored by default because it requires a local SDK.
- `PATH=/var/tmp/dotcraft-dotnet:$PATH DOTCRAFT_TEST_DOTNET=/var/tmp/dotcraft-dotnet/dotnet DOTCRAFT_TEST_SDK_VERSION=8.0.428 CARGO_TARGET_DIR=/var/tmp/dotcraft-sdk-discovery-target cargo test -p rc-dotnet --test sdk_discovery_live -- --ignored`: one live test passed, including a successful Shop.Core build in a temporary directory.
- `CARGO_TARGET_DIR=/var/tmp/dotcraft-sdk-discovery-target cargo test -p rc-dotnet --lib`: 132 tests passed.

## Scope

The tested Linux backend scope covers explicit host files and directories, PATH discovery, SDK enumeration and selected version use with a real .NET 8 project build. Windows/macOS installation layouts, desktop SDK selection UI and managed SDK installation are outside this acceptance run. B-08 can be marked complete for this tested Linux SDK discovery scope; these limits do not establish cross-platform parity.
