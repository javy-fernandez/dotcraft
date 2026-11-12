# J-09: endpoint scan project boundaries

## Behavior

The ASP.NET endpoint scan stops at nested C#, F#, and Visual Basic project directories. Previously it stopped only at a nested `.csproj`, so C# files physically present under a `.fsproj` or `.vbproj` directory could appear as endpoints of the parent web project. The scanner reuses the shared DotCraft project-extension check and only treats regular project files as boundaries.

## Validation

- `CARGO_TARGET_DIR=/tmp/dotcraft-endpoint-boundaries-target cargo test -p rc-dotnet --test endpoint_project_boundaries` — 2 passed on Linux.
- `CARGO_TARGET_DIR=/tmp/dotcraft-endpoint-boundaries-target cargo test -p rc-dotnet endpoints::tests` — 10 existing endpoint tests passed, including cancellation, bounded source reads, route parsing, and Shop fixture coverage.
- Real Shop.Api fixture still reports exactly `GET /Orders/sample`, `POST /Orders/quote`, `GET /WeatherForecast`, and `GET /health`.
- A web project fixture retains endpoints from ordinary nested source folders while excluding generated C# endpoints under nested `.csproj`, `.fsproj`, and uppercase `.VBPROJ` folders. On Unix, a symbolic link named `Linked.csproj` does not incorrectly turn an ordinary source folder into a nested project. Pre-cancelled scans return no results.
- `git diff --check` passed.

## Provenance and limits

This extends DotCraft's existing endpoint scanner from upstream commit `becfb0fe316203df2d8d7c1e99c6a54bd37cd13b`; no external source or assets were copied. Scanner isolation and source extraction are tested, but the Endpoints tool window, live server routing, and cross-method group propagation were not exercised. This is a tested J-09 backend scope, not full Rider parity.
