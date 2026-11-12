# B-13: NuGet package command preflight

## Behavior

`rc-dotnet::nuget` now validates add, update and remove package commands before starting `dotnet`. It requires an existing `.csproj`, `.fsproj` or `.vbproj`, a package ID of at most 100 bytes with nonempty alphanumeric-led segments separated by dots or dashes, and a bounded version argument without leading/trailing whitespace, control characters or an option-like prefix. The CLI still validates full NuGet version semantics. Valid operations retain the existing argument order and asynchronous execution.

The ID shape and nuget.org length bound follow [Microsoft's NU1017 guidance](https://learn.microsoft.com/en-us/nuget/reference/errors-and-warnings/nu1017) and [NuGet manifest reference](https://learn.microsoft.com/en-us/nuget/reference/nuspec). No third-party code or assets were copied.

## Validation

- `CARGO_TARGET_DIR=/var/tmp/dotcraft-nuget-preflight-target cargo test -p rc-dotnet --test nuget_preflight`: one Unix integration test passed. It copies the real Shop.Api project fixture, uses a fake executable host, checks that invalid IDs, versions and project paths never start a process, and confirms valid add/remove argument vectors.
- `CARGO_TARGET_DIR=/var/tmp/dotcraft-nuget-preflight-target cargo test -p rc-dotnet --lib`: 132 tests passed.

## Limits

This scope validates command inputs and invocation, not actual package restore or project modification. The fake-host test is Unix-only. Private feeds may allow package IDs outside the nuget.org convention; those IDs are conservatively rejected here. SDK execution and desktop package-manager acceptance remain, so B-13 is partial.
