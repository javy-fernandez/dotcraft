# Search Everywhere .NET file selection (A-07)

Source baseline: `javy-fernandez/dotcraft` `becfb0f`. This slice is original DotCraft code and copies no CrabCraft code or assets.

File results in Search Everywhere now expose their workspace-relative paths to accessibility tools. This distinguishes files with the same name in different .NET projects while leaving the visible name and directory presentation intact.

Validation on Linux: `CARGO_TARGET_DIR=/tmp/dotcraft-zen-target cargo test -p rc-app file_search_disambiguates_and_opens_a_dotnet_source_file`. The egui UI harness indexes real `Shop.sln` files, searches for `Program.cs`, chooses the Shop.Cli result by its relative path, and verifies that the source file opens.

This covers local file search and selection. Type/symbol search, remote workspaces, index updates, larger repositories and other platforms require separate acceptance.
