# B-06: project template target validation

## Behavior

Adding a project to an existing solution now checks that the target is an existing `.sln` or `.slnx` file before `dotnet new` can create a project directory. Missing paths, directories with a solution extension, unrelated files, and `.slnf` filters are rejected with a message passed through the existing translated dialog path. Valid classic and XML solution targets keep their command plans.

## Validation

- `CARGO_TARGET_DIR=/tmp/dotcraft-template-preflight-target cargo test -p rc-dotnet --test template_solution_preflight` — 2 passed on Linux.
- `CARGO_TARGET_DIR=/tmp/dotcraft-template-preflight-target cargo test -p rc-dotnet new_project::tests` — 1 existing command-planning test passed.
- Real Shop `.sln` and TenProjects `.slnx` fixtures passed preflight and produced `dotnet new` followed by `dotnet sln ... add` plans without modifying the fixtures.
- Missing `.sln`, existing `.slnf`, directory named `.slnx`, and unrelated extension were rejected before a project directory appeared. An uppercase `.SLNX` file was accepted; an existing destination folder remained rejected.
- `git diff --check` passed.

## Provenance and limits

This extends DotCraft's existing project-template planner at upstream commit `becfb0fe316203df2d8d7c1e99c6a54bd37cd13b`; no external source or assets were copied. The .NET SDK was unavailable in this workspace, so actual template creation and UI acceptance were not exercised. Preflight checks the target extension and existence, not whether its contents form a valid solution. Adding to a `.slnf` filter requires selecting its source solution. This is a tested B-06 planning scope, not full Rider parity.
