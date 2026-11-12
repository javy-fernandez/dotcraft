# Live NuGet search in Shop workspace

The B-12 acceptance test opens real Shop.sln in the native NuGet window, searches nuget.org for `Newtonsoft.Json`, waits for the result and verifies that the package has a version list and an accessible result row. The existing rc-dotnet integration test also searches nuget.org, checks verified metadata and stable versions, and lists versions. PR #221 supplies bounded query/page/response handling.

Validation on Linux: `CARGO_TARGET_DIR=/var/tmp/dotcraft-zen-target CARGO_BUILD_JOBS=2 cargo test -p rc-app shop_nuget_browse_shows_a_live_package_search_result -- --ignored --nocapture` (1 passed) and `cargo test -p rc-dotnet searches_nuget_org_and_lists_versions -- --ignored --nocapture` (1 passed). Both tests are ignored by default because they need network access.

This verifies basic live search and result display for a .NET solution. Private feeds, authentication, pagination UI, offline behavior, large result sets, source switching and other platforms remain untested. The implementation uses original DotCraft code and its Shop fixture; no CrabCraft code or assets were copied.
