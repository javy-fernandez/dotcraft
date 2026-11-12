# Features to implement

This high-level backlog tracks Rider-inspired capabilities to implement or complete in DotCraft. The detailed status record is [FEATURES.md](../FEATURES.md). The reference is the JetBrains Rider 2026.2 documentation reviewed on November 6, 2026. Current implementation work uses the original `javy-fernandez/dotcraft` repository at `becfb0f`. This checklist covers feature families rather than every individual inspection, command, or third-party plugin.

Unchecked items require implementation or validation. They do not necessarily indicate missing functionality: DotCraft already declares support for several editor, Roslyn, build, test, debugger, NuGet, terminal, and Git capabilities in the [README](../README.md). The original added coverage, profiling, monitoring, local history, templates, HTTP environments, and NuGet package management after this checklist was first written. Audit existing behavior before opening implementation tasks, and check an item only after its intended scope works and has appropriate validation.

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
- [x] Draw branch and merge lanes in the local Git history and select a commit. Validated with a real local merge fixture on Linux; the view loads the first 500 commits.
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

## Workspace layout

- [x] Switch between normal, distraction-free, and Zen editor layouts while preserving the current tool-window arrangement. This is validated on Linux with the `Shop.sln` UI fixture; mode state is temporary.

## Implementation progress

See [combined validation evidence](implementation/validation.md) for commands and tested scope.
The original repository incorporated the earlier feature batch through upstream PRs
[108](https://github.com/javy-fernandez/dotcraft/pull/108) and
[109](https://github.com/javy-fernandez/dotcraft/pull/109). The fork's older feature PRs
remain historical review slices; their merge state in the fork does not describe the
original repository's implementation. Broad checklist items remain unchecked when
additional scope is pending.

| Slice | Delivery | Evidence |
| --- | --- | --- |
| Feature status and delivery index | [Original PR 151](https://github.com/javy-fernandez/dotcraft/pull/151) | [Detailed inventory](../FEATURES.md) |
| Solution-filter loading and SDK build context | Original merge `a0f31fb`; [fork PR 2](https://github.com/RafaelMendes1407/dotcraft/pull/2) | [Solution filters](implementation/solution-filters.md) |
| Filtered test-project selection | [Original PR 153](https://github.com/javy-fernandez/dotcraft/pull/153) | [Solution filters](implementation/solution-filters.md) |
| Live .NET build, run, test, and Roslyn checks | Original merge `a0f31fb` | [Live .NET validation](implementation/live-dotnet.md) |
| Debugger attachment | Original merge `a0f31fb` | [Attach debugger](implementation/attach-debugger.md) |
| EditorConfig indentation | Original merge `a0f31fb` | [EditorConfig](implementation/editorconfig.md) |
| Installed SDK selection | Original merge `a0f31fb` | [SDK selection](implementation/sdk-selection.md) |
| Responsive settings and compact menu | Original merge `a0f31fb` | [Settings](implementation/responsive-settings.md), [menu](implementation/scrollable-menu.md) |
| Read-only SQLite browsing | Original merge `a0f31fb` | [SQLite](implementation/sqlite-browser.md) |
| Terminal scrollback search | [Original PR 152](https://github.com/javy-fernandez/dotcraft/pull/152) | [Terminal search](implementation/terminal-search.md) |
| .NET stack-trace source links | [Original PR 154](https://github.com/javy-fernandez/dotcraft/pull/154) | [Stack-trace navigation](implementation/stack-trace-navigation.md) |
| File watcher rename and atomic-save handling (L-03, partial) | [Original PR 155](https://github.com/javy-fernandez/dotcraft/pull/155) | [PR evidence](https://github.com/javy-fernandez/dotcraft/pull/155/files) |
| Solution build configurations and platforms (B-19, partial) | [Original PR 156](https://github.com/javy-fernandez/dotcraft/pull/156) | [PR evidence](https://github.com/javy-fernandez/dotcraft/pull/156/files) |
| C# complete statement (C-17, partial) | [Original PR 157](https://github.com/javy-fernandez/dotcraft/pull/157) | [PR evidence](https://github.com/javy-fernandez/dotcraft/pull/157/files) |
| Git submodule status and cancellation (I-23, partial) | [Original PR 158](https://github.com/javy-fernandez/dotcraft/pull/158) | [PR evidence](https://github.com/javy-fernandez/dotcraft/pull/158/files) |
| Concurrent workspace session saves (L-02, partial) | [Original PR 159](https://github.com/javy-fernandez/dotcraft/pull/159) | [PR evidence](https://github.com/javy-fernandez/dotcraft/pull/159/files) |
| Git revision read safety (I-10, partial) | [Original PR 160](https://github.com/javy-fernandez/dotcraft/pull/160) | [PR evidence](https://github.com/javy-fernandez/dotcraft/pull/160/files) |
| Distraction-free and Zen modes (A-22) | [Original PR 161](https://github.com/javy-fernandez/dotcraft/pull/161) | [Focus modes](https://github.com/RafaelMendes1407/dotcraft/blob/feat/zen-and-distraction-free/docs/implementation/focus-modes.md) |
| EditorConfig numeric-range globs (C-18, partial) | [Original PR 162](https://github.com/javy-fernandez/dotcraft/pull/162) | [PR evidence](https://github.com/javy-fernandez/dotcraft/pull/162/files) |
| Long-line editor safeguard (C-16, partial) | [Original PR 163](https://github.com/javy-fernandez/dotcraft/pull/163) | [PR evidence](https://github.com/javy-fernandez/dotcraft/pull/163/files) |
| Blocking background runtime (L-04, partial) | [Original PR 164](https://github.com/javy-fernandez/dotcraft/pull/164) | [PR evidence](https://github.com/javy-fernandez/dotcraft/pull/164/files) |
| Git log branch and merge graph (I-14) | [Original PR 165](https://github.com/javy-fernandez/dotcraft/pull/165) | [Git log graph](https://github.com/RafaelMendes1407/dotcraft/blob/feat/git-log-graph-ui/docs/implementation/git-log-graph.md) |
| Local History concurrent revision safety (I-34, partial) | [Original PR 166](https://github.com/javy-fernandez/dotcraft/pull/166) | [PR evidence](https://github.com/javy-fernandez/dotcraft/pull/166/files) |

## Completion criteria

For each item, record the implemented scope, supported platforms and runtimes, required dependencies, user-facing documentation, and validation evidence. Split broad items into smaller implementation tasks as work starts. Existing partial implementations should be completed or validated before being marked done.
