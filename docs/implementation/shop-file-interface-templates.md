# File and interface templates in Shop workspace

The B-07 acceptance test opens real Shop.sln in the native egui app, then uses a temporary copied Shop.Core project to exercise New File and New Interface. New File creates nested `Orders/docs/notes.md` and opens it. New Interface creates `Orders/IClock.cs` with the `Shop.Core.Orders` namespace and opens it. The separate [PR 217](https://github.com/javy-fernandez/dotcraft/pull/217) verifies New Class creation and duplicate refusal in the same workspace style.

Validation: `CARGO_TARGET_DIR=/var/tmp/dotcraft-zen-target CARGO_BUILD_JOBS=2 cargo test -p rc-app shop_file_and_interface_templates_create_and_open_their_files -- --nocapture` (1 passed on Linux). The copied project and created files are removed after the test.

This verifies native file, class and interface creation in a .NET solution workspace. Other item kinds, project-system notification, background file creation, SDK compilation and other platforms remain untested. The implementation uses original DotCraft templates and the Shop fixture; no CrabCraft code or assets were copied.
