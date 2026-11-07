# Features to implement

This high-level backlog tracks Rider-inspired capabilities to implement or complete in DotCraft. The detailed status record is [FEATURES.md](../FEATURES.md). The reference is the JetBrains Rider 2026.2 documentation reviewed on November 6, 2026. It covers feature families rather than every individual inspection, command, or third-party plugin.

Unchecked items require implementation or validation. They do not necessarily indicate missing functionality: DotCraft already declares support for several editor, Roslyn, build, test, debugger, NuGet, terminal, and Git capabilities in the [README](../README.md). Audit existing behavior before opening implementation tasks, and check an item only after its intended scope works and has appropriate validation.

## Platforms and languages

- [ ] Support development workflows on Windows, macOS, and Linux.
- [ ] Support C#, F#, and VB.NET with language intelligence.
- [ ] Support Razor, ASP.NET markup, XAML, and XML.
- [ ] Support JavaScript, TypeScript, HTML, CSS, SCSS, and LESS.
- [ ] Support JSON and SQL editing.
- [ ] Open and manage .NET solutions, projects, libraries, services, desktop applications, and web applications.

Reference: [Rider features](https://www.jetbrains.com/rider/features/).

## Intelligent editor

- [ ] Provide contextual completion and automatic namespace imports.
- [ ] Match delimiters and insert closing braces automatically.
- [ ] Provide live templates and postfix templates.
- [ ] Support multiple selections and caret editing.
- [ ] Generate common code constructs and rearrange code.
- [ ] Expose context actions and quick fixes from the editor.
- [ ] Show inheritance navigation markers and framework navigation links.

Reference: [Rider features](https://www.jetbrains.com/rider/features/).

## Navigation and search

- [ ] Search files, types, members, settings, and actions through Search Everywhere.
- [ ] Find symbol usages across projects and supported languages.
- [ ] Navigate to declarations, implementations, base types, derived types, and extension methods.
- [ ] Find relevant symbol references in string literals.
- [ ] Browse file structure and navigate recent files and locations.

References: [Navigation and search](https://www.jetbrains.com/help/rider/Navigation_and_Search__Index.html), [Rider features](https://www.jetbrains.com/rider/features/).

## Library exploration

- [ ] Browse assemblies and decompile library types into readable C#.
- [ ] Navigate and find usages in decompiled code.
- [ ] Retrieve original library source when available from source servers.

Reference: [Rider features](https://www.jetbrains.com/rider/features/).

## Code analysis

- [ ] Run inspections while editing and report issues by severity.
- [ ] Inspect a selected project or the entire solution.
- [ ] Analyze solution-wide errors, including files outside the current editor.
- [ ] Configure inspection severity and suppression behavior.
- [ ] Apply quick fixes individually and in bulk where supported.
- [ ] Support annotations that influence code analysis.
- [ ] Explore call chains and the origins and destinations of values.

Reference: [Code analysis](https://www.jetbrains.com/help/rider/Code_Analysis__Index.html). Rider documents more than 2,500 inspections; implementing equivalent analysis is a separate scope from providing the inspection interface.

## Refactoring

- [ ] Rename symbols across the solution.
- [ ] Move, copy, and safely delete types and other symbols.
- [ ] Extract methods, interfaces, and classes.
- [ ] Introduce variables, fields, and parameters.
- [ ] Inline variables, methods, and other supported constructs.
- [ ] Change signatures and update affected references.
- [ ] Offer applicable refactorings through a contextual menu.
- [ ] Preview and detect conflicts before applying changes.
- [ ] Undo a refactoring across files as one operation.

References: [Refactorings](https://www.jetbrains.com/help/rider/Refactorings__Index.html), [Rider features](https://www.jetbrains.com/rider/features/).

## Build and execution

- [ ] Build solutions and projects and navigate build diagnostics.
- [ ] Create and manage multiple run and debug configurations.
- [ ] Run executable projects and application launch profiles.
- [ ] Manage NuGet dependencies.
- [ ] Provide an integrated terminal for development commands.

References: [First steps with Rider](https://www.jetbrains.com/help/rider/First_Steps.html), [existing DotCraft capabilities](../README.md).

## Debugging

Checked debugger workflows have live evidence on .NET 8 Linux; broader runtime/platform support remains pending. See [debugger validation](implementation/attach-debugger.md).

- [x] Launch applications under the debugger and attach to existing processes.
- [x] Manage line breakpoints and conditional breakpoints.
- [ ] Step into, over, and out of code and run to the cursor. Step Over and Run to Cursor have live evidence; Step Into and Step Out still need explicit validation.
- [ ] Inspect variables, watches, threads, and stack frames. Variables, watches, and selected frames have live evidence; explicit thread-list validation remains pending.
- [x] Evaluate expressions while execution is paused.
- [ ] Navigate from runtime stack traces to source.
- [ ] Define runtime and platform coverage for .NET, .NET Framework, and Mono workflows.

Reference: [Rider features](https://www.jetbrains.com/rider/features/). Runtime support depends on the debugger backend and host platform.

## Testing and coverage

- [ ] Discover, run, and debug NUnit, xUnit, and MSTest tests.
- [ ] Launch tests from editor gutter actions.
- [ ] Group tests and manage independent test sessions.
- [ ] Display results, output, and navigable failure stack traces.
- [ ] Collect coverage and display line coverage in the editor.
- [ ] Browse coverage results by project, assembly, type, and member as appropriate.
- [ ] Automatically rerun new and affected tests after configured triggers.
- [ ] Support continuous testing modes with optional coverage collection.

References: [Rider features](https://www.jetbrains.com/rider/features/), [Coverage analysis](https://www.jetbrains.com/help/rider/Analyzing_Coverage_NET.html), [Coverage results](https://www.jetbrains.com/help/rider/Working_with_Coverage_Results.html), [Continuous testing](https://www.jetbrains.com/help/rider/Work_with_Continuous_Testing.html).

Rider uses dotCover for its coverage integration. Select a suitable DotCraft coverage backend and define its licensing and platform requirements before implementation.

## Version control

- [ ] Review local changes, repository history, and incoming changes.
- [ ] Highlight changes in the editor and provide blame annotations.
- [ ] Stage changes, commit, push, and manage branches.
- [ ] Organize changes into changelists and shelve work.
- [ ] Compare file revisions and resolve merge conflicts visually.
- [ ] Define optional integrations for Subversion, Mercurial, Perforce, and TFS.

References: [Rider features](https://www.jetbrains.com/rider/features/), [existing DotCraft Git support](../README.md).

## Web development

- [ ] Provide Node.js workflows and React, Angular, and Vue assistance.
- [ ] Integrate package manager workflows such as npm and Yarn.
- [ ] Provide web language analysis, completion, navigation, and refactoring.
- [ ] Run and debug supported frontend applications and tests.
- [x] Create, edit, and execute HTTP requests inside the editor.

References: [Rider features](https://www.jetbrains.com/rider/features/), [HTTP Client](https://www.jetbrains.com/help/rider/Http_client_in__product__code_editor.html).

## Databases and Entity Framework

- [ ] Connect to databases and manage database sessions.
- [ ] Edit and execute SQL with completion, formatting, analysis, and navigation.
- [ ] Browse and edit schemas and table data.
- [ ] Visualize database schemas and relationships.
- [ ] Provide an Entity Framework Core interface for supported CLI operations.

References: [Rider features](https://www.jetbrains.com/rider/features/), [JetBrains EF Core plugin](https://github.com/JetBrains/rider-efcore).

## Monitoring and profiling

- [ ] Monitor CPU usage, memory, garbage collection, counters, and metrics.
- [ ] Collect and inspect runtime stack traces.
- [ ] Capture and analyze performance snapshots.
- [ ] Capture and analyze memory snapshots and allocations.
- [ ] Navigate performance findings back to application code.

References: [Monitoring](https://www.jetbrains.com/help/rider/Program-Monitoring.html), [Profiling applications](https://www.jetbrains.com/help/rider/Profiling_Applications.html).

Rider integrates dotTrace and dotMemory. Some advanced monitoring features require Windows and a dotUltimate license. Define DotCraft backends and supported platforms independently.

## Containers and remote development

- [ ] Build images and run Docker containers.
- [ ] Manage Docker Compose applications and container registries.
- [ ] Provide a services view for application configurations, databases, Docker, and optional Kubernetes connections.
- [ ] Support development with an IDE backend on a remote SSH host.
- [ ] Support local and remote Dev Container workflows within a defined platform matrix.

References: [Docker](https://www.jetbrains.com/help/rider/docker.html), [Services window](https://www.jetbrains.com/help/rider/Services_Tool_Window.html), [Remote development](https://www.jetbrains.com/help/rider/Remote_development-a.html), [Remote Dev Containers](https://www.jetbrains.com/help/rider/Start-Dev-Container-for-a-remote-project.html).

## Game development

- [ ] Integrate Unity development workflows and engine-aware C# assistance.
- [ ] Navigate Unity asset references, scenes, and prefabs.
- [ ] Run Unity Edit Mode and Play Mode tests.
- [ ] Open Unreal Engine projects and provide engine-aware C++ assistance.
- [ ] Complete and validate Unreal reflection macros and metadata.
- [ ] Navigate Unreal Blueprint references and generated code.
- [ ] Integrate Unreal debugging, Live Coding, and Hot Reload workflows.
- [ ] Run Unreal automation tests and supported Google Test and Boost.Test suites.
- [ ] Integrate Godot editor workflows and C# development.
- [ ] Edit and navigate ShaderLab and HLSL shader code.

References: [Rider for game development](https://www.jetbrains.com/rider-gamedev/), [Godot support](https://www.jetbrains.com/help/rider/Godot.html).

## AI assistance

- [ ] Provide project-aware AI assistance with explicit context controls.
- [ ] Support an agent that plans tasks and proposes or applies changes across files.
- [ ] Allow agent-driven command and test execution with configurable approval behavior.
- [ ] Review and roll back agent changes.
- [ ] Configure model providers and optional external tools through MCP.
- [ ] Respect project instructions and file exclusion rules.

Reference: [Junie](https://www.jetbrains.com/help/ai-assistant/junie-agent.html). Rider integrations are reference behavior; DotCraft must define its own providers, credentials, and usage requirements.

## Extensibility

- [ ] Define a plugin API and supported extension points.
- [ ] Install, enable, disable, and update extensions.
- [ ] Allow extensions to contribute language support and tool integrations.
- [ ] Suggest relevant extensions for unsupported files or workflows.

Reference: [Rider plugins](https://www.jetbrains.com/rider/features/). IntelliJ and ReSharper plugins are reference ecosystems; compatibility requires a separate technical assessment.

## Implementation progress

See [combined validation evidence](implementation/validation.md) for commands and tested scope.
Use the [integration guide](implementation/integration.md) to test the assembled branch and review feature dependencies.

Each pull request below has a separate implementation note with validation evidence. Open pull requests are awaiting review and are not merged into the default branch. Broad checklist items remain unchecked when additional scope is pending.

| Slice | Delivery | Evidence |
| --- | --- | --- |
| Reachable tool buttons in short windows | [PR 7](https://github.com/RafaelMendes1407/dotcraft/pull/7) | [Tool window overflow](implementation/tool-window-overflow.md) |
| Deleted-side Git conflict resolution | [PR 8](https://github.com/RafaelMendes1407/dotcraft/pull/8) | [Git deleted conflicts](implementation/git-deleted-conflicts.md) |
| Native HTML, CSS, JavaScript, TypeScript, and TSX syntax | [PR 9](https://github.com/RafaelMendes1407/dotcraft/pull/9) | [Web syntax](implementation/web-syntax.md) |
| Rejection of unsupported workspace resource edits | [PR 10](https://github.com/RafaelMendes1407/dotcraft/pull/10) | [Workspace edit safety](implementation/workspace-edit-safety.md) |
| Git stash management UI | [PR 11](https://github.com/RafaelMendes1407/dotcraft/pull/11) | [Git stashes](implementation/git-stashes.md) |
| Search destinations for existing settings pages | [PR 12](https://github.com/RafaelMendes1407/dotcraft/pull/12) | [Settings search](implementation/settings-search.md) |
| Basic in-editor HTTP request execution | [PR 13](https://github.com/RafaelMendes1407/dotcraft/pull/13) | [HTTP Client](implementation/http-client.md) |
| Attach to existing .NET processes | [PR 14](https://github.com/RafaelMendes1407/dotcraft/pull/14) | [Attach debugger](implementation/attach-debugger.md) |
| Inline variables in HTTP request files | [PR 15](https://github.com/RafaelMendes1407/dotcraft/pull/15), based on PR 13 | [HTTP variables](implementation/http-variables.md) |
| Responsive Settings layout and extended regression coverage | [PR 4](https://github.com/RafaelMendes1407/dotcraft/pull/4) | [Responsive Settings](implementation/responsive-settings.md) |
| Solution filters, source workspace lifecycle, and SDK build context | [PR 2](https://github.com/RafaelMendes1407/dotcraft/pull/2) | [Solution filters](implementation/solution-filters.md) |
| EditorConfig indentation and compatibility fixes | [PR 3](https://github.com/RafaelMendes1407/dotcraft/pull/3) | [EditorConfig](implementation/editorconfig.md) |
| Installed SDK selection through global.json | [PR 5](https://github.com/RafaelMendes1407/dotcraft/pull/5) | [SDK selection](implementation/sdk-selection.md) |
| Read-only SQLite browser and SQL console | [PR 16](https://github.com/RafaelMendes1407/dotcraft/pull/16), based on PR 6 | [SQLite](implementation/sqlite-browser.md) |
| Scrollable main menu and compact category navigation | [PR 17](https://github.com/RafaelMendes1407/dotcraft/pull/17), based on PR 14 | [Main menu](implementation/scrollable-menu.md) |

## Completion criteria

For each item, record the implemented scope, supported platforms and runtimes, required dependencies, user-facing documentation, and validation evidence. Split broad items into smaller implementation tasks as work starts. Existing partial implementations should be completed or validated before being marked done.
