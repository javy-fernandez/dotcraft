# C-01 C# syntax paint in the Shop fixture

## Behavior

The editor paints C# syntax roles from its parser in both light and dark themes. Comment and string syntax colors now take precedence over overlapping semantic tokens, preserving the lexical meaning of those regions when a language server returns a broad or stale token.

## Validation and provenance

The acceptance test loads `tests/fixtures/Shop/src/Shop.Core/PriceCalculator.cs`, parses it, renders it through `EditorView`, and inspects the actual egui text shapes for keyword, documentation comment, and string colors. It injects overlapping semantic spans on the comment and string lines to verify the precedence fix in both themes. No CrabCraft code or assets were copied.

Commands: `cargo test -p rc-editor --test csharp_syntax_paint_shop` and `cargo test -p rc-editor --lib` with a dedicated Cargo target directory.

## Scope and limitations

This covers C# syntax paint for representative Shop source tokens in the built-in light and dark styles. It does not validate a full visual snapshot, custom themes, or live Roslyn semantic token delivery; semantic highlighting has its own C-02 inventory row.
