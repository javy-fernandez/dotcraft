# Application monitoring for Shop.Api on Linux

The H-24 acceptance extension starts real Shop.Api from Shop.sln with official .NET SDK 8.0.428 and waits for runtime counters. It asserts that the latest process CPU sample is finite and nonnegative, resident memory is positive, and the .NET GC heap counter is finite and positive. The native Monitoring Charts view renders CPU, memory and runtime metrics; the inspected [screenshot](shop-monitoring-linux.png) shows a 78 MB working set, 2.9 MB GC heap and thread-pool/GC metrics.

Validation: `PATH=/path/to/dotnet:$PATH DOTNET_ROOT=/path/to/dotnet CARGO_TARGET_DIR=/path/to/dedicated-target cargo test -p rc-app screens_profiler -- --ignored --nocapture` (1 passed on Linux with offscreen Vulkan). The test also completes its existing CPU-profile and memory-snapshot checks.

This validates process/runtime monitoring for a local ASP.NET Core .NET 8 application. History retention, heavy load, process restart, chart accessibility, other runtimes, Windows/macOS and other GPU backends remain untested. The implementation and screenshot use original DotCraft and its Shop fixture; no CrabCraft code or assets were copied.
