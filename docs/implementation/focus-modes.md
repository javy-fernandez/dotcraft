# Distraction-free and Zen modes (A-22)

Source baseline: `javy-fernandez/dotcraft` `becfb0f`. This is original DotCraft code; no CrabCraft source or assets were copied.

The View menu provides Distraction-free Mode and Zen Mode. Distraction-free Mode hides the tool-window stripes and panels while retaining the toolbar and status bar. Zen Mode also hides the toolbar and status bar. Both keep the existing dock layout in memory, so leaving the mode restores the same windows and sizes. The floating Exit Focus Mode button remains available when the normal menu is hidden; Escape also returns to normal mode when dispatched as an editor action.

The mode is temporary for the current process and does not alter the persisted workspace layout. All new interface labels use `rc_kit::i18n::tr` and have explicit English fallback entries in every locale.
Notification expiry continues to advance while Zen Mode hides the status bar.

Validation on Linux: `CARGO_TARGET_DIR=/tmp/dotcraft-zen-target cargo test -p rc-app focus_mode`. Two UI harness tests use the `Shop.sln` fixture, exercise the View menu and exit control, verify dock preservation, and check notification expiry in Zen Mode. Other platforms and native window chrome behavior have not been manually checked.
