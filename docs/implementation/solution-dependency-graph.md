# B-28: Solution dependency graph backend

## Behavior

`rc-dotnet::solution_graph` builds a directed graph from the projects in a loaded `.sln`, `.slnx` or `.slnf` and their `ProjectReference` items. Edges point from a project to the project it references. References outside the loaded solution, including projects excluded by a filter, are recorded separately and are not followed. The graph can report a cycle and export escaped Graphviz DOT text. Loading is limited to 512 projects, 10,000 references and 1 MiB per project file.

The implementation uses the repository's existing solution and project parsers. No third-party code or assets were copied.

## Validation

- `CARGO_TARGET_DIR=/tmp/dotcraft-solution-graph-target cargo test -p rc-dotnet --test solution_graph`: three tests passed. The real Shop.sln yields six projects and five project-reference edges. A real Shop `.slnf` subset records its excluded Shop.Core dependency without loading it. A small synthetic solution tests cycle detection, unknown references and DOT label escaping.
- `CARGO_TARGET_DIR=/tmp/dotcraft-solution-graph-target cargo test -p rc-dotnet --lib`: 132 tests passed.

## Limits

This is a source-level project-reference graph. It does not evaluate conditional MSBuild references, render a diagram in the desktop UI, infer package dependencies, or run a build. DOT export requires a separate renderer to produce an image. B-28 remains partial until visual and SDK acceptance are exercised.
