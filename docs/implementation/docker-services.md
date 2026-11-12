# Docker Services window (#122, partial)

The Services stripe now opens a Docker container list. The list comes from `docker ps --all` and shows each container's name, image, and status. Select a container to read its last 300 log lines, including stderr. Refresh, start, and stop buttons run Docker CLI commands asynchronously. The header can cancel an active Docker operation. Errors from a missing CLI or daemon are shown in the window. Active CLI operations are aborted when the window state is dropped.

Validation: the focused container row parser test and tool window session identity test passed; `cargo check -p rc-app --locked --offline` passed with compact profiles. Docker is not installed in the validation environment, so live daemon behavior was not exercised.

The implementation is native DotCraft code and copies no external code or assets. Docker CLI behavior follows the official [container commands](https://docs.docker.com/reference/cli/docker/container/). Run/debug configurations and compose support remain for #122.
