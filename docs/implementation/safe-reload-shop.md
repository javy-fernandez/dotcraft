# Safe reload of a Shop C# source file

The conflict banner's **Load from Disk** choice now uses the document reload path that requests syntax analysis and reapplies the file's indentation style. The editor previously replaced the text directly, bypassing those updates.

An offscreen egui test opens the real `Shop.sln` workspace and a temporary copy of `Shop.Core/PriceCalculator.cs`. It makes an unsaved edit and an external disk edit, chooses **Load from Disk**, then repeats the conflict and chooses **Keep Mine**. It checks the editor buffer, modified/conflict flags, and saved disk content.

Validation: `CARGO_TARGET_DIR=/var/tmp/dotcraft-zen-target CARGO_BUILD_JOBS=2 cargo test -p rc-app shop_source_conflict_load_and_keep_choices -- --nocapture` on Linux.

Watcher timing, large files, binary files, and other platforms remain outside this test. The change uses existing DotCraft code and the Shop fixture; no CrabCraft source or assets were copied.
