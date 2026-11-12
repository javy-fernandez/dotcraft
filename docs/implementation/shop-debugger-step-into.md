# Step Into on Shop.Cli with .NET 8 Linux

The existing real debugger regression `debugs_the_cli_breakpoints_stack_variables_watches_steps_and_output` already exercises G-05 explicitly. It stops at Shop.Core `PriceCalculator.Calculate`, advances to the `DiscountFor(order, subtotal)` call, sends DAP Step In, and asserts that the next stop is inside `DiscountFor` at its opening brace rather than in the caller.

I repeated that regression using official .NET SDK 8.0.428 and netcoredbg 3.2.0-1092 on Linux:

`PATH=/path/to/dotnet:$PATH DOTNET_ROOT=/path/to/dotnet DOTCRAFT_NETCOREDBG=/path/to/netcoredbg CARGO_TARGET_DIR=/path/to/dedicated-target cargo test -p rc-dap debugs_the_cli_breakpoints_stack_variables_watches_steps_and_output -- --nocapture`

Result: 1 live integration test passed. This validates backend Step Into for a local .NET 8 C# CLI call on Linux. Toolbar/keyboard dispatch, async methods, framework code, other runtimes and platforms remain untested. No source code or assets were copied from CrabCraft.
