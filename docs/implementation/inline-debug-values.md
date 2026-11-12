# Inline variable values while debugging

## Behavior

- When the debugger stops, DotCraft requests the selected frame's local and argument variables through the existing DAP session. Values appear after the nearest matching source line in the 25-line window ending at the execution point. Up to three values appear per line, with long values shortened to keep the editor readable.
- A value changed since the previous stop gets a highlighted background. Values disappear when execution continues or the debug session ends.
- Settings > Editor > Show inline variable values while debugging turns the display on or off independently of Roslyn inlay hints. It defaults to on and is persisted in settings.

## Validation

- Compact `cargo check -p rc-app --locked --offline --target-dir /home/rafael-mendes/Documentos/dotcraft/target/issue-131 --jobs 2` passed on Linux.
- The focused inline-value test passed, covering identifier boundaries and changed-value tracking. The existing live .NET debugger acceptance test passed with an added assertion that a `subtotal` value from netcoredbg reaches the active editor. This run used .NET SDK 8.0.428 for the fixture and netcoredbg 3.2.0-1092.
- The `rc-core` settings default test and `git diff --check` passed.

## Provenance and limits

This is original DotCraft code using its existing DAP client and editor painter. No third-party code or assets were copied. MIT OR Apache-2.0 applies.

Line placement uses identifier matching near the selected frame's execution point. It does not use Roslyn symbol bindings, so a repeated identifier in a comment or unrelated scope can receive the value. Complex values are shown as text; expansion remains in the Variables tool window. Linux was the validated platform.
