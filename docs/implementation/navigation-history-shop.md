# Navigation history with a removed Shop source file

Back and Forward now skip missing history entries without first recording the current location in the opposite stack. Previously a removed target consumed one Back action and left Forward pointing to the current file.

An offscreen egui test opens `Shop.sln`, records a caret in `Shop.Core/Order.cs`, records a second location in a temporary copy of `PriceCalculator.cs`, and opens `Shop.Api/Program.cs`. After deleting the copied file, Back skips it and returns to the exact Order caret; Forward returns to Program.

Validation: `CARGO_TARGET_DIR=/var/tmp/dotcraft-zen-target CARGO_BUILD_JOBS=2 cargo test -p rc-app shop_navigation_skips_removed_source_and_keeps_forward_location -- --nocapture` on Linux.

This covers local navigation history with a removed file. Renames, remote files, and other platforms remain untested. It uses DotCraft's existing navigation and Shop fixture; no CrabCraft source or assets were copied.
