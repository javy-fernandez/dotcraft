# Tool window overflow

Tool buttons remain reachable when the window is too short for the complete stripe. The left stripe scrolls its left and bottom tools together; the right stripe scrolls its own tools. Larger windows keep their existing layout.

Implementation: `crates/rc-app/src/ui/stripes.rs`, adapted from CrabCraft commit `475ade7` under MIT OR Apache-2.0. No new assets or dependencies are included, and DotCraft branding and .NET tools are preserved.

Validation: `cargo test -p rc-app overflowing_stripes_scroll_to_and_toggle_the_last_tool -- --nocapture` passed. The headless regression scrolls both stripes in a 120-pixel viewport, checks the final button is visible, and opens and closes its tool window.

This completes the overflow behavior only; other CrabCraft interface improvements remain separate tasks.
