# J-02 XML syntax in Shop project files

## Behavior

DotCraft recognizes `.csproj` as XML, parses project files with and without a UTF-8 BOM, and paints element tags, attribute names, and attribute values with their theme colors. Editing a target framework value through `EditorView` keeps the document parseable and preserves tag coloring after a reparse.

## Validation and provenance

The acceptance test loads `tests/fixtures/Shop/src/Shop.Core/Shop.Core.csproj` (with BOM) and `tests/fixtures/Shop/src/Shop.Api/Shop.Api.csproj` (without BOM). It checks their parse trees, renders the actual egui text shapes through `EditorView` in both built-in themes, and edits `TargetFramework` through the editor before reparsing. The existing implementation passed without a product-code change. No CrabCraft source or assets were copied.

Commands: `cargo test -p rc-editor --test xml_syntax_shop` and `cargo test -p rc-editor --lib` with a dedicated Cargo target directory. Both passed (1 and 17 tests respectively).

## Scope and limitations

This covers XML parse, edit, and syntax paint for real `.csproj` files under the built-in themes. XML schema validation, namespaces, entity behavior, and custom themes are outside the tested J-02 core scope.
