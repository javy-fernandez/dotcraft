# B-15: Solution filter creation and editing backend

## Behavior

`rc-dotnet::solution_filter_edit` creates `.slnf` JSON and updates the selected projects of an existing filter. It validates the source `.sln` or `.slnx`, confirms every selected project belongs to that solution and is inside the workspace, rejects duplicate selections and paths outside the workspace, and writes solution-relative project paths. Updates preserve unknown top-level and `solution` JSON fields and a UTF-8 BOM. The `.slnf` loader now accepts that BOM so edited filters can be loaded again. These APIs return text and do not write files.

The implementation uses the repository's existing solution parser. No third-party code or assets were copied.

## Validation

- `CARGO_TARGET_DIR=/tmp/dotcraft-filter-edit-target cargo test -p rc-dotnet --test solution_filter_edit`: two tests passed. They copy the real TenProjects `.slnx` and its ten project files into a temporary workspace, create and reload a two-project filter, edit it to one project while retaining custom JSON fields and BOM, and reject invalid membership and workspace escapes.
- `CARGO_TARGET_DIR=/tmp/dotcraft-filter-edit-target cargo test -p rc-dotnet --lib`: 132 tests passed, including existing `.slnf` parser tests.

## Limits

JSON formatting is normalized by serialization when a filter is edited; unknown field values are preserved. The backend does not expose a desktop interaction, persist the returned text, run a filtered SDK build, or update language-service state. B-15 remains partial until those workflows are exercised.
