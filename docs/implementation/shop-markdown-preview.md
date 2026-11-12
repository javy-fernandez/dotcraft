# Markdown preview in Shop workspace

The J-04 acceptance test opens the real Shop.sln in the native egui app and creates a temporary Markdown file in that workspace. The split preview exposes the rendered `Shop notes` heading through the accessibility tree. The test switches through Preview Only, Editor Only, and Editor and Preview and verifies each document mode.

Validation: `CARGO_TARGET_DIR=/var/tmp/dotcraft-zen-target CARGO_BUILD_JOBS=2 cargo test -p rc-app shop_markdown_preview_switches_modes_and_renders_heading -- --nocapture` (1 passed on Linux). The temporary file is removed after the test.

This covers local Markdown heading rendering and mode selection in a .NET solution workspace. Live edit synchronization, links, images, long documents, other Markdown constructs, custom themes and other platforms remain untested. The implementation is existing original DotCraft code; no CrabCraft code or assets were copied.
