# Replace in Files for a .NET solution (D-08)

The Find tool window supports Replace All across a workspace. This slice moves closed-file disk replacement from the egui frame to a blocking worker and exposes Cancel Replace. The worker checks cancellation between files and uses `rc-search`'s atomic file replacement. Open documents are edited through their current editor buffers after the worker completes, preserving Undo. Closing the workspace requests cancellation; a closed document that was open when replacement started is reported and skipped instead of being rewritten on the UI thread.

A headless egui acceptance test opens the real six-project `Shop.sln` fixture and searches for `CouponCode` in C# files. It keeps `Order.cs` open, changes `OrdersController.cs` externally after search, and makes another matched closed file invalid UTF-8. Replace All updates the open editor buffer without saving it to disk, rewrites the closed controller from its current disk content while preserving the external edit, and leaves the invalid file untouched with a visible error. Undo restores the open document's original text.

Validation in this checkout's dedicated Cargo target directory:

- `cargo test -p rc-app replace_in_shop_csharp_files_keeps_open_editor_undo_and_closed_disk_edits` — passed (1 UI test).
- `cargo test -p rc-search replace` — passed (7 backend tests).
- `git diff --check` — passed.

This slice uses DotCraft's existing native Rust/egui Find UI, Shop fixture, and `rc-search` replacement code. No CrabCraft code or assets are copied. The tested scope is literal C# replacement across open and closed local files on Linux. Regex replacement, cancellation during a single large file, concurrent edits during the worker's run, large workspaces, and other platforms remain untested.
