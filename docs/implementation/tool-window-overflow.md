# Tool window overflow

Tool buttons remain reachable when the window is too short for the complete stripe. The left stripe scrolls its left and bottom tools together; the right stripe scrolls its own tools. Larger windows keep their existing layout.

Implementation: `crates/rc-app/src/ui/stripes.rs`, adapted from CrabCraft commit `475ade7b5ef0020c953f68a51f1b897380c1ea06` under MIT OR Apache-2.0; the retained MIT notice is [crabcraft-MIT.txt](crabcraft-MIT.txt). No new assets or dependencies are included, and DotCraft branding and .NET tools are preserved.

Validation: `cargo test -p rc-app overflowing_stripes_scroll_to_and_toggle_the_last_tool -- --nocapture` passed. The headless regression scrolls both stripes in a 120-pixel viewport, checks the final button is visible, and opens and closes its tool window.

This completes the overflow behavior only; other CrabCraft interface improvements remain separate tasks.

## Upstream validation

Ported from fork branch `feat/tool-window-overflow` at `35016d5ccb8904f38f5be9b334781403a8918c47` onto original DotCraft `main` at `9e849cda99043fce68e8896d64a2608ff78141c8` (0.4.0). All 21 upstream tools remain in the existing catalog, including HTTP Client, Endpoints, Coverage, Monitoring, TODO, Bookmarks, Hierarchy, and Local History. The regression selects the actual last tool dynamically, so it tests the current catalog rather than assuming the older list.

On Linux with Rust 1.99.1, the stripe regression and all five translation-invariant tests passed on the upstream port. `git diff --check` also passed.

Validation uses a target directory dedicated to this isolated checkout, shared only with its sequential web-syntax validation branch. No GPU or real window input is required. Windows and macOS execution remain unvalidated.

```sh
CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 cargo test -p rc-app overflowing_stripes_scroll_to_and_toggle_the_last_tool --locked --offline --target-dir /home/rafael-mendes/Documentos/dotcraft/target/upstream-web --jobs 1 -- --nocapture
CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 cargo test -p rc-app -p rc-kit i18n::tests --locked --offline --target-dir /home/rafael-mendes/Documentos/dotcraft/target/upstream-web --jobs 1
```
