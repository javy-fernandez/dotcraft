# Native workspace lifecycle with Shop.sln

An offscreen egui test starts on Welcome, drops the real six-project `Shop.sln` into the window, opens `Shop.Core/PriceCalculator.cs`, closes the project, and reopens Shop from Recent Projects. It checks the solution's six projects, the cleared editor on Close Project, and the restored C# tab after reopening.

Validation: `CARGO_TARGET_DIR=/var/tmp/dotcraft-zen-target CARGO_BUILD_JOBS=2 cargo test -p rc-app shop_solution_drop_close_and_recent_reopen_in_native_window -- --nocapture` on Linux.

Native title bars, OS file associations, and other platforms remain outside this test. It uses existing DotCraft behavior and the Shop fixture; no CrabCraft source or assets were copied.
