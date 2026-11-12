# Terminal scrollback search

The Terminal tool window can find text in the active tab's retained scrollback. The
search bar opens from the header and provides previous/next navigation, a result count,
and Refresh. Results update after terminal output settles briefly and are highlighted in
the terminal grid without changing the user's text selection. Enter and Shift+Enter
navigate while the query field has focus.

The search is a case-insensitive literal match. It checks the newest 4,000 retained
terminal lines, including history above the visible viewport, and returns at most 1,000 of the newest
matches to the UI. Empty queries return no matches. Results belong to the active tab;
switching tabs restores each tab's search state.

## Validation

Backend tests cover history, case-insensitive matching, result ordering and bounds,
and navigation to offscreen lines. The terminal and application crates are checked
with a dedicated Cargo target directory. Exact commands and results are recorded in
the feature pull request.

## Provenance and limits

The implementation uses DotCraft's existing Rust terminal model and egui widget. No
third-party code or assets were copied. Search runs on a cancellable worker in short
grid-lock batches when the query changes, output changes, or the user presses Refresh.
Results from a changed terminal generation are discarded. It does not search
output discarded from scrollback or older than the 4,000-line scan limit, and it does
not include alternate-screen buffers in scrollback history.
