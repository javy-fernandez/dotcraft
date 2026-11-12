# CPU profiling Shop.Api on Linux

The existing `screens_profiler` acceptance test opens real Shop.sln, starts Shop.Api with official .NET SDK 8.0.428, and waits for runtime counters. It clicks Start CPU Profiling in the native Monitoring window, generates concurrent HTTP load for three seconds, stops the profile, and asserts the resulting profile has samples. The egui UI renders both call-tree and hot-spots views. I reran the test; it passed and produced the inspected [call-tree screenshot](shop-cpu-profile-linux.png) with 4,343 samples.

Validation: `PATH=/path/to/dotnet:$PATH DOTNET_ROOT=/path/to/dotnet CARGO_TARGET_DIR=/path/to/dedicated-target cargo test -p rc-app screens_profiler -- --ignored --nocapture` (1 passed on Linux with offscreen Vulkan).

This validates CPU profiling of a local ASP.NET Core .NET 8 process and the call-tree view. Other runtimes, hot-spots accuracy, short or idle profiles, large profiles, Windows/macOS and GPU backends remain untested. The implementation and screenshot use original DotCraft and its Shop fixture; no CrabCraft code or assets were copied.
