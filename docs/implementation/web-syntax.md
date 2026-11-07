# Web language syntax

HTML, CSS, JavaScript, TypeScript, and TSX files now use dedicated tree-sitter grammars, highlighting queries, folding, and language-appropriate comments. JavaScript includes JSX and common module extensions; TypeScript includes `.mts` and `.cts`. Razor remains a distinct language. HTML retains tag closing and attribute quote assistance, including HTML void elements.

Queries were adapted from CrabCraft commit `475ade7`, licensed MIT OR Apache-2.0. Four tree-sitter grammar dependencies and their lockfile entries are added. No proprietary JetBrains code or assets are included.

Validation:

- `cargo test -p rc-syntax --offline`: six existing unit tests and three new web integration tests passed.
- `cargo test -p rc-app -p rc-editor native_html_keeps_tag_and_attribute_typing_assists --offline`: the editor regression passed.

The integration tests parse each language without syntax errors, verify representative token colors and multiline folds, and check extension and comment handling. JSX and TSX tests also cover native, component, and member tags, attributes, and opening/closing tag pairs. Existing C# and Razor tests remain green.

This completes native syntax services for these five languages. Semantic completion, framework intelligence, embedded HTML script/style language layers, frontend debugging, CSS preprocessors, and web package-manager workflows remain separate tasks.
