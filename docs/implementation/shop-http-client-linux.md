# HTTP client against Shop.Api on Linux

The existing `screens_http_client_and_endpoints` acceptance test starts real Shop.Api with official .NET SDK 8.0.428 and opens its `Shop.Api.http` file in the native egui app. It sends the variable-based GET request from the editor gutter, waits for HTTP 200, and verifies a JSON body containing `temperatureC`. The HTTP Client window renders the method, URL, status, timing and response body; I reran the test and inspected the [screenshot](shop-http-client-linux.png).

Validation: `PATH=/path/to/dotnet:$PATH DOTNET_ROOT=/path/to/dotnet CARGO_TARGET_DIR=/path/to/dedicated-target cargo test -p rc-app screens_http_client_and_endpoints -- --ignored --nocapture` (1 passed on Linux with offscreen Vulkan). Prior local tests also cover cancellation and inline variables.

This verifies basic local .NET 8 HTTP execution and the native response view. Authentication, cookies, redirects, streaming bodies, file uploads, advanced environment flows, other runtimes and platforms remain untested. The implementation and screenshot use original DotCraft and its Shop fixture; no CrabCraft code or assets were copied.
