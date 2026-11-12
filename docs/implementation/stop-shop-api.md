# Stop a running Shop.Api process

An ignored live acceptance test opens `Shop.sln`, launches Shop.Api without its launch profile on an ephemeral loopback port, waits for its listening output and process ID, then calls Stop. It verifies that the process exits and the run console reports the stop. A cleanup call kills any remaining process even if startup fails.

Validation on Linux with official local SDK 8.0.428: `PATH=/var/tmp/dotcraft-dotnet:$PATH DOTNET_ROOT=/var/tmp/dotcraft-dotnet DOTNET_CLI_HOME=/var/tmp/dotcraft-cli-home CARGO_TARGET_DIR=/var/tmp/dotcraft-zen-target CARGO_BUILD_JOBS=2 cargo test -p rc-app live_shop_api_run_can_be_stopped -- --ignored --nocapture`.

This covers local .NET 8 ASP.NET process stop. Debugger stop, Windows/macOS process trees, hot reload, and other runtimes remain untested. It uses DotCraft's existing runner and the Shop fixture; no CrabCraft source or assets were copied.
