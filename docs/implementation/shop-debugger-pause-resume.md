# Pause and resume Shop.Api on .NET 8 Linux

The G-08 acceptance test launches real Shop.Api under netcoredbg on an ephemeral loopback port. Once the server is listening, DotCraft sends DAP Pause and verifies a stopped state with `StopReason::Pause`. It then sends Continue, verifies the server is running again, and receives HTTP 200 from `/orders/sample`. Finally it stops the session and waits for the port to close.

Validation: `PATH=/path/to/dotnet:$PATH DOTNET_ROOT=/path/to/dotnet DOTCRAFT_NETCOREDBG=/path/to/netcoredbg CARGO_TARGET_DIR=/path/to/dedicated-target cargo test -p rc-dap pauses_and_resumes_a_live_shop_api_process -- --nocapture` (1 passed with official .NET SDK 8.0.428 and netcoredbg 3.2.0-1092 on Linux).

This validates backend Pause/Continue on a local ASP.NET Core process. UI controls, multiple threads, pause during requests, other runtimes and platforms remain untested. No CrabCraft code or assets were copied.
