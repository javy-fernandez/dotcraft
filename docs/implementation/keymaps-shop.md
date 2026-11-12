# Rider and Visual Studio keymaps in the Shop workspace

The settings dialog can switch between Rider and Visual Studio keymaps while a solution is open. The active keymap drives global IDE shortcuts, editor shortcuts, menu shortcut labels, and, on macOS, the native menu accelerators. This change reinstalls the native menu after a keymap switch so its accelerators follow the new selection.

An offscreen egui acceptance test opens `tests/fixtures/Shop/Shop.sln`, invokes Go to File through the Rider shortcut, changes the setting to Visual Studio in the same app session, and invokes Go to File through the Visual Studio shortcut. It checks that each shortcut opens the Files tab. On Linux, the expected labels are `Ctrl+Shift+N` and `Ctrl+Shift+T`; the Rider macOS test expects `⇧⌘O`.

Validation: `CARGO_TARGET_DIR=/var/tmp/dotcraft-zen-target CARGO_BUILD_JOBS=2 cargo test -p rc-app rider_and_visual_studio_shortcuts_follow_settings_in_shop -- --nocapture` on Linux. The native macOS menu reinstall is not exercised on macOS in this environment.

This slice uses DotCraft's existing keymaps and Shop fixture. No CrabCraft source or assets were copied.
