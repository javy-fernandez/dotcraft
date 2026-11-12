# L-04: background runtime for blocking .NET project I/O

## Behavior

`Runtime::spawn_blocking` routes synchronous file and process work to Tokio's blocking pool, separate from DotCraft's four asynchronous worker threads and the UI thread. Existing `Runtime::spawn` tasks retain a cancellable `JoinHandle`. A blocking closure already running cannot be stopped by aborting its handle, so callers must arrange their own cancellation if needed.

## Validation

- `CARGO_TARGET_DIR=/tmp/dotcraft-core-runtime-target cargo test -p rc-core` — 13 passed on Linux.
- Four blocking workers read a real `App.csproj` fixture and waited. A short asynchronous task finished while all four workers remained occupied. Each file read ran on a thread different from the test caller.
- A pending asynchronous task was aborted and its join error reported cancellation.
- `rustfmt --edition 2024 --check crates/rc-core/src/runtime.rs` and `git diff --check` passed.

## Provenance and limits

This extends DotCraft's existing Tokio runtime at upstream commit `becfb0fe316203df2d8d7c1e99c6a54bd37cd13b`; no external source or assets were copied. The .NET SDK was unavailable in this workspace, so a `dotnet` process was not run. Live UI scheduling, process cancellation, and use of the new API by application callers remain unverified. This is a tested L-04 runtime scope, not full Rider parity.
