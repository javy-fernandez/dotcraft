# Go to File with a Shop C# source line

An offscreen egui test opens the real six-project `Shop.sln`, waits for its file index, invokes Go to File, enters `PriceCalculator.cs:12`, and presses Enter. It verifies that the C# file opens with the caret on source line 12.

Validation: `CARGO_TARGET_DIR=/var/tmp/dotcraft-zen-target CARGO_BUILD_JOBS=2 cargo test -p rc-app shop_go_to_file_name_and_line_opens_csharp_caret -- --nocapture` on Linux.

This covers indexed local C# file and line navigation. Duplicate basenames, live index updates, remote paths and other platforms remain outside this test. It uses existing DotCraft navigation and the Shop fixture; no CrabCraft source or assets were copied.
