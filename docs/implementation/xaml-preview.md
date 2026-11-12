# Live XAML design sketch (#140, partial)

The XAML Preview tool window sits beside the editor. It reads the unsaved buffer of the active `.xaml` or `.axaml` file and updates as the buffer version changes. Parsing runs on a background task; stale results are ignored when the user edits again or switches files. The preview limits input to 256 KB, 500 elements, and 24 levels, and displays XML errors without blocking the editor.

The design surface draws a static approximation of common MAUI and Avalonia controls: pages, windows, vertical and horizontal stacks, borders, text, buttons, entries, checkboxes, and image placeholders. Unknown elements remain visible by name. No external MAUI or Avalonia runtime is launched.

Validation: two parser tests passed for controls from both frameworks, malformed markup, and depth limits; the tool window session identity test passed; compact `cargo check -p rc-app --locked --offline` passed. The implementation is native DotCraft code and copies no external code or assets.

This is a design sketch rather than a runtime renderer. It does not evaluate bindings, styles, templates, resources, grid sizing, custom controls, or platform behavior. MAUI/Avalonia live preview parity remains open in #140.

## Runtime preview integration blockers

Avalonia's [documented previewer protocol](https://github.com/AvaloniaUI/Avalonia/wiki/XAML-previewer-protocol) runs a host process shipped in the project's Avalonia NuGet package against a built application. The IDE must discover the version-specific previewer path and application output, start a local TCP-BSON server, send XAML updates, and handle rendered frames. DotCraft has no Avalonia previewer transport or fixture, and the validation environment has no Avalonia package cached. A reliable bridge needs a pinned Avalonia project and protocol fixture before the wire implementation can be tested.

.NET MAUI's [XAML Hot Reload](https://learn.microsoft.com/en-us/dotnet/maui/xaml/hot-reload?view=net-maui-10.0) updates a running Debug application with a debugger attached. DotCraft has no MAUI launch/deployment or Hot Reload transport, and `dotnet workload list` in the validation environment reports no installed workloads. A runtime preview requires a platform target and workload plus an app deployment and update channel. The static design sketch remains available without those prerequisites.
