# Step Out on Shop.Cli with .NET 8 Linux

The existing real debugger regression `debugs_the_cli_breakpoints_stack_variables_watches_steps_and_output` already exercises G-06 explicitly. After Step In stops inside Shop.Core `DiscountFor`, it sends DAP Step Out and asserts that execution returns to `PriceCalculator.Calculate` at the original call line.

I repeated that regression using official .NET SDK 8.0.428 and netcoredbg 3.2.0-1092 on Linux:

`PATH=/path/to/dotnet:$PATH DOTNET_ROOT=/path/to/dotnet DOTCRAFT_NETCOREDBG=/path/to/netcoredbg CARGO_TARGET_DIR=/path/to/dedicated-target cargo test -p rc-dap debugs_the_cli_breakpoints_stack_variables_watches_steps_and_output -- --nocapture`

Result: 1 live integration test passed. This validates backend Step Out for a local .NET 8 C# CLI call on Linux. Toolbar/keyboard dispatch, async methods, exception unwinding, other runtimes and platforms remain untested. No source code or assets were copied from CrabCraft.
