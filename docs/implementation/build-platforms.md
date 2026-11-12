# B-19: solution build configurations and platforms (backend slice)

This slice reads the solution-wide configuration/platform pairs from a classic
`.sln` file. It also accepts a `.slnf` filter and reads the pairs from its source
`.sln`. `BuildRequest` can pass an explicitly selected platform to MSBuild while
preserving the existing `Debug` configuration default. Platform names retain the
solution spelling, including `Any CPU`, so the solution's project mappings can
resolve each project's platform.

## Validation

- `CARGO_TARGET_DIR=/tmp/dotcraft-dotnet-slice-target cargo test -p rc-dotnet configuration --lib`
- `CARGO_TARGET_DIR=/tmp/dotcraft-dotnet-slice-target cargo test -p rc-dotnet build::tests --lib`
- `CARGO_TARGET_DIR=/tmp/dotcraft-dotnet-slice-target cargo test -p rc-dotnet --lib` (134 passed)

The tests cover the six choices in the Shop `.sln` fixture, filter resolution,
exclusion of per-project mappings, duplicate choices, malformed entries, and the
build command argument containing a platform name with a space.

## Remaining work

The desktop app does not yet offer a configuration/platform selector or persist a
choice. XML `.slnx` configuration discovery, project-level mapping inspection, and
a live SDK build with a non-default platform remain to be validated. B-19 remains
partial.
