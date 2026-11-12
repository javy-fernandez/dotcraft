# Shop.Api launch settings on Linux

The B-11 acceptance test loads the real Shop.Api `launchSettings.json` and evaluates its project with official .NET SDK 8.0.428. Selecting the `http` project profile yields a launch plan using the SDK, the Development environment, `http://localhost:5112` as ASP.NET URL, and `http://localhost:5112/swagger` as the browser target. This complements the native profile picker acceptance in [PR 236](https://github.com/javy-fernandez/dotcraft/pull/236).

Validation: `PATH=/path/to/dotnet:$PATH DOTNET_ROOT=/path/to/dotnet CARGO_TARGET_DIR=/path/to/dedicated-target cargo test -p rc-dotnet plans_the_real_shop_api_launch_profile_with_sdk -- --ignored --nocapture` (1 passed on Linux). The test is ignored by default because it requires a local SDK.

This verifies real .NET 8 profile parsing, MSBuild-backed plan resolution and UI profile selection across the two PRs. Automatic browser opening, profile changes during a running process, IIS Express, alternate platforms and runtimes remain untested. The implementation uses original DotCraft code and its Shop fixture; no CrabCraft code or assets were copied.
