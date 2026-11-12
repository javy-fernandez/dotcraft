# Read-only remote Explorer

**File › Open Remote Folder…** opens an SSH or WSL connection to a `dotcraft-remote-agent` installed on the target. Enter an SSH host (using its normal SSH configuration) or WSL distribution, the agent executable path, and an absolute POSIX folder path. DotCraft connects off the UI thread, lists the root, and loads child directories only when expanded. Selecting a file reads it through the agent and displays a read-only UTF-8 preview in the Explorer. The protocol limits transfers to 1 MiB; the preview renders the first 64 KiB to keep the UI responsive. Closing the project or opening a local project stops the remote worker.

Remote paths remain POSIX strings on the host, including Windows hosts. This avoids treating `/home/user/project` as a Windows path while browsing WSL. Each remote request converts the string to a path only inside the agent, where the remote operating system owns path semantics.

The explorer is a deliberately read-only workspace slice. Remote files are not yet editable tabs, and DotCraft does not route file watching, .NET SDK discovery, builds, Roslyn, debugging, terminals, or searches to the agent. Those features stay unavailable until they have remote backends; opening a remote folder closes the active local project so local tools are not mistakenly applied to remote paths. Agent installation and a reachable SSH/WSL target are prerequisites.

Validation includes a live local-agent binary round trip for handshake/list/read, a POSIX path test, and an application compile. No SSH host or WSL distribution was available for an end-to-end remote smoke test. No external code or assets were copied. This slice builds on the transport in PR #309 and keeps issue #141 open.
