# Attach to a running process

With a workspace open, use Attach to Process from Search Everywhere, the run configuration menu, or the native Run menu. Enter the decimal process ID of an existing .NET process. DotCraft rejects zero, malformed IDs, its own PID, and attempts to replace an active debug session. No build or application launch is required. A workspace without a run configuration is sufficient. From the welcome screen or a standalone file, the PID dialog explains that a project must be opened and disables Attach, including the Enter shortcut; this prevents starting an invisible debugging session in the standalone-file layout. Restart is disabled for attached sessions so it cannot launch an unrelated run configuration.

The existing netcoredbg adapter is located or downloaded on first use. Attachment preserves watches and resets previous session presentation state. Stop disconnects the debugger from an attached process rather than terminating that process.

Closing or switching workspaces stops the debugger and cancels deferred attachment. Stop is available in both toolbars while an adapter download is pending; cancellation discards queued completion events so an old download cannot start a new session afterward.

## Validation

```sh
cargo test -p rc-app attach_ --offline
cargo test -p rc-app validates_external_process_ids --offline
cargo test -p rc-app debugger::tests --offline
cargo test -p rc-dap --lib --offline
cargo test -p rc-app restart_is_disabled_for_attach_sessions_and_available_for_launches --offline
cargo test -p rc-app -p rc-kit every_text_in_the_code_has_a_translation --offline
```

The attach action, workspace requirement for button and Enter activation, invalid PID rejection, PID parsing, workspace closure, stale download events, Stop interactions, and attach-versus-launch Restart behavior have passed targeted tests. The attach action also compiles independently from the parallel HTTP, web, stash, settings, and stripe changes.

The live regression requires an installed .NET 8 SDK, netcoredbg, and restored fixture dependencies:

```sh
cargo test -p rc-app attaches_from_the_ide_and_stop_leaves_the_target_running -- --ignored --nocapture
```

It builds and starts the sample ASP.NET application, attaches from the IDE, stops debugging, waits for the adapter to exit, and verifies the target still serves an HTTP request. Live verification passed with .NET SDK 8.0.427, ASP.NET runtime 8.0.33, and the downloaded netcoredbg adapter: one test passed in 4.63 seconds, including the HTTP 200 response after detaching. Dependencies were restored using temporary SDK, CLI-home, and NuGet directories under `/tmp`. The test stays explicitly ignored for hosts without these prerequisites.

The existing live `debugs_the_cli_and_the_api_from_the_ide` regression also passed, exercising conditional breakpoints, Step Over, Run to Cursor, values, watches, evaluation, hover values, and the API launch path. The DAP unit suite passes 18 tests. Its fake-adapter log reader accepts only newline-completed records; a regression verifies that a partially appended record cannot cause a JSON parse race.

## Scope

PID entry is supported; a process browser, remote process attachment, and automatic runtime filtering are separate tasks. OS process permissions and netcoredbg runtime compatibility still apply. This does not establish debugging support for every .NET runtime or host platform.

Implementation reuses DotCraft's existing DAP attach backend and first-use adapter installation. New UI strings use English source text and explicit English locale fallbacks in `scripts/i18n/attach-process.tsv`.
