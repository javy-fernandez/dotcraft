# Continuous testing (#134)

The Tests window offers three modes: Off, On Save, and On Build. On Save waits briefly after an explicit editor save and reruns affected test projects. On Build reruns after a successful Build or Rebuild when relevant files changed. Mode and scope are stored with the workspace session.

The affected scope selects the owning project for each changed file and walks .NET project references to find dependent test projects. Shared files without a clear owner conservatively select all test projects. An unrelated project selects no tests. The current test session scope repeats a test target and filter captured with **Use current test run**. Runs use the existing Tests window, progress and result counters, cancellation, and raw output. Multiple affected projects run sequentially so each run can be observed and cancelled.

Project metadata is loaded on a background task. The save debounce and test processes do not block the UI thread. Turning the mode Off cancels an automatic run and clears queued work. The Tests window Stop action also clears the automatic queue.

## Validation

- Project graph tests cover dependent, unrelated, and shared file selection.
- Workspace session round trip covers the saved mode, scope, target, and filter.
- `cargo check -p rc-app --locked --offline` with compact debug profiles.

## Provenance and limits

Implementation is native DotCraft code built on its existing .NET project parser and test runner; no external code or assets were copied. The project graph is built when the workspace opens. Linked source files and shared build configuration files trigger every test project. The Tests window shows the most recent project result, while the status line reports the current automatic project or completion; it does not aggregate results across several projects.
