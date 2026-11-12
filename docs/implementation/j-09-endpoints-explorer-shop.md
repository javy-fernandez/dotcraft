# J-09 — ASP.NET endpoints explorer on Shop

## Behavior

The Endpoints tool window scans the active solution's projects in the background. For Shop.Api it presents four controller and minimal API routes, filters by HTTP method, opens a handler source location, and generates a request in the project's `.http` file.

## Validation

- `CARGO_TARGET_DIR=/var/tmp/dotcraft-endpoints-explorer-shop-target CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 cargo test -p rc-app shop_endpoints_scan_show_handlers_and_generate_request -- --nocapture`
- `CARGO_TARGET_DIR=/var/tmp/dotcraft-endpoints-explorer-shop-target CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 cargo test -p rc-dotnet --lib endpoints_shop_fixture -- --nocapture`
- Live `Shop.Api` server with .NET SDK 8.0.428 from `/var/tmp/dotcraft-dotnet/dotnet`, launched on an ephemeral localhost port. GET `/health`, GET `/Orders/sample`, POST `/Orders/quote` with a valid Shop order, and GET `/WeatherForecast` each returned HTTP 200. The response bodies were checked for the expected health text, `Ada` customer, or forecast array.
- `git diff --check`.

The egui test opens a temporary copy of the real `tests/fixtures/Shop/Shop.sln`, waits for the background scan, verifies all four routes in the tool window, filters to POST, uses the context menu to open the `OrdersController.Sample` handler, and generates a GET `/health` request through the tool window. It checks the active source location and the appended `.http` request. The original fixture remains untouched.

## Provenance and limits

This uses the repository-owned Shop fixture and existing DotCraft scanner and UI. No CrabCraft code or assets were copied. The four Shop routes are validated through source scanning, egui interaction, and the live ASP.NET server. This does not imply full ASP.NET endpoint parity: Razor Pages, dynamic route registration, and cross-method group propagation remain outside this tested scope. Project boundary hardening is in separate PR #184.
