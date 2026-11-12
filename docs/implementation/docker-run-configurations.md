# Docker and Compose run configurations (#122, partial)

DotCraft discovers Dockerfile, Dockerfile variants, Containerfile variants, and common Compose filenames up to four directory levels below a workspace. Generated `bin`, `obj`, `node_modules`, and `.git` directories are skipped. Discovery runs in a background task and adds the files to the normal run configuration list.

Running a Dockerfile builds an image tagged for that file, then starts a named container with its default command. Running a Compose file uses `docker compose -f <file> up --build`. Both stream stdout and stderr to the Run window. Stop cancels the CLI process and asks Docker to stop the corresponding container or Compose project. Commands are passed as argument arrays, without shell interpolation.

Validation: the discovery unit test passed for Dockerfile and Compose files and excludes generated directories. Compact `cargo check -p rc-app --locked --offline` passed for the application integration. Docker CLI is not installed in this environment, so live daemon behavior remains untested.

The implementation is native DotCraft code. Docker CLI command behavior follows the official [Docker run](https://docs.docker.com/reference/cli/docker/container/run/) and [Compose](https://docs.docker.com/reference/cli/docker/compose/) documentation. No external code or assets were copied.

## Container debugging blocker

The current .NET debug path only launches an application on the host or asks a host `netcoredbg` process to attach to a local PID. `DapClient` starts the adapter as a local child process over stdio, and `Debugger::attach` passes the selected PID directly as the DAP `processId`. A PID reported inside a container cannot be assumed to identify the same process on the host, especially with Docker Desktop or a remote daemon. DotCraft also has no container process picker, in-container debugger transport, or source-path mapping between PDB paths and workspace files.

A supported workflow needs to select the target container and .NET process, run or connect to a compatible debug adapter in that container, carry DAP messages to DotCraft, and map source paths. A possible host-PID attach on a local Linux daemon would need separate process-namespace and ptrace validation and would not cover Docker Desktop or remote daemons. This environment has no Docker CLI or daemon, so neither approach can be exercised against a real .NET container. No container-debug action is exposed by this PR, and #122 remains open.

The expected process-selection and attach workflow is described in [Microsoft's Docker attach documentation](https://learn.microsoft.com/en-us/visualstudio/debugger/attach-to-process-running-in-docker-container?view=visualstudio). DotCraft's local adapter is [netcoredbg](https://github.com/Samsung/netcoredbg).
