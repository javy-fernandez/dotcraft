# B-13 NuGet installation and removal: live .NET 8 acceptance

## Behavior and provenance

DotCraft uses `dotnet add package` and `dotnet remove package` for projects without Central Package Management. This change adds live acceptance for those commands and the native NuGet tool window. No code or assets from another repository are copied.

The tests use Microsoft's official .NET SDK 8.0.428 on Linux x86_64 and a temporary copy of `tests/fixtures/Shop`. `Newtonsoft.Json` 13.0.1 and 13.0.3 archives come from the pre-existing local NuGet cache and are copied into a temporary file feed. A temporary `NuGet.Config` clears other sources, so the tests do not depend on a live package service. None of these archives or generated build outputs enter the repository.

## Validation

With `DOTCRAFT_TEST_DOTNET=/var/tmp/dotcraft-dotnet/dotnet`, `DOTNET_ROOT=/var/tmp/dotcraft-dotnet`, `NUGET_PACKAGES=/var/tmp/dotcraft-cli-home/.nuget/packages`, an isolated `DOTNET_CLI_HOME`, and `CARGO_TARGET_DIR=/var/tmp/dotcraft-nuget-install-live-target`:

```text
cargo test -p rc-dotnet --test nuget_install_live -- --ignored --nocapture
cargo test -p rc-app nuget_window_installs_and_removes_package_in_shop_core -- --ignored --nocapture
cargo test -p rc-dotnet --lib
```

The backend test adds `Newtonsoft.Json` 13.0.1 to Shop.Core, updates it to 13.0.3, compiles source that calls `JsonConvert`, then removes the package reference. The app test clicks **Install** and **Remove** in the NuGet window, checking the package state and project file after each operation.

## Limits

Acceptance covers a non-CPM project and a local file feed on .NET 8/Linux. Other package sources, authentication, offline cache misses, CPM edits, and other SDK versions or platforms need their own validation. Invalid package input preflight is covered separately by PR #219; package discovery is tracked under B-12.
