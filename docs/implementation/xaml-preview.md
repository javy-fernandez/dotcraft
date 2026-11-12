# Live XAML design sketch (#140, partial)

The XAML Preview tool window sits beside the editor. It reads the unsaved buffer of the active `.xaml` or `.axaml` file and updates as the buffer version changes. Parsing runs on a background task; stale results are ignored when the user edits again or switches files. The preview limits input to 256 KB, 500 elements, and 24 levels, and displays XML errors without blocking the editor.

The design surface draws a static approximation of common MAUI and Avalonia controls: pages, windows, vertical and horizontal stacks, borders, text, buttons, entries, checkboxes, and image placeholders. Unknown elements remain visible by name. No external MAUI or Avalonia runtime is launched.

Validation: two parser tests passed for controls from both frameworks, malformed markup, and depth limits; the tool window session identity test passed; compact `cargo check -p rc-app --locked --offline` passed. The implementation is native DotCraft code and copies no external code or assets.

This is a design sketch rather than a runtime renderer. It does not evaluate bindings, styles, templates, resources, grid sizing, custom controls, or platform behavior. MAUI/Avalonia live preview parity remains open in #140.
