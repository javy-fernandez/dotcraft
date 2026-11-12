# Recent Projects welcome flow (A-08)

Source baseline: `javy-fernandez/dotcraft` `becfb0f`. This slice is original DotCraft code and copies no CrabCraft code or assets.

Welcome-screen recent project rows expose their full paths to accessibility tools and mark missing paths as unavailable. Selecting a recent solution opens the workspace and keeps it first in recent history.

Validation on Linux: `CARGO_TARGET_DIR=/tmp/dotcraft-zen-target cargo test -p rc-app recent_solution_opens_from_the_welcome_screen`. The egui UI harness selects the real `Shop.sln` fixture from Welcome and verifies workspace and recent-history state.

This covers an existing local solution at a standard desktop size. Missing-path removal, compact layouts, remote paths, other platforms and SDK build/run are outside this acceptance slice.
