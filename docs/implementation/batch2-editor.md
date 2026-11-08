# Batch 2: editor workflows

This slice extends the native Rust/egui editor on the consolidated PR 108 baseline.
The implementations are original work against DotCraft's existing editor, snippets,
workspace index, and session APIs; no additional external code or assets were copied.
Existing EditorConfig adaptation notices and MIT OR Apache-2.0 licenses are retained.

| Inventory | Implemented scope | Remaining scope |
| --- | --- | --- |
| C-18 | Cached EditorConfig policies trim trailing spaces/tabs, insert a missing final newline, and select UTF-8 BOM and LF/CRLF on Save. Text cleanup is undoable and maps caret/bookmarks. | Numeric-range globs, other encodings, full formatting, and save before asynchronous rule resolution remain outside this slice. `false` preserves existing final newlines; it does not remove them. |
| C-23 | Tools → Editor Workflows edits, previews, inserts, deletes, and persists workspace C# snippets. Abbreviation + Tab expands custom entries with snippet placeholders and undo; custom names override built-ins. | Template import/export, semantic applicability, and other languages. |
| C-24 | Explicit C# `.if`, `.not`, `.null`, `.notnull`, `.return`, and `.await` transforms for standalone identifier/member paths. Preserves indentation and undo. | Requires a current background syntax tree; no type inference, arbitrary expressions, calls, or multiple carets. String/comment contexts are excluded. |
| D-16 | A cancellable background scan of the current workspace index finds C#, Razor, XAML, designer, and test filename families; result links open the chosen file. | Filename conventions only, at most 200 results; no semantic relationship inference. |
| D-17 | Toggle, open, and remove up to 500 workspace favorite files, restored through existing session persistence. Existing line/mnemonic bookmarks remain intact. | No favorite groups, symbol favorites, or automatic rename tracking. |

Template editing accepts 1–32 ASCII letters/digits/underscores (not an initial digit),
up to 16 KiB per body and 100 templates. Existing session storage is reused. Related
file discovery performs no filesystem reads on the UI thread, cancels when the window
or workspace closes, and discards cancelled results. Save uses the already resolved
EditorConfig policy and the existing document save pipeline; this slice does not change
that pipeline's synchronous file-write behavior.

## Validation

- A real temporary C# document and `.editorconfig` exercise rule discovery, save bytes,
  BOM/CRLF, whitespace cleanup, caret/bookmark preservation, clean state, and undo.
  An action-level test checks explicit Save on a clean file and read-only protection.
  Save notifications flush pending editor changes to the initialized language server
  before didSave. Failed saves do not emit a successful-save notification.
- Editor command tests exercise custom Tab expansion, snippet selection, undo, all six
  postfix forms, Unicode member paths, stale syntax, and multiline string/comment rejection.
- Headless egui controls save a custom template, toggle favorites, persist/reload the
  session, and open a related Razor file. A worker test checks indexed matching and
  cancellation; bounds and filename families have dedicated assertions.
- Final combined commands and counts are recorded in [batch2.md](batch2.md).

No live .NET compiler/runtime or GPU screenshot acceptance is claimed for this slice.
Broader inventory families remain partial rather than claiming full Rider parity.
