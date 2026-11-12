# B-18: Target framework text editing backend

## Behavior

`rc-dotnet::project::set_target_frameworks` now edits the single explicit, unconditional `TargetFramework` or `TargetFrameworks` property of an SDK-style project. It supports one framework or a semicolon-separated list of up to 16 distinct frameworks. It preserves the project's other XML, line endings, surrounding whitespace, and UTF-8 BOM. It rejects missing, duplicate, conditional, or ambiguous properties, as well as invalid framework text.

The implementation uses the repository's existing XML parser and project metadata model. No third-party code or assets were copied.

## Validation

- `CARGO_TARGET_DIR=/tmp/dotcraft-tfm-edit-target cargo test -p rc-dotnet --test target_framework_edit`: three tests passed. They edit the real `Shop.Core.csproj` and `Shop.Api.csproj` fixtures, check exact preservation of all other content, and verify rejection cases.
- `CARGO_TARGET_DIR=/tmp/dotcraft-tfm-edit-target cargo test -p rc-dotnet --lib`: 132 tests passed.

## Limits

This is a backend text-edit API. It does not modify files on disk, expose a desktop editor, or run an SDK build. Conditional properties, inherited properties, and projects with multiple framework declarations require MSBuild-aware handling and are rejected. B-18 remains partial until UI and SDK acceptance are exercised.
