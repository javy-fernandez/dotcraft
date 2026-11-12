# B-17: Project reference text edits

## Behavior

`rc-dotnet::project_references` provides backend text edits to add and remove `ProjectReference` items in SDK-style projects. It resolves the source, target and workspace root to canonical paths, requires both projects to be existing `.csproj`, `.fsproj` or `.vbproj` files inside the root, and rejects self references and duplicate target references. Additions use a relative MSBuild path and preserve the surrounding XML and line ending style. Removal finds matching references by resolved path, including alternate relative spellings, and preserves other items. Neither operation writes a file.

The implementation uses the repository's existing XML parser and project metadata model. No third-party code or assets were copied.

## Validation

- `CARGO_TARGET_DIR=/tmp/dotcraft-project-refs-target cargo test -p rc-dotnet --test project_reference_edit`: two tests passed. They use real Shop.Api, Shop.Cli and Shop.Core project fixtures, check round-trip add/remove and existing-reference rejection, and reject self, outside-root, non-project and Unix symlink-escape targets.
- `CARGO_TARGET_DIR=/tmp/dotcraft-project-refs-target cargo test -p rc-dotnet --lib`: 132 tests passed.

## Limits

This backend API does not expose an app editor, persist changes, evaluate MSBuild conditions, or detect dependency cycles between projects. Removal requires the target file to exist. B-17 remains partial until UI and SDK acceptance are exercised.
