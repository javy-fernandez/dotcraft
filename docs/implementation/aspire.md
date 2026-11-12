# Aspire AppHost workflow

DotCraft recognizes project-based Aspire AppHosts through `Aspire.AppHost.Sdk` or `IsAspireHost`. The AppHost appears as a run configuration. Starting it opens the Run and Aspire Resources windows; debugging it opens the Debug and Aspire Resources windows. The Resources window also has one-click Run and Debug actions for the selected AppHost.

While an AppHost is running, the Resources window refreshes its projects, containers, databases, state, health and endpoints every four seconds. Select a resource to view its last 200 console log lines, refreshed alongside the resource list. The AppHost's dashboard URL is extracted from process output and opened in the browser on request. It remains in memory because the URL can contain an authentication token.

Resource and log inspection uses the installed Aspire CLI's `describe --format Json` and `logs <resource> --tail 200 --timestamps` commands with an explicit `--apphost` path and non-interactive mode. CLI calls run on worker threads, are cancelled when the workspace changes, and time out after 15 seconds. Run and Debug use DotCraft's existing .NET project launch and netcoredbg paths. The CLI is required for resource and log inspection; the window reports a launch error if it is absent.

The implementation follows the [Aspire AppHost configuration](https://learn.microsoft.com/en-us/dotnet/aspire/app-host/configuration), [Aspire describe](https://aspire.dev/reference/cli/commands/aspire-describe/) and [Aspire logs](https://aspire.dev/reference/cli/commands/aspire-logs/) documentation. JSON fields were checked against Aspire's [shared resource model](https://source.dot.net/Aspire.Dashboard/src/Shared/Model/Serialization/ResourceJson.cs.html). No Aspire code or assets were copied.

Validation: compact offline `cargo check -p rc-app`, focused Aspire parser and dashboard tests, and locale JSON parsing. A live AppHost and Aspire CLI were unavailable in the validation environment, so a running distributed application could not be exercised here.
