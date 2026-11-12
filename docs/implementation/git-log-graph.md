# Git log branch graph (I-14)

The Git tool window now draws the branch and merge lanes computed by `rc-vcs::Graph`, including diagonal edges and stable colors. It previously drew every commit on the same vertical line. Commit rows also expose their subjects as selectable labels for keyboard and accessibility tooling. The existing commit details and file selection behavior remain.

This uses DotCraft's existing native Rust Git log model and egui UI. No CrabCraft code or assets were copied.

Validation: a temporary real Git repository with a side branch and a merge supplies the log. The `rc-app` UI harness verifies that the displayed graph contains a diagonal branch edge and that selecting the side commit updates the selected commit. Existing `rc-vcs` graph tests cover lane assignment. The test runs in a dedicated Cargo target directory.

The tested scope is the local Git log and graph in the desktop UI on Linux. The view still loads at most 500 commits at once; pagination, very large histories, platform-specific rendering, and live GPU screenshot review remain outside this scope.
