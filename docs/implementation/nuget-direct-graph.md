# B-22: Static direct NuGet dependency graph

## Behavior

`rc-dotnet::nuget_graph` builds a bounded project-to-package graph from the existing static CPM preview. Package nodes are keyed by case-insensitive ID and selected version, so projects using different versions of one package remain distinct. Edges record whether a version came from a project declaration, a `VersionOverride`, a central file, or could not be resolved. Global CPM package references are included for enabled projects. The graph is limited to 512 projects, 10,000 package nodes and 10,000 edges.

The implementation composes the repository's existing CPM parser. No third-party code or assets were copied.

## Validation

- `CARGO_TARGET_DIR=/tmp/dotcraft-nuget-graph-target cargo test -p rc-dotnet --test nuget_graph`: two tests passed. The real CentralShop fixture distinguishes central Newtonsoft.Json 13.0.1 from the CLI's 13.0.3 override and includes Humanizer.Core. The real Shop.Api fixture retains its directly declared Swashbuckle version. A temporary CPM project checks an unresolved package and a global reference.
- `CARGO_TARGET_DIR=/tmp/dotcraft-nuget-graph-target cargo test -p rc-dotnet --lib`: 132 tests passed.

## Limits

This graph covers statically visible direct references. It does not read restore assets, compute transitive dependencies, evaluate conditional MSBuild items, display a desktop graph, or verify versions with the SDK. B-22 remains partial beyond this tested scope.
