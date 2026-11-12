# L-03: file watcher rename and atomic-save events

## Behavior

The workspace and external-document watchers classify creation, removal, and modification events by their `notify` event kind. For rename events, the source is reported as removed and the destination as created. This preserves both paths when a C# file is renamed or an editor replaces a file through an atomic save, so the file index and language server receive the corresponding path changes.

## Validation

- `CARGO_TARGET_DIR=/tmp/dotcraft-core-slice-target cargo test -p rc-core` — 13 passed on Linux.
- A real filesystem test watches a small .NET project containing `App.csproj`. It creates and renames `Program.cs`, then replaces `Main.cs` with a temporary C# file via rename. The watcher emits the expected source-removal and destination-creation events.
- A separate test covers `notify`'s `From`, `To`, and `Both` rename modes when both paths exist at callback time.
- `git diff --check` passed.

## Provenance and limits

This change extends DotCraft's existing watcher at upstream commit `becfb0fe316203df2d8d7c1e99c6a54bd37cd13b`; no external code or assets were copied. Live editor, file index, and language-server behavior were not exercised. The atomic replacement filesystem test runs on Unix; Windows and macOS watcher backends remain untested. Ambiguous rename modes still use path existence as a fallback.
