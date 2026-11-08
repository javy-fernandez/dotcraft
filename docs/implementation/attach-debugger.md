# Attach to a running process

With a workspace open, use Attach to Process from Search Everywhere, the run configuration menu, or the native Run menu. Enter the decimal process ID of an existing .NET process. DotCraft rejects zero, malformed IDs, its own PID, and attempts to replace an active debug session. No build or application launch is required. A workspace without a run configuration is sufficient. From the welcome screen or a standalone file, the PID dialog explains that a project must be opened and disables Attach, including the Enter shortcut; this prevents starting an invisible debugging session in the standalone-file layout. Restart is disabled for attached sessions so it cannot launch an unrelated run configuration.

The existing netcoredbg adapter is located or downloaded on first use. Attachment preserves watches and resets previous session presentation state. Stop disconnects the debugger from an attached process rather than terminating that process.

Closing or switching workspaces stops the debugger and cancels deferred attachment. Stop is available in both toolbars while an adapter download is pending; cancellation discards queued completion events so an old download cannot start a new session afterward.

## Upstream integration and validation

This slice ports fork branch `feat/attach-to-process` at
`d675eafa2a1eb857c89fe43cbf528dd3a63f9eee` to original DotCraft 0.4.0,
upstream commit `9e849cda99043fce68e8896d64a2608ff78141c8`. It preserves
upstream's About/update dialog, templates, bookmarks, monitoring, coverage,
HTTP/Endpoints tools, and existing locale entries. It does not depend on the
fork's inventory or other feature branches.

Use an isolated Cargo target directory for this checkout:

```sh
CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 cargo test -p rc-dap --lib --locked --offline --target-dir target/upstream-attach --jobs 1
CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 cargo test -p rc-app attach_ --locked --offline --target-dir target/upstream-attach --jobs 1
CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 cargo test -p rc-app -p rc-kit i18n::tests --locked --offline --target-dir target/upstream-attach --jobs 1
```

Fresh checks of this upstream port passed on Linux: 19 DAP unit tests, seven
attachment/PID/headless application tests, and five translation invariants.
Cargo resolution used `--locked --offline` in the dedicated
`target/upstream-attach` directory. These are targeted checks, not a full
workspace or cross-platform validation run.

The headless regressions cover PID parsing, opening Attach without a run
configuration, workspace requirements for button and Enter activation, invalid
PID rejection before adapter startup, workspace closure, stale download events,
Stop in both toolbars, and attach-versus-launch Restart behavior. The DAP suite
also verifies that fake-adapter log snapshots read only completed records. A
protocol regression attaches the scripted adapter to a controlled Python target,
checks `terminateDebuggee=false` on disconnect, waits for adapter exit, and
verifies that the target is still alive. This validates detach lifecycle without
requiring a .NET installation; it does not replace live netcoredbg validation.

The live regression remains explicitly ignored for hosts without an installed
.NET 8 SDK, netcoredbg, and restored fixture dependencies:

```sh
cargo test -p rc-app attaches_from_the_ide_and_stop_leaves_the_target_running -- --ignored --nocapture
```

It builds and starts the sample ASP.NET application, attaches from the IDE,
stops debugging, waits for the adapter to exit, and verifies HTTP 200 from the
still-running target. The fork previously recorded this live regression passing
on Linux with SDK 8.0.427 and ASP.NET runtime 8.0.33. That historical result does
not establish fresh live validation of the upstream port. No .NET executable is
available on the current validation host's PATH.

## Scope

PID entry is supported; a process browser, remote process attachment, and automatic runtime filtering are separate tasks. OS process permissions and netcoredbg runtime compatibility still apply. This does not establish debugging support for every .NET runtime or host platform.

Implementation reuses DotCraft's existing DAP attach backend and first-use adapter installation. New UI strings use English source text and explicit English locale fallbacks in `scripts/i18n/attach-process.tsv`.
