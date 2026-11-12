# J-01 — Razor syntax in the Shop workspace

## Behavior

DotCraft recognizes `.razor` as Razor, parses Razor directives, HTML markup, and C# code through its native tree-sitter pipeline, and paints edited content in the native egui editor.

## Validation

- `CARGO_TARGET_DIR=/var/tmp/dotcraft-razor-syntax-shop-target CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 cargo test -p rc-syntax --test razor_shop -- --nocapture`
- `CARGO_TARGET_DIR=/var/tmp/dotcraft-razor-syntax-shop-target CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 cargo test -p rc-app razor_component_in_shop_workspace_paints_and_edits -- --nocapture`

The syntax test verifies a Razor parse tree without errors, Razor directive and HTML/C# highlight roles, and a foldable `@code` block. The editor test creates a temporary `Orders.razor` inside the real Shop solution workspace, opens it in DotCraft, checks its native editor paint shapes, edits its heading through `EditorState`, reparses, and verifies the new text is painted.

## Provenance and limits

The tests use the repository-owned Shop fixture and DotCraft's existing parser/editor code. No CrabCraft code or assets were copied. This validates the tested Razor syntax and editor path. It does not cover every Razor construct, live Razor language-server features, or an SDK build of the component; the .NET SDK is unavailable here.
