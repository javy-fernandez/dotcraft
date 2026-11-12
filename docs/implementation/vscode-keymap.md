# Visual Studio Code keymap preset

## Behavior

- Settings > Keymap now offers VS Code alongside the existing Rider and Visual Studio presets. The choice is persisted in user settings and changes active editor, menu, and global shortcuts immediately.
- macOS and Windows/Linux use separate VS Code bindings for common editing, search, navigation, build, debug, and tool-window actions. Existing Rider and Visual Studio settings remain compatible.
- DotCraft actions without a single-stroke VS Code equivalent retain their previous Rider shortcut. VS Code's two-stroke keyboard chords are not modeled by DotCraft's current keymap API.

## Validation

- `cargo test -p rc-editor --lib keymap::tests --locked --offline --target-dir /home/rafael-mendes/Documentos/dotcraft/target/issue-136 --jobs 2` passed all three focused tests, validating representative platform bindings and detecting duplicate chords.
- `cargo test -p rc-core --lib settings::tests::vscode_keymap_survives_settings_round_trip --locked --offline --target-dir /home/rafael-mendes/Documentos/dotcraft/target/issue-136 --jobs 2` passed, validating settings persistence and existing Rider data.
- `CARGO_PROFILE_TEST_DEBUG=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0 cargo test -p rc-app keymap_acceptance_tests --locked --offline --target-dir /home/rafael-mendes/Documentos/dotcraft/target/issue-136 --jobs 2 -- --test-threads=1` passed. The headless acceptance test changes presets within the Shop workspace and exercises Go to File with each choice.
- `CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0 cargo check -p rc-app --locked --offline --target-dir /home/rafael-mendes/Documentos/dotcraft/target/issue-136 --jobs 2` passed with the existing `open_files::push` dead-code warning.
- All seven locale JSON files parse successfully; the new preset label uses an explicit English fallback in each locale.

## Provenance and limits

Mappings were checked against the [official VS Code default keyboard shortcuts reference](https://code.visualstudio.com/docs/reference/default-keybindings). This implementation contains original DotCraft code; no third-party code or assets were copied. The repository's MIT OR Apache-2.0 licensing applies.

The preset maps supported DotCraft actions, not every VS Code command. VS Code multi-stroke chords, platform or keyboard-layout overrides, and importing a user's `keybindings.json` remain unsupported.
