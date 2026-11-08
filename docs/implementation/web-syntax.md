# Web language syntax

HTML, CSS, JavaScript, TypeScript, and TSX files now use dedicated tree-sitter grammars, highlighting queries, folding, and language-appropriate comments. JavaScript includes JSX highlighting and multiline component folding, as well as common module extensions; TypeScript includes `.mts` and `.cts`. Razor remains a distinct language. HTML retains tag closing and attribute quote assistance, including HTML void elements.

Queries were adapted from CrabCraft commit `475ade7b5ef0020c953f68a51f1b897380c1ea06`, licensed MIT OR Apache-2.0; the retained MIT notice is [crabcraft-MIT.txt](crabcraft-MIT.txt). Four tree-sitter grammar dependencies and their lockfile entries are added. No proprietary JetBrains code or assets are included.

## Validation

- `cargo test -p rc-syntax --offline`: six existing unit tests and three new web integration tests passed.
- `cargo test -p rc-app -p rc-editor native_html_keeps_tag_and_attribute_typing_assists --offline`: the editor regression passed.

The integration tests parse each language without syntax errors, verify representative token colors and multiline folds, and check extension and comment handling. JSX and TSX tests also cover native, component, and member tags, attributes, opening/closing tag pairs, and multiline component folding. Existing C# and Razor tests remain green.

This completes native syntax services for these five languages. Semantic completion, framework intelligence, embedded HTML script/style language layers, frontend debugging, CSS preprocessors, and web package-manager workflows remain separate tasks.

## Upstream port

The web syntax slice was ported from fork branch `feat/web-language-syntax` at `0915f2e307ec137138fb1d8d8139dfdc9cdf6eb1` onto original DotCraft `main` at `9e849cda99043fce68e8896d64a2608ff78141c8` (0.4.0). The port preserves the upstream snippet, local-history, breakpoint, and bookmark editor changes. It changes HTML detection from the Razor grammar to the HTML grammar while keeping Razor files on their existing grammar.

Validation uses a dedicated `target/upstream-web` directory to avoid sharing local-crate artifacts with other checkouts. Windows and macOS execution, embedded language layers, semantic services, and framework support remain unvalidated.

The upstream port is checked with these commands on Linux / Rust 1.99.1:

```sh
CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 cargo test -p rc-syntax --locked --offline --target-dir /home/rafael-mendes/Documentos/dotcraft/target/upstream-web --jobs 1
CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 cargo test -p rc-app -p rc-editor native_html_keeps_tag_and_attribute_typing_assists --locked --offline --target-dir /home/rafael-mendes/Documentos/dotcraft/target/upstream-web --jobs 1
```

On Linux, include `rc-app` when testing the editor so its target-specific X11/Wayland features enable the desktop backend. Testing `rc-editor` alone otherwise stops in upstream `winit` platform selection before the editor test can compile.
