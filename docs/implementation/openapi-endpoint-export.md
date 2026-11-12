# J-24: OpenAPI export from ASP.NET endpoints

## Behavior

`rc-dotnet::openapi_export::export` turns statically discovered ASP.NET endpoints into a bounded OpenAPI 3.0.3 JSON document. It records an explicit HTTP(S) server origin, paths, supported HTTP methods, required path parameters with simple type constraints, handler summaries, and a default response description. It rejects duplicate path/method pairs, malformed or unsupported route templates, unsafe server URLs, and oversized input or output. Optional, defaulted, catch-all and complex constrained parameters are rejected because the export cannot describe them accurately.

The exporter composes the repository's existing endpoint scanner, launch profile parser and OpenAPI importer. No third-party code or assets were copied.

## Validation

- `CARGO_TARGET_DIR=/tmp/dotcraft-openapi-export-target cargo test -p rc-dotnet --test openapi_export`: two tests passed. The real Shop.Api fixture yields four operations, with its launch profile providing the server URL. The document is parsed as JSON and imported back to four `.http` requests. A second test covers typed parameters, duplicate operations, invalid routes, method injection and unsafe origins.
- `CARGO_TARGET_DIR=/tmp/dotcraft-openapi-export-target cargo test -p rc-dotnet --lib`: 132 tests passed, including existing OpenAPI importer tests.

## Limits

The export is a route skeleton. It does not infer request or response schemas, authentication, status codes or API metadata beyond the handler summary. It does not write a file, expose a desktop action, or validate against a running server. J-24 remains partial until those workflows are exercised.
