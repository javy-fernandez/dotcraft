# C-24 postfix templates in the Shop C# editor

## Behavior

The editor expands the six supported postfix keys (`if`, `not`, `null`, `notnull`, `return`, and `await`) from an identifier or member expression on its own C# line when Tab is pressed. Expansion replaces the postfix source and is undoable.

## Validation and provenance

The acceptance test starts from `tests/fixtures/Shop/src/Shop.Core/PriceCalculator.cs`, adds `taxable.<key>` immediately after the existing taxable calculation in memory, parses the C# file, sends Tab through `EditorView`, checks each of the six outputs, and confirms undo restores the exact source for each key. No CrabCraft source or assets were copied.

Commands: `cargo test -p rc-editor --test postfix_shop` and `cargo test -p rc-editor --lib` with a dedicated Cargo target directory. Both passed (1 integration test covering six expansions and 17 library tests).

## Scope and limitations

This covers the existing six postfix transformations for simple identifier/member expressions after parsing is current. The editor intentionally excludes calls, literals, comments, multi-caret edits, and type-aware templates. Immediate Tab before the background parser catches up and popup discovery are not tested. C-24 completion is limited to the supported transformations and their Tab/undo workflow.
