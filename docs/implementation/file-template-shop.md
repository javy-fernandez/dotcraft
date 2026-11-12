# Class template in a Shop project

An offscreen egui test keeps `Shop.sln` open, copies the real `Shop.Core.csproj` to a temporary project folder, and creates `Orders/Invoice.cs` through the New Class dialog. It verifies the generated namespace, class declaration and opened editor tab. Repeating the creation verifies that the existing file is neither overwritten nor closed.

Validation: `CARGO_TARGET_DIR=/var/tmp/dotcraft-zen-target CARGO_BUILD_JOBS=2 cargo test -p rc-app shop_class_template_creates_namespaced_file_and_refuses_duplicate -- --nocapture` on Linux.

This covers one C# class template and duplicate handling. Other item kinds, project-system integration, background file creation and other platforms remain. It uses existing DotCraft templates and the Shop fixture; no CrabCraft source or assets were copied.
