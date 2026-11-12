# HTTP environment selection against Shop.Api

The J-06 acceptance test opens real Shop.sln and starts Shop.Api with official .NET SDK 8.0.428 on Linux. A temporary `.http` file uses `{{host}}`; its sibling `http-client.env.json` supplies the `dev` host. The native egui HTTP Client environment picker discovers `dev`, selection is retained, and sending the request resolves the selected URL to `http://localhost:5112/weatherforecast/`. The live API returns HTTP 200 with JSON containing `temperatureC`.

Validation: `PATH=/path/to/dotnet:$PATH DOTNET_ROOT=/path/to/dotnet CARGO_TARGET_DIR=/path/to/dedicated-target cargo test -p rc-app shop_http_environment_selection_sends_a_live_request -- --ignored --nocapture` (1 passed on Linux). The test is ignored by default because it requires an installed SDK and restored Shop packages. The temporary HTTP and environment files are removed after the test.

This validates public environment selection and variable resolution for a local .NET 8 request. Private/user precedence has separate backend evidence in PR #195; secret handling, dynamic values, environment switching while a request is active and other platforms remain untested. The implementation uses original DotCraft code and the repository Shop fixture; no CrabCraft code or assets were copied.
