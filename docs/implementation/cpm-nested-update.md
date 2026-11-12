# B-23: Nested central package version updates

## Behavior

The static Central Package Management preview now parses `PackageVersion Update` items and applies imported central files before the nearest file. An `Update` changes a previously included package version and records the overriding file as its source. An `Update` without a prior `Include` does not create a package version. An `Update` without version metadata retains the imported version. A child `Include` remains independent and can supply a new version. Central version text edits can also find an `Update` item.

This follows the nested `Directory.Packages.props` pattern in [Microsoft's Central Package Management documentation](https://learn.microsoft.com/en-us/nuget/consume-packages/Central-Package-Management). No third-party code or assets were copied.

## Validation

- `cargo test -p rc-dotnet --test cpm_updates`: three tests passed. They cover the real `tests/fixtures/CentralShop` project and central file, then copy its central settings into a temporary nested setup with imported `Include`, child `Update`, absent-item `Update`, metadata-only `Update`, and independent child `Include`.
- `cargo test -p rc-dotnet --lib`: 132 tests passed, including 19 existing CPM tests and nested central file behavior.

## Limits

The static preview handles unconditional items in the discovered import chain. It does not evaluate conditional MSBuild items, property values inherited from other files, or arbitrary import placement within one file. The SDK-backed path remains the authority for an evaluated project. `dotnet` SDK execution and the desktop UI were not exercised here, so B-23 remains partial beyond the tested static scope.
