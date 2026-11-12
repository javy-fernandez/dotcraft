# Docker and Compose run configurations (#122, partial)

DotCraft discovers Dockerfile, Dockerfile variants, Containerfile variants, and common Compose filenames up to four directory levels below a workspace. Generated `bin`, `obj`, `node_modules`, and `.git` directories are skipped. Discovery runs in a background task and adds the files to the normal run configuration list.

Running a Dockerfile builds an image tagged for that file, then starts a named container with its default command. Running a Compose file uses `docker compose -f <file> up --build`. Both stream stdout and stderr to the Run window. Stop cancels the CLI process and asks Docker to stop the corresponding container or Compose project. Commands are passed as argument arrays, without shell interpolation.

Validation: the discovery unit test passed for Dockerfile and Compose files and excludes generated directories. Compact `cargo check -p rc-app --locked --offline` passed for the application integration. Docker CLI is not installed in this environment, so live daemon behavior remains untested.

The implementation is native DotCraft code. Docker CLI command behavior follows the official [Docker run](https://docs.docker.com/reference/cli/docker/container/run/) and [Compose](https://docs.docker.com/reference/cli/docker/compose/) documentation. No external code or assets were copied. Container debugging is outside this slice and #122 remains open.
