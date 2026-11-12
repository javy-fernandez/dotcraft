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
- [x] Open a standalone executable .NET project folder and discover its run configuration. Validated with Shop.Cli on Linux; other project kinds and SDK execution still need coverage.
- [x] Open a .NET source folder and a multi-project solution as distinct workspaces. Validated with `Shop/src` and `Shop.sln` on Linux.

Reference: [Rider features](https://www.jetbrains.com/rider/features/).

## Intelligent editor

- [ ] Provide contextual completion and automatic namespace imports.
- [ ] Match delimiters and insert closing braces automatically.
- [x] Pair C# brackets during typing, respect comments and strings, and preserve undo. Validated through editor input events on a real Shop C# file; broader delimiter behavior remains in the unchecked item above.
- [x] Toggle selected C# line and block comments with Rider shortcuts and undo. Validated on a real Shop C# file.
- [x] Type brackets with mixed selected and empty C# carets without losing selected text. Validated through editor events and undo on a real Shop C# file.
- [x] Undo and redo C# edits while restoring caret, diagnostic and inlay positions. Validated through editor events on a real Shop C# file.
- [x] Indent the next C# line after `case` and `default` labels as one undoable edit. Validated through editor events on a real Shop C# file.
- [x] Fold and unfold C# scopes, including Fold All, without changing source text. Validated through editor events on a real Shop C# file.
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
- [x] Provide an integrated local terminal for development commands on Linux. `Shop.sln` workspace cwd, output, process exit and a second tab were validated; SDK commands and other platforms remain untested.

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
- [x] Clone and open a local Git repository containing `Shop.sln` on Linux. Remote authentication, large clones and cancellation remain untested.
- [x] Fetch, Pull and Push a `Shop.sln` repository through the Git UI against a local bare remote, including divergent Push rejection. Validated on Linux; hosted authentication remains untested.
- [ ] Organize changes into changelists and shelve work.
- [x] Shelve and restore local changes from the Git UI, including unsaved and untracked C# files. Validated with `Shop.sln` on Linux; changelists and conflict restoration remain in the unchecked item above.
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
- [x] Show the active .NET file, encoding, line ending and caret position in the status bar, with an accessible Go to Line control. Validated with `Shop.sln` on Linux.
- [x] Reopen a filtered recent C# file with the keyboard while keeping the selection valid. Validated with `Shop.sln` on Linux.
- [x] Return to a recent C# source location by line number with the keyboard. Validated with `Shop.sln` on Linux.
- [x] Activate and close C# editor tabs with accessible controls. Validated with `Shop.sln` on Linux.
- [x] Open a recent .NET solution from Welcome with an accessible full-path row. Validated with `Shop.sln` on Linux.
- [x] Split two C# editors side by side or vertically, then unsplit without closing either file. Validated with `Shop.sln` on Linux.
- [x] Show status notifications and expire information and errors at their respective intervals. Validated with `Shop.sln` on Linux.
- [x] Switch light and dark themes without closing a .NET solution. Validated with `Shop.sln` on Linux.
- [x] Hide, reopen and move the docked Explorer while preserving its session state. Validated with `Shop.sln` on Linux.
- [x] Use left and right tool-window stripes, including compact overflow scrolling. Validated with `Shop.sln` on Linux.
- [x] Apply and cancel an Editor setting without closing a .NET solution. Validated with `Shop.sln` on Linux.

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
| Filtered test-project selection (not merged) | [Closed original PR 153](https://github.com/javy-fernandez/dotcraft/pull/153) | [Historical evidence](implementation/solution-filters.md) |
| Live .NET build, run, test, and Roslyn checks | Original merge `a0f31fb` | [Live .NET validation](implementation/live-dotnet.md) |
| Debugger attachment | Original merge `a0f31fb` | [Attach debugger](implementation/attach-debugger.md) |
| EditorConfig indentation | Original merge `a0f31fb` | [EditorConfig](implementation/editorconfig.md) |
| Installed SDK selection | Original merge `a0f31fb` | [SDK selection](implementation/sdk-selection.md) |
| Responsive settings and compact menu | Original merge `a0f31fb` | [Settings](implementation/responsive-settings.md), [menu](implementation/scrollable-menu.md) |
| Read-only SQLite browsing | Original merge `a0f31fb` | [SQLite](implementation/sqlite-browser.md) |
| Terminal scrollback search (not merged) | [Closed original PR 152](https://github.com/javy-fernandez/dotcraft/pull/152) | [Historical evidence](implementation/terminal-search.md) |
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
| Accessible status bar metadata (A-14) | [Original PR 167](https://github.com/javy-fernandez/dotcraft/pull/167) | [Status bar evidence](https://github.com/RafaelMendes1407/dotcraft/blob/feat/status-bar-dotnet-acceptance/docs/implementation/status-bar-dotnet.md) |
| C# automatic brackets (C-06) | [Original PR 168](https://github.com/javy-fernandez/dotcraft/pull/168) | [PR evidence](https://github.com/javy-fernandez/dotcraft/pull/168/files) |
| Standalone project folder discovery (B-04) | [Original PR 169](https://github.com/javy-fernandez/dotcraft/pull/169) | [PR evidence](https://github.com/javy-fernandez/dotcraft/pull/169/files) |
| Filtered Recent Files selection (A-09) | [Original PR 170](https://github.com/javy-fernandez/dotcraft/pull/170) | [Recent Files evidence](https://github.com/RafaelMendes1407/dotcraft/blob/feat/recent-files-filter-selection/docs/implementation/recent-files-filter.md) |
| Recent Locations source navigation (A-10) | [Original PR 171](https://github.com/javy-fernandez/dotcraft/pull/171) | [Recent Locations evidence](https://github.com/RafaelMendes1407/dotcraft/blob/feat/recent-locations-navigation/docs/implementation/recent-locations-dotnet.md) |
| C# line and block comments (C-10) | [Original PR 172](https://github.com/javy-fernandez/dotcraft/pull/172) | [PR evidence](https://github.com/javy-fernandez/dotcraft/pull/172/files) |
| Integrated local terminal (L-01) | [Original PR 173](https://github.com/javy-fernandez/dotcraft/pull/173) | [PR evidence](https://github.com/javy-fernandez/dotcraft/pull/173/files) |
| Accessible C# editor tabs (A-05) | [Original PR 174](https://github.com/javy-fernandez/dotcraft/pull/174) | [Editor tabs evidence](https://github.com/RafaelMendes1407/dotcraft/blob/feat/accessible-editor-tabs/docs/implementation/editor-tabs-dotnet.md) |
| Workspace solution/folder symlink tree (B-01) | [Original PR 175](https://github.com/javy-fernandez/dotcraft/pull/175) | [PR evidence](https://github.com/javy-fernandez/dotcraft/pull/175/files) |
| Recent solution on Welcome (A-08) | [Original PR 176](https://github.com/javy-fernandez/dotcraft/pull/176) | [Welcome evidence](https://github.com/RafaelMendes1407/dotcraft/blob/feat/recent-projects-welcome/docs/implementation/recent-projects-shop.md) |
| Solution folders and loose items (B-05) | [Original PR 177](https://github.com/javy-fernandez/dotcraft/pull/177) | [PR evidence](https://github.com/javy-fernandez/dotcraft/pull/177/files) |
| Mixed selection and caret typing (C-07) | [Original PR 178](https://github.com/javy-fernandez/dotcraft/pull/178) | [PR evidence](https://github.com/javy-fernandez/dotcraft/pull/178/files) |
| Split C# editors (A-06) | [Original PR 179](https://github.com/javy-fernandez/dotcraft/pull/179) | [Split editor evidence](https://github.com/RafaelMendes1407/dotcraft/blob/feat/split-editor-dotnet-acceptance/docs/implementation/split-editor-shop.md) |
| Local Git clone and open (I-09) | [Original PR 180](https://github.com/javy-fernandez/dotcraft/pull/180) | [PR evidence](https://github.com/javy-fernandez/dotcraft/pull/180/files) |
| Launch profile URL classification (B-11, partial) | [Original PR 181](https://github.com/javy-fernandez/dotcraft/pull/181) | [PR evidence](https://github.com/javy-fernandez/dotcraft/pull/181/files) |
| Search Everywhere .NET file results (A-07, partial) | [Original PR 182](https://github.com/javy-fernandez/dotcraft/pull/182) | [File search evidence](https://github.com/RafaelMendes1407/dotcraft/blob/feat/search-everywhere-dotnet-files/docs/implementation/search-everywhere-dotnet-files.md) |
| C# undo and redo (C-12) | [Original PR 183](https://github.com/javy-fernandez/dotcraft/pull/183) | [PR evidence](https://github.com/javy-fernandez/dotcraft/pull/183/files) |
| ASP.NET endpoint project boundaries (J-09, partial) | [Original PR 184](https://github.com/javy-fernandez/dotcraft/pull/184) | [PR evidence](https://github.com/javy-fernandez/dotcraft/pull/184/files) |
| Standalone Shop.Cli workspace UI (B-04) | [Original PR 185](https://github.com/javy-fernandez/dotcraft/pull/185) | [App evidence](https://github.com/RafaelMendes1407/dotcraft/blob/feat/standalone-project-ui-acceptance/docs/implementation/standalone-project-ui.md) |
| Local Shelves push/pop (I-11) | [Original PR 186](https://github.com/javy-fernandez/dotcraft/pull/186) | [PR evidence](https://github.com/javy-fernandez/dotcraft/pull/186/files) |
| Project template solution preflight (B-06, partial) | [Original PR 187](https://github.com/javy-fernandez/dotcraft/pull/187) | [PR evidence](https://github.com/javy-fernandez/dotcraft/pull/187/files) |
| Folder and solution app opening (B-01) | [Original PR 188](https://github.com/javy-fernandez/dotcraft/pull/188) | [App evidence](https://github.com/RafaelMendes1407/dotcraft/blob/feat/workspace-open-ui-acceptance/docs/implementation/workspace-open-shop.md) |
| C# case-label indentation (C-11) | [Original PR 189](https://github.com/javy-fernandez/dotcraft/pull/189) | [PR evidence](https://github.com/javy-fernandez/dotcraft/pull/189/files) |
| Status notification expiry (A-15) | [Original PR 190](https://github.com/javy-fernandez/dotcraft/pull/190) | [Notification evidence](https://github.com/RafaelMendes1407/dotcraft/blob/feat/notifications-dotnet-acceptance/docs/implementation/notifications-shop.md) |
| Light and dark theme switching (A-02) | [Original PR 191](https://github.com/javy-fernandez/dotcraft/pull/191) | [Theme evidence](https://github.com/RafaelMendes1407/dotcraft/blob/feat/theme-dotnet-acceptance/docs/implementation/theme-shop-workspace.md) |
| Nested CPM PackageVersion updates (B-23, partial) | [Original PR 192](https://github.com/javy-fernandez/dotcraft/pull/192) | [PR evidence](https://github.com/javy-fernandez/dotcraft/pull/192/files) |
| Docked Explorer workspace behavior (A-04) | [Original PR 193](https://github.com/javy-fernandez/dotcraft/pull/193) | [Dock evidence](https://github.com/RafaelMendes1407/dotcraft/blob/feat/dock-dotnet-acceptance/docs/implementation/docked-explorer-shop.md) |
| Local Git Fetch/Pull/Push (I-15) | [Original PR 194](https://github.com/javy-fernandez/dotcraft/pull/194) | [PR evidence](https://github.com/javy-fernandez/dotcraft/pull/194/files) |
| HTTP parent environment resolution (J-06, partial) | [Original PR 195](https://github.com/javy-fernandez/dotcraft/pull/195) | [PR evidence](https://github.com/javy-fernandez/dotcraft/pull/195/files) |
| C# code folding (C-08) | [Original PR 196](https://github.com/javy-fernandez/dotcraft/pull/196) | [PR evidence](https://github.com/javy-fernandez/dotcraft/pull/196/files) |
| Tool-window stripes (A-03) | [Original PR 197](https://github.com/javy-fernandez/dotcraft/pull/197) | [Stripe evidence](https://github.com/RafaelMendes1407/dotcraft/blob/feat/tool-stripes-dotnet-acceptance/docs/implementation/tool-stripes-shop.md) |
| Settings apply and cancel (A-11) | [Original PR 198](https://github.com/javy-fernandez/dotcraft/pull/198) | [Settings evidence](https://github.com/RafaelMendes1407/dotcraft/blob/feat/settings-dotnet-acceptance/docs/implementation/settings-shop-workspace.md) |
| Rider and Visual Studio Go to File keymaps (A-12) | [Original PR 199](https://github.com/javy-fernandez/dotcraft/pull/199) | [Keymap evidence](https://github.com/RafaelMendes1407/dotcraft/blob/feat/keymaps-dotnet-acceptance/docs/implementation/keymaps-shop.md) |
| Target framework property editing (B-18, partial) | [Original PR 200](https://github.com/javy-fernandez/dotcraft/pull/200) | [PR evidence](https://github.com/javy-fernandez/dotcraft/pull/200/files) |
| Shop Git Amend and saved editor changes (I-12) | [Original PR 201](https://github.com/javy-fernandez/dotcraft/pull/201) | [PR evidence](https://github.com/javy-fernandez/dotcraft/pull/201/files) |
| C# inlay hint padding (C-09, partial) | [Original PR 202](https://github.com/javy-fernandez/dotcraft/pull/202) | [PR evidence](https://github.com/javy-fernandez/dotcraft/pull/202/files) |
| Live locale switch in Shop (A-13, partial) | [Original PR 203](https://github.com/javy-fernandez/dotcraft/pull/203) | [Locale evidence](https://github.com/RafaelMendes1407/dotcraft/blob/feat/localization-dotnet-acceptance/docs/implementation/localization-shop.md) |
| Project reference editing (B-17, partial) | [Original PR 204](https://github.com/javy-fernandez/dotcraft/pull/204) | [PR evidence](https://github.com/javy-fernandez/dotcraft/pull/204/files) |
| Safe C# reload conflict choices (A-20) | [Original PR 205](https://github.com/javy-fernandez/dotcraft/pull/205) | [Reload evidence](https://github.com/RafaelMendes1407/dotcraft/blob/feat/safe-reload-shop-conflict/docs/implementation/safe-reload-shop.md) |
| Native Shop workspace lifecycle (A-01) | [Original PR 206](https://github.com/javy-fernandez/dotcraft/pull/206) | [Workspace evidence](https://github.com/RafaelMendes1407/dotcraft/blob/feat/native-workspace-shop-acceptance/docs/implementation/native-workspace-shop.md) |
| REST requests from Shop.Api endpoints (J-23, partial) | [Original PR 207](https://github.com/javy-fernandez/dotcraft/pull/207) | [PR evidence](https://github.com/javy-fernandez/dotcraft/pull/207/files) |
| Git Cherry-pick and conflict abort (I-17) | [Original PR 208](https://github.com/javy-fernandez/dotcraft/pull/208) | [PR evidence](https://github.com/javy-fernandez/dotcraft/pull/208/files) |
| Solution filter editing backend (B-15, partial) | [Original PR 209](https://github.com/javy-fernandez/dotcraft/pull/209) | [PR evidence](https://github.com/javy-fernandez/dotcraft/pull/209/files) |
| C# Surround With read-only safety (C-25, partial) | [Original PR 210](https://github.com/javy-fernandez/dotcraft/pull/210) | [PR evidence](https://github.com/javy-fernandez/dotcraft/pull/210/files) |
| Presentation mode with Shop (A-23) | [Original PR 211](https://github.com/javy-fernandez/dotcraft/pull/211) | [Presentation evidence](https://github.com/RafaelMendes1407/dotcraft/blob/feat/presentation-mode-shop/docs/implementation/presentation-mode-shop.md) |

## Completion criteria

For each item, record the implemented scope, supported platforms and runtimes, required dependencies, user-facing documentation, and validation evidence. Split broad items into smaller implementation tasks as work starts. Existing partial implementations should be completed or validated before being marked done.
