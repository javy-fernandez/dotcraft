# Run configuration selection in Shop.sln

An offscreen egui test opens the real six-project `Shop.sln`, chooses the Shop.Cli run configuration from the toolbar popup, saves the workspace session and reopens the solution in a new app instance. The selected configuration is retained.

Validation: `CARGO_TARGET_DIR=/var/tmp/dotcraft-zen-target CARGO_BUILD_JOBS=2 cargo test -p rc-app shop_run_configuration_selection_persists_across_workspace_reopen -- --nocapture` on Linux.

This checks discovery, toolbar selection and persistence of an application configuration. Launch profile changes, actual SDK execution, compound configuration editing and other platforms remain outside this test. It uses existing DotCraft behavior and the Shop fixture; no CrabCraft source or assets were copied.
