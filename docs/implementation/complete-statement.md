# C-17: Complete statement (first slice)

`Ctrl+Shift+Enter` on Windows/Linux and `Cmd+Shift+Enter` on macOS completes a supported single-line C# statement and starts a new line at the same indentation. The semicolon and newline form one undo step. Existing semicolons are preserved.

This slice covers `return`, `throw`, `var`, `await`, `break`, `continue`, simple assignments, and calls ending in `)`. It requires a current syntax tree and a single caret. Ambiguous lines, control-flow headers, comments, read-only files, and other languages remain unchanged. It does not synthesize missing parentheses or braces, and it does not claim full Rider parity. A live desktop acceptance pass remains open.

Validation: `CARGO_TARGET_DIR=/tmp/dotcraft-editor-slice-target cargo test -p rc-editor --lib` covers the shortcut, one-step undo, insertion, and safeguards.
