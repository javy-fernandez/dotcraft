# Status bar .NET fixture validation (A-14)

Source baseline: `javy-fernandez/dotcraft` `becfb0f`. This slice adds acceptance coverage to the existing native egui status bar; it copies no CrabCraft code or assets.

The status bar shows the workspace and active file breadcrumbs, encoding, line ending, and caret position. Its caret-position control opens Go to Line. This slice gives the custom status controls explicit accessibility labels so assistive tooling can identify their values.

Validation on Linux: `CARGO_TARGET_DIR=/tmp/dotcraft-zen-target cargo test -p rc-app status_bar_reports_dotnet_document_and_opens_line_navigation`. The UI harness loads the real `Shop.sln` fixture, opens `Shop.Cli/Program.cs`, checks the visible metadata, and activates Go to Line.

This checks the UTF-8 BOM/LF fixture and standard desktop layout. Other encodings, line endings, compact widths, and platform chrome are outside this acceptance slice.
