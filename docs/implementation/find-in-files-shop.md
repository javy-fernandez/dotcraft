# Find in Files navigation with Shop.sln

Find in Files result lines now expose an accessible button name containing the relative path, line number and source text. This lets a user distinguish and activate results without relying on visual text painting.

An offscreen egui test searches the real Shop solution for a unique C# documentation sentence, activates its accessible result row, and verifies that `Shop.Core/PriceCalculator.cs` opens on the exact source line.

Validation: `CARGO_TARGET_DIR=/var/tmp/dotcraft-zen-target CARGO_BUILD_JOBS=2 cargo test -p rc-app shop_find_in_files_result_opens_exact_csharp_line -- --nocapture` on Linux.

This covers a literal C# search result. Regex, replacement, huge workspaces and other platforms remain untested. It uses DotCraft's existing search and Shop fixture; no CrabCraft source or assets were copied.
