# J-06: Parent HTTP environments and nested overrides

## Behavior

The `.http` resolver now discovers environment files in the request directory and its ancestors through the solution or repository root. Public files are applied from parent to child, followed by private files from parent to child, then `.user` files from parent to child. A nearer file of the same kind overrides parent values; private and `.user` values override public values. Environment names from the discovered files are available for selection. The search does not cross the solution or repository root.

The parent public file and child private override behavior follows [JetBrains HTTP Client variable documentation](https://www.jetbrains.com/help/webstorm/http-client-variables.html), which describes nested directories and private overrides. The `.user` override follows [Microsoft's `.http` file documentation](https://learn.microsoft.com/en-us/aspnet/core/test/http-files). No third-party code or assets were copied.

## Validation

- `CARGO_TARGET_DIR=/tmp/dotcraft-http-env-parents-target cargo test -p rc-dotnet --test http_env_parent`: one test passed. It uses the real `Shop.Api.http` fixture and `Shop.sln`, with public environments at the solution and intermediate directory, and private and `.user` values next to the request. It checks request URL resolution, variable precedence, environment names, and the solution boundary.
- `CARGO_TARGET_DIR=/tmp/dotcraft-http-env-parents-target cargo test -p rc-dotnet --lib`: 132 tests passed, including existing environment and request resolution tests.

## Limits

This scope covers static environment discovery and request resolution. It does not exercise the desktop environment selector or send a live HTTP request. The implementation does not search unrelated project directories for environment files or implement request scripts. J-06 remains partial beyond this tested scope.
