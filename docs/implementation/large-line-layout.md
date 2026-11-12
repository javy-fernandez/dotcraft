# C-16: Bounded layout for long editor lines

The editor now lays out at most 4,096 characters of any line longer than that threshold. The visible window follows horizontal scroll and the caret. The complete text stays in the buffer, so editing, saving and undo remain available; a long line no longer allocates a galley and column map proportional to its full length on each frame.

This applies to every language. Long-line windows omit syntax colors, inlay hints and fold placeholders. Horizontal placement is approximate, especially with tabs and wide glyphs. The existing file-size limits are unchanged. A live desktop performance and interaction pass remains open, so this is a partial C-16 safeguard rather than full large-file support.

Validation: `CARGO_BUILD_JOBS=2 CARGO_TARGET_DIR=/tmp/dotcraft-editor-large-line-target cargo test -p rc-editor --lib` includes a 100,000-character line regression that checks bounded galley size, caret mapping and unchanged buffer content.
