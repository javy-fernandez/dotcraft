# Thread list on Shop.Cli with .NET 8 Linux

The G-09 acceptance assertion extends the existing Shop.Cli real debugger regression. After a verified breakpoint in `PriceCalculator.Calculate`, it waits for the DAP thread response, checks that the stopped thread ID is in DotCraft's thread list, and checks every reported thread has a positive ID and nonempty name. The same regression then exercises stack, variables and stepping.

Validation: `PATH=/path/to/dotnet:$PATH DOTNET_ROOT=/path/to/dotnet DOTCRAFT_NETCOREDBG=/path/to/netcoredbg CARGO_TARGET_DIR=/path/to/dedicated-target cargo test -p rc-dap debugs_the_cli_breakpoints_stack_variables_watches_steps_and_output -- --nocapture` (1 passed with official .NET SDK 8.0.428 and netcoredbg 3.2.0-1092 on Linux).

This validates backend thread-list delivery for a local .NET 8 C# CLI breakpoint. Multiple threads, thread switching, UI rendering, other runtimes and platforms remain untested. No source code or assets were copied from CrabCraft.
