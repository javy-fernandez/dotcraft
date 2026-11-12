# Clone a local .NET solution (I-09)

DotCraft's Git tool window already offers Clone Repository and Open cloned repository. This slice validates the complete local workflow with a Git repository containing the real `Shop.sln` fixture and its projects. The UI test first rejects an option-shaped source without creating the destination, then clones to a new path with spaces, opens the cloned workspace, and confirms that the solution model contains six projects.

The workflow uses DotCraft's native Rust/egui Git tool UI and `rc-vcs` CLI backend. No CrabCraft code or assets are copied.

Validation: `cargo test -p rc-app clone_shop_solution_from_the_git_tool_window` passed its headless UI test. `cargo test -p rc-vcs` passed 21 unit, 18 repository, and six tools tests; one optional LFS test remained ignored. Both commands used this checkout's dedicated Cargo target directory. The existing backend test also rejects reuse of an existing destination and unsafe transport syntax.

The tested scope is cloning a local Git repository on Linux and opening its .NET solution in DotCraft. Remote hosts, authentication, proxy failures, large clones, and other platforms remain unverified. Cancellation is available through the Git tool window's running-operation control, but it is not exercised by this fixture test.
