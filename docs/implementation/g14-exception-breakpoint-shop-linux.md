# G-14 exception breakpoints in Shop.Cli on Linux

## Behavior

The debugger sends an explicit `all` exception filter for a Shop.Cli run that throws `ArgumentException` from `Order.cs`. It stops at the throw as a first-chance exception, supplies `exceptionInfo` with the exception type and message, and later stops again when the exception is unhandled. A second run with default filters stops on the unhandled exception.

## Validation and provenance

The existing live regression in `crates/rc-dap/tests/netcoredbg.rs` exercises `tests/fixtures/Shop` with the .NET SDK and netcoredbg. This slice adds assertions that the explicit filter is retained and offered by the adapter and that the default run has no explicit filter. The configured `DOTCRAFT_NETCOREDBG` binary is selected as `Origin::Settings`. No CrabCraft code or assets were copied.

Validation passed on Linux with official .NET SDK 8.0.428 and the configured netcoredbg binary at `target/test-home/tools/netcoredbg/3.2.0-1092/netcoredbg`:

- `DOTNET_ROOT=/var/tmp/dotcraft-dotnet DOTCRAFT_NETCOREDBG=<netcoredbg-binary> CARGO_TARGET_DIR=<dedicated-target> cargo test -p rc-dap --test netcoredbg stops_on_exceptions_with_exception_info -- --exact --nocapture` — 1 live test passed.
- `cargo test -p rc-dap --lib` — 19 tests passed.

## Scope and limitations

This verifies the real Shop.Cli launch and exception information flow on Linux with a configured SDK and adapter. It does not cover the desktop exception-breakpoint settings UI, other exception filters, attach sessions, or other operating systems.
