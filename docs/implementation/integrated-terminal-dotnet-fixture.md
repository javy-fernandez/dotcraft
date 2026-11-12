# Integrated terminal in a .NET workspace (L-01)

The existing terminal opens a shell in the active workspace, displays its PTY output in the Terminal tool window, and supports multiple tabs. This slice adds acceptance coverage against the `Shop.sln` .NET fixture. A real shell checks that the solution file is visible from the terminal's working directory; the headless egui test verifies the Terminal window opens and creates another tab in that same directory. The `rc-term` integration test also verifies process exit and the terminal grid output.

The terminal implementation is native DotCraft Rust/egui using the existing `portable-pty` and `alacritty_terminal` dependencies. No CrabCraft code or assets are copied.

Validation: `cargo test -p rc-term` passed 15 unit and 10 session integration tests; the GPU snapshot test and a doctest remain ignored. `cargo test -p rc-app terminal_tool_window_uses_the_dotnet_solution_directory_and_opens_tabs` passed its headless UI acceptance test. Both commands used this checkout's dedicated Cargo target directory. The PTY tests cover input, output, resize, color, selection, and process lifecycle.

This validates local shell integration with a real .NET solution fixture on Linux. The .NET SDK is unavailable in the test environment, so SDK commands were not run in this terminal. Windows and macOS terminal behavior and GPU screenshot rendering remain unverified.
