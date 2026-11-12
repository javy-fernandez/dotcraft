# Distinguish duplicate filenames in Search Everywhere (A-07)

Search Everywhere already displays each file's relative directory beside its basename. Its accessibility label previously exposed only the basename, so two `Program.cs` results could not be distinguished by assistive controls. File result labels now use the repository-relative path while the visual layout remains unchanged.

A headless egui acceptance test opens the real six-project `Shop.sln` fixture from a directory with spaces. It waits for the workspace file index, searches the Files tab for `Program.cs:12`, confirms separate `src/Shop.Api/Program.cs` and `src/Shop.Cli/Program.cs` controls, and selects the API file. It verifies that the editor opens at line 12. It then searches again and selects the CLI file by its distinct path label.

Validation in this checkout's dedicated Cargo target directory:

- `cargo test -p rc-app search_everywhere_distinguishes_shop_program_files_by_path` — passed (1 UI test).
- `cargo test -p rc-app ui::search_popup::tests` — passed (3 UI tests).
- `git diff --check` — passed.

This slice uses DotCraft's existing native Rust/egui search popup and file index. No CrabCraft code or assets are copied. It verifies duplicate local C# filename navigation on Linux. Search Everywhere also includes types, symbols, actions, and settings; this PR does not complete those broader A-07 paths. Remote workspaces, very large indexes, and other platforms remain untested.
