# J-23: REST request templates from ASP.NET endpoints

## Behavior

`rc-dotnet::endpoint_http::requests_from_endpoints` converts discovered ASP.NET endpoints into an editable `.http` file. It uses an explicit HTTP(S) origin, accepts only known HTTP methods, and substitutes safe examples for route parameters, including numeric, GUID, Boolean, optional and unconstrained parameters. It rejects malformed or unsafe routes and bounds input and output sizes. Generated requests include an `Accept: application/json` header and a reminder to review authentication, route values and bodies before sending.

The generator composes the repository's existing endpoint scanner, launch profile parser and HTTP file parser. No third-party code or assets were copied.

## Validation

- `CARGO_TARGET_DIR=/tmp/dotcraft-endpoint-http-target cargo test -p rc-dotnet --test endpoint_http`: two tests passed. The real Shop.Api fixture yields four endpoints; its launch profile supplies the base URL. All four generated requests parse and resolve through the HTTP file backend. The second test covers route samples, invalid routes, method injection and unsafe origins.
- `CARGO_TARGET_DIR=/tmp/dotcraft-endpoint-http-target cargo test -p rc-dotnet --lib`: 132 tests passed, including existing endpoint scanner, launch profile and HTTP file tests.

## Limits

The generator returns text only. It does not expose a desktop action, write a `.http` file, infer request bodies or authentication, or send a request. Endpoints with an unspecified or unsupported method are skipped. Generated route values are examples, not values inferred from a live application. J-23 remains partial until UI and live request acceptance are exercised.
