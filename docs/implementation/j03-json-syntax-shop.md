# J-03 JSON syntax in the Shop API fixture

## Behavior

DotCraft parses `appsettings.json` as JSON and paints property keys and string values with distinct syntax colors in the built-in light and dark themes. The JSON query now captures keys once: the specific key pattern follows the generic string pattern so it wins precedence without a duplicate capture.

## Validation and provenance

The acceptance test loads both `tests/fixtures/Shop/src/Shop.Api/appsettings.json` and `appsettings.Development.json`, checks that their parse trees have no errors, renders them through `EditorView`, and inspects painted egui text colors for `Logging` and `Information`. It then types into the `Information` value through the editor, reparses, and verifies the string color and valid tree after editing. No CrabCraft source or assets were copied.

Commands: `cargo test -p rc-editor --test json_syntax_shop`, `cargo test -p rc-editor --lib`, and `cargo test -p rc-syntax --lib` with a dedicated Cargo target directory. All passed (1, 17, and 6 tests respectively).

## Scope and limitations

The acceptance covers JSON parsing, editing, and syntax paint in the Shop API settings files under the built-in themes. It does not validate JSON Schema, formatting, JSONC comments, or custom themes. Those are outside the tested J-03 core scope.
