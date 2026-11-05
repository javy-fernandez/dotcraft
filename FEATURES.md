# DotCraft feature inventory and implementation plan

Status of record for Rider-inspired development. Written in English; baseline reviewed
on 2026-11-05 against public Rider 2026.2 documentation and the current DotCraft source.
This is a broad capability inventory, not a claim of exhaustive action-by-action parity:
individual inspections, context actions, shortcuts, plugin features and platform variants
require their own follow-up breakdowns. No Rider source code or proprietary assets are reused.

## Status rules

| Status | Meaning |
| --- | --- |
| ✅ | Verified in DotCraft on a real .NET project, with evidence recorded here. |
| 🚧 | Partial implementation; remaining acceptance work is recorded below. |
| 🟢 | Existing DotCraft implementation located in source; not revalidated in this audit. |
| 🔁 | Candidate for reuse from CrabCraft; requires .NET adaptation and verification. |
| ⬜ | Backlog; no readiness claim. |

Update this file with each feature change and run `python3 scripts/features_summary.py`.
A source implementation or passing parser test alone does not establish Rider parity.

<!-- summary:start -->
| Status | Items |
| --- | --- |
| ✅ Verified | 0 |
| 🚧 In progress | 1 |
| 🟢 Existing | 104 |
| 🔁 Reuse candidate | 34 |
| ⬜ Backlog | 234 |
| 🚫 Out of scope | 0 |
| **Total** | **373** |
<!-- summary:end -->

## Sources and scope

- [Rider capabilities](https://www.jetbrains.com/rider/features/): editor, project types, navigation, VCS, web and databases.
- [Rider refactorings](https://www.jetbrains.com/help/rider/Refactorings__Index.html): transformations and conflict handling.
- [Rider debugging](https://www.jetbrains.com/help/rider/Debugging_Code.html): debug workflows and platform limitations.
- [Rider remote development](https://www.jetbrains.com/help/rider/Remote_development_overview.html): remote backend workflows.
- [Rider profiling](https://www.jetbrains.com/help/rider/Performance_Profiling.html) and [monitoring](https://www.jetbrains.com/help/rider/Program-Monitoring.html).
- [Rider 2026.2 release](https://blog.jetbrains.com/dotnet/2026/07/22/rider-2026-2-release/): current baseline additions.
- [Microsoft solution filters](https://learn.microsoft.com/en-us/visualstudio/msbuild/solution-filters): `.slnf` format and path semantics.

Some capabilities depend on OS, runtime, external tools, plugins or subscriptions in Rider.
An inventory entry describes an implementation target, not universal Rider availability.

## Delivery order and reuse

1. Audit the inherited .NET baseline and fix solution loading (B-14). Validate build,
   run, tests, diagnostics and debugging using `tests/fixtures/Shop` and `TenProjects`.
2. Port shared UI improvements from the sibling CrabCraft repository: responsive settings,
   bounded menus, docking previews and accessible navigation. Adapt app types, actions,
   settings and translations instead of copying Rust/Cargo workflows.
3. Port shared editor, terminal, Git, HTTP and database components in reviewable slices.
   Preserve DotCraft notices and check source/asset licenses for every imported component.
4. Expand .NET intelligence and refactoring, then coverage, profiling and framework tools.
5. Implement remote development and extensibility after local lifecycle and trust controls.

Reuse must preserve native Rust/egui, background work and cancellation, localization,
.NET project semantics, and existing licenses. A 🔁 item is only a candidate: no source
has been imported merely because the sibling application lists it as complete.

## Initial implementation evidence

B-14: `Solution::load` now reads JSON solution filters, resolves the parent solution
relative to the filter and selected projects relative to the parent, and retains the
selected project tree. It preserves the filter path for existing build/test command
consumers. Invalid JSON, missing fields, unknown projects, missing source solutions,
and nested filters return errors. No new graphical control is required: Open already
accepts `.slnf`, and the existing explorer consumes the resulting solution tree.

Verification: `cargo test -p rc-dotnet -p rc-core --lib`. The real `TenProjects`
fixture is loaded through a filter and both selected project files are parsed.
Parser regressions cover classic and XML parent solutions, Windows separators, filters
in a subdirectory, empty selections, duplicate selections and invalid input.
Remaining: real SDK build/test execution, graphical explorer verification, Roslyn behavior
with filters, interactive editing, and dependency-loading behavior. B-14 remains 🚧.
The existing workspace root stays the filter directory; filters stored outside the
solution root need a separate watcher/search-root audit.


## A. Desktop interface and workflow

Source / reuse boundary: `crates/rc-app/src/ui; crates/rc-kit`.

| ID | Capability | Status | Next acceptance step |
| --- | --- | --- | --- |
| A-01 | Native desktop workspace | 🟢 | Exercise on a real .NET fixture; record evidence. |
| A-02 | Light and dark themes | 🟢 | Exercise on a real .NET fixture; record evidence. |
| A-03 | Tool window stripes | 🟢 | Exercise on a real .NET fixture; record evidence. |
| A-04 | Docked tool windows | 🟢 | Exercise on a real .NET fixture; record evidence. |
| A-05 | Editor tabs | 🟢 | Exercise on a real .NET fixture; record evidence. |
| A-06 | Split editors | 🟢 | Exercise on a real .NET fixture; record evidence. |
| A-07 | Search Everywhere | 🟢 | Exercise on a real .NET fixture; record evidence. |
| A-08 | Recent projects | 🟢 | Exercise on a real .NET fixture; record evidence. |
| A-09 | Recent files | 🟢 | Exercise on a real .NET fixture; record evidence. |
| A-10 | Recent locations | 🟢 | Exercise on a real .NET fixture; record evidence. |
| A-11 | Settings dialog | 🟢 | Exercise on a real .NET fixture; record evidence. |
| A-12 | Rider and Visual Studio keymaps | 🟢 | Exercise on a real .NET fixture; record evidence. |
| A-13 | Localization | 🟢 | Exercise on a real .NET fixture; record evidence. |
| A-14 | Status bar | 🟢 | Exercise on a real .NET fixture; record evidence. |
| A-15 | Notifications | 🟢 | Exercise on a real .NET fixture; record evidence. |
| A-16 | Responsive settings layout | 🔁 | Inspect sibling implementation, adapt and verify in .NET. |
| A-17 | Scrollable main menu | 🔁 | Inspect sibling implementation, adapt and verify in .NET. |
| A-18 | Tab docking previews | 🔁 | Inspect sibling implementation, adapt and verify in .NET. |
| A-19 | Background document saving | 🔁 | Inspect sibling implementation, adapt and verify in .NET. |
| A-20 | Safe reload from disk | 🔁 | Inspect sibling implementation, adapt and verify in .NET. |
| A-21 | Accessible keyboard navigation | 🔁 | Inspect sibling implementation, adapt and verify in .NET. |
| A-22 | Distraction-free and Zen modes | ⬜ | Implement with integration and UI acceptance evidence. |
| A-23 | Presentation mode | ⬜ | Implement with integration and UI acceptance evidence. |
| A-24 | Customizable toolbar | ⬜ | Implement with integration and UI acceptance evidence. |
| A-25 | Settings synchronization | ⬜ | Implement with integration and UI acceptance evidence. |
| A-26 | IDE settings import | ⬜ | Implement with integration and UI acceptance evidence. |
| A-27 | Command-line launcher | ⬜ | Implement with integration and UI acceptance evidence. |
| A-28 | Project trust controls | ⬜ | Implement with integration and UI acceptance evidence. |
| A-29 | Plugin manager | ⬜ | Implement with integration and UI acceptance evidence. |
| A-30 | IDE updates and rollback | ⬜ | Implement with integration and UI acceptance evidence. |

## B. Solutions, projects, SDKs and dependencies

Source / reuse boundary: `crates/rc-dotnet; crates/rc-core/src/workspace.rs`.

| ID | Capability | Status | Next acceptance step |
| --- | --- | --- | --- |
| B-01 | Open solutions and folders | 🟢 | Exercise on a real .NET fixture; record evidence. |
| B-02 | Classic .sln solutions | 🟢 | Exercise on a real .NET fixture; record evidence. |
| B-03 | XML .slnx solutions | 🟢 | Exercise on a real .NET fixture; record evidence. |
| B-04 | Standalone .NET projects | 🟢 | Exercise on a real .NET fixture; record evidence. |
| B-05 | Solution folders and items | 🟢 | Exercise on a real .NET fixture; record evidence. |
| B-06 | Project templates | 🟢 | Exercise on a real .NET fixture; record evidence. |
| B-07 | File and class templates | 🟢 | Exercise on a real .NET fixture; record evidence. |
| B-08 | .NET SDK discovery | 🟢 | Exercise on a real .NET fixture; record evidence. |
| B-09 | Runtime discovery | 🟢 | Exercise on a real .NET fixture; record evidence. |
| B-10 | MSBuild property evaluation | 🟢 | Exercise on a real .NET fixture; record evidence. |
| B-11 | Launch settings profiles | 🟢 | Exercise on a real .NET fixture; record evidence. |
| B-12 | NuGet search | 🟢 | Exercise on a real .NET fixture; record evidence. |
| B-13 | NuGet installation and removal | 🟢 | Exercise on a real .NET fixture; record evidence. |
| B-14 | JSON .slnf solution filters | 🚧 | Parser implemented; verify SDK, UI and language-server behavior. |
| B-15 | Interactive filter creation and editing | ⬜ | Implement with integration and UI acceptance evidence. |
| B-16 | Unload and reload individual projects | ⬜ | Implement with integration and UI acceptance evidence. |
| B-17 | Project references editor | ⬜ | Implement with integration and UI acceptance evidence. |
| B-18 | Target frameworks editor | ⬜ | Implement with integration and UI acceptance evidence. |
| B-19 | Build configurations and platforms | ⬜ | Implement with integration and UI acceptance evidence. |
| B-20 | global.json SDK management | ⬜ | Implement with integration and UI acceptance evidence. |
| B-21 | .NET workload management | ⬜ | Implement with integration and UI acceptance evidence. |
| B-22 | NuGet dependency graph | ⬜ | Implement with integration and UI acceptance evidence. |
| B-23 | Central package management | ⬜ | Implement with integration and UI acceptance evidence. |
| B-24 | NuGet feed credentials | ⬜ | Implement with integration and UI acceptance evidence. |
| B-25 | Vulnerable and outdated packages | ⬜ | Implement with integration and UI acceptance evidence. |
| B-26 | NuGet Package Manager Console | ⬜ | Implement with integration and UI acceptance evidence. |
| B-27 | Project migration and upgrade | ⬜ | Implement with integration and UI acceptance evidence. |
| B-28 | Solution dependency diagrams | ⬜ | Implement with integration and UI acceptance evidence. |
| B-29 | Source generators inspection | ⬜ | Implement with integration and UI acceptance evidence. |

## C. Editor and language intelligence

Source / reuse boundary: `crates/rc-editor; crates/rc-text; crates/rc-syntax; crates/rc-lsp`.

| ID | Capability | Status | Next acceptance step |
| --- | --- | --- | --- |
| C-01 | C# syntax highlighting | 🟢 | Exercise on a real .NET fixture; record evidence. |
| C-02 | Roslyn semantic highlighting | 🟢 | Exercise on a real .NET fixture; record evidence. |
| C-03 | Code completion | 🟢 | Exercise on a real .NET fixture; record evidence. |
| C-04 | Signature help | 🟢 | Exercise on a real .NET fixture; record evidence. |
| C-05 | Quick documentation | 🟢 | Exercise on a real .NET fixture; record evidence. |
| C-06 | Automatic brackets | 🟢 | Exercise on a real .NET fixture; record evidence. |
| C-07 | Multiple carets | 🟢 | Exercise on a real .NET fixture; record evidence. |
| C-08 | Code folding | 🟢 | Exercise on a real .NET fixture; record evidence. |
| C-09 | Inlay hints | 🟢 | Exercise on a real .NET fixture; record evidence. |
| C-10 | Line and block comments | 🟢 | Exercise on a real .NET fixture; record evidence. |
| C-11 | Indentation | 🟢 | Exercise on a real .NET fixture; record evidence. |
| C-12 | Undo and redo | 🟢 | Exercise on a real .NET fixture; record evidence. |
| C-13 | Code actions and quick fixes | 🟢 | Exercise on a real .NET fixture; record evidence. |
| C-14 | Rename symbol | 🟢 | Exercise on a real .NET fixture; record evidence. |
| C-15 | Diagnostics in editor | 🟢 | Exercise on a real .NET fixture; record evidence. |
| C-16 | Large-file safeguards | 🔁 | Inspect sibling implementation, adapt and verify in .NET. |
| C-17 | Complete statement | 🔁 | Inspect sibling implementation, adapt and verify in .NET. |
| C-18 | EditorConfig integration | 🔁 | Inspect sibling implementation, adapt and verify in .NET. |
| C-19 | Spelling assistance | 🔁 | Inspect sibling implementation, adapt and verify in .NET. |
| C-20 | Web language editor improvements | 🔁 | Inspect sibling implementation, adapt and verify in .NET. |
| C-21 | Smart and type-matching completion | ⬜ | Implement with integration and UI acceptance evidence. |
| C-22 | Import completion | ⬜ | Implement with integration and UI acceptance evidence. |
| C-23 | Live templates editor | ⬜ | Implement with integration and UI acceptance evidence. |
| C-24 | Postfix templates | ⬜ | Implement with integration and UI acceptance evidence. |
| C-25 | Surround with templates | ⬜ | Implement with integration and UI acceptance evidence. |
| C-26 | Generate constructors and members | ⬜ | Implement with integration and UI acceptance evidence. |
| C-27 | Override and implement members | ⬜ | Implement with integration and UI acceptance evidence. |
| C-28 | Code cleanup profiles | ⬜ | Implement with integration and UI acceptance evidence. |
| C-29 | Rearrange members | ⬜ | Implement with integration and UI acceptance evidence. |
| C-30 | Formatting preview | ⬜ | Implement with integration and UI acceptance evidence. |
| C-31 | Injected language editing | ⬜ | Implement with integration and UI acceptance evidence. |
| C-32 | Nullable flow analysis presentation | ⬜ | Implement with integration and UI acceptance evidence. |
| C-33 | Solution-wide error analysis | ⬜ | Implement with integration and UI acceptance evidence. |
| C-34 | Inspection severity profiles | ⬜ | Implement with integration and UI acceptance evidence. |
| C-35 | Batch quick fixes | ⬜ | Implement with integration and UI acceptance evidence. |
| C-36 | Roslyn analyzer configuration | ⬜ | Implement with integration and UI acceptance evidence. |
| C-37 | Structural search and replace | ⬜ | Implement with integration and UI acceptance evidence. |
| C-38 | Duplicate code analysis | ⬜ | Implement with integration and UI acceptance evidence. |
| C-39 | AI completion and chat | ⬜ | Implement with integration and UI acceptance evidence. |
| C-40 | AI agents and approval controls | ⬜ | Implement with integration and UI acceptance evidence. |
| C-41 | Agent protocol integrations | ⬜ | Implement with integration and UI acceptance evidence. |

## D. Navigation and search

Source / reuse boundary: `crates/rc-app/src/search.rs; crates/rc-search; crates/rc-app/src/insight.rs`.

| ID | Capability | Status | Next acceptance step |
| --- | --- | --- | --- |
| D-01 | Go to file | 🟢 | Exercise on a real .NET fixture; record evidence. |
| D-02 | Go to type | 🟢 | Exercise on a real .NET fixture; record evidence. |
| D-03 | Go to symbol | 🟢 | Exercise on a real .NET fixture; record evidence. |
| D-04 | Go to definition | 🟢 | Exercise on a real .NET fixture; record evidence. |
| D-05 | Go to implementation | 🟢 | Exercise on a real .NET fixture; record evidence. |
| D-06 | Find usages | 🟢 | Exercise on a real .NET fixture; record evidence. |
| D-07 | Find in files | 🟢 | Exercise on a real .NET fixture; record evidence. |
| D-08 | Replace in files | 🟢 | Exercise on a real .NET fixture; record evidence. |
| D-09 | File structure | 🟢 | Exercise on a real .NET fixture; record evidence. |
| D-10 | Navigation history | 🟢 | Exercise on a real .NET fixture; record evidence. |
| D-11 | Type hierarchy | 🔁 | Inspect sibling implementation, adapt and verify in .NET. |
| D-12 | Call hierarchy | 🔁 | Inspect sibling implementation, adapt and verify in .NET. |
| D-13 | Scope-aware search | 🔁 | Inspect sibling implementation, adapt and verify in .NET. |
| D-14 | Stack trace navigation | 🔁 | Inspect sibling implementation, adapt and verify in .NET. |
| D-15 | Navigate to base and derived symbols | ⬜ | Implement with integration and UI acceptance evidence. |
| D-16 | Related files navigation | ⬜ | Implement with integration and UI acceptance evidence. |
| D-17 | Bookmarks and favorites | ⬜ | Implement with integration and UI acceptance evidence. |
| D-18 | File nesting | ⬜ | Implement with integration and UI acceptance evidence. |
| D-19 | Assembly explorer | ⬜ | Implement with integration and UI acceptance evidence. |
| D-20 | Library decompilation | ⬜ | Implement with integration and UI acceptance evidence. |
| D-21 | SourceLink and symbol servers | ⬜ | Implement with integration and UI acceptance evidence. |
| D-22 | IL viewer | ⬜ | Implement with integration and UI acceptance evidence. |
| D-23 | Native disassembly | ⬜ | Implement with integration and UI acceptance evidence. |
| D-24 | Cross-language usages | ⬜ | Implement with integration and UI acceptance evidence. |
| D-25 | Search in string literals | ⬜ | Implement with integration and UI acceptance evidence. |
| D-26 | Dependency analysis | ⬜ | Implement with integration and UI acceptance evidence. |
| D-27 | Architecture diagrams | ⬜ | Implement with integration and UI acceptance evidence. |

## E. Refactoring

Source / reuse boundary: `crates/rc-app/src/insight.rs; crates/rc-lsp`.

| ID | Capability | Status | Next acceptance step |
| --- | --- | --- | --- |
| E-01 | Cross-file rename | 🟢 | Exercise on a real .NET fixture; record evidence. |
| E-02 | Language-server refactoring actions | 🟢 | Exercise on a real .NET fixture; record evidence. |
| E-03 | Refactor This chooser | ⬜ | Implement with integration and UI acceptance evidence. |
| E-04 | Change signature | ⬜ | Implement with integration and UI acceptance evidence. |
| E-05 | Extract method | ⬜ | Implement with integration and UI acceptance evidence. |
| E-06 | Extract local function | ⬜ | Implement with integration and UI acceptance evidence. |
| E-07 | Extract interface | ⬜ | Implement with integration and UI acceptance evidence. |
| E-08 | Extract superclass | ⬜ | Implement with integration and UI acceptance evidence. |
| E-09 | Extract class | ⬜ | Implement with integration and UI acceptance evidence. |
| E-10 | Introduce variable | ⬜ | Implement with integration and UI acceptance evidence. |
| E-11 | Introduce field | ⬜ | Implement with integration and UI acceptance evidence. |
| E-12 | Introduce property | ⬜ | Implement with integration and UI acceptance evidence. |
| E-13 | Introduce parameter | ⬜ | Implement with integration and UI acceptance evidence. |
| E-14 | Introduce constant | ⬜ | Implement with integration and UI acceptance evidence. |
| E-15 | Inline symbol | ⬜ | Implement with integration and UI acceptance evidence. |
| E-16 | Move type to file | ⬜ | Implement with integration and UI acceptance evidence. |
| E-17 | Move type to namespace | ⬜ | Implement with integration and UI acceptance evidence. |
| E-18 | Move members | ⬜ | Implement with integration and UI acceptance evidence. |
| E-19 | Safe delete | ⬜ | Implement with integration and UI acceptance evidence. |
| E-20 | Pull members up | ⬜ | Implement with integration and UI acceptance evidence. |
| E-21 | Push members down | ⬜ | Implement with integration and UI acceptance evidence. |
| E-22 | Encapsulate field | ⬜ | Implement with integration and UI acceptance evidence. |
| E-23 | Convert property and method | ⬜ | Implement with integration and UI acceptance evidence. |
| E-24 | Convert interface and abstract class | ⬜ | Implement with integration and UI acceptance evidence. |
| E-25 | Adjust namespaces | ⬜ | Implement with integration and UI acceptance evidence. |
| E-26 | Refactoring conflict preview | ⬜ | Implement with integration and UI acceptance evidence. |
| E-27 | Atomic multi-file refactoring undo | ⬜ | Implement with integration and UI acceptance evidence. |
| E-28 | In-place refactorings | ⬜ | Implement with integration and UI acceptance evidence. |
| E-29 | Project-level rename and move | ⬜ | Implement with integration and UI acceptance evidence. |

## F. Build and run

Source / reuse boundary: `crates/rc-dotnet/src/build.rs; crates/rc-app/src/runner.rs`.

| ID | Capability | Status | Next acceptance step |
| --- | --- | --- | --- |
| F-01 | Build solution or project | 🟢 | Exercise on a real .NET fixture; record evidence. |
| F-02 | Rebuild | 🟢 | Exercise on a real .NET fixture; record evidence. |
| F-03 | Clean | 🟢 | Exercise on a real .NET fixture; record evidence. |
| F-04 | Cancel build | 🟢 | Exercise on a real .NET fixture; record evidence. |
| F-05 | Streamed build output | 🟢 | Exercise on a real .NET fixture; record evidence. |
| F-06 | Compiler diagnostics links | 🟢 | Exercise on a real .NET fixture; record evidence. |
| F-07 | Run configurations | 🟢 | Exercise on a real .NET fixture; record evidence. |
| F-08 | Launch profile selection | 🟢 | Exercise on a real .NET fixture; record evidence. |
| F-09 | Stop running processes | 🟢 | Exercise on a real .NET fixture; record evidence. |
| F-10 | Incremental build scheduling | ⬜ | Implement with integration and UI acceptance evidence. |
| F-11 | Build dependency visualization | ⬜ | Implement with integration and UI acceptance evidence. |
| F-12 | Build log viewer | ⬜ | Implement with integration and UI acceptance evidence. |
| F-13 | Before-launch tasks | ⬜ | Implement with integration and UI acceptance evidence. |
| F-14 | Compound run configurations audit | ⬜ | Implement with integration and UI acceptance evidence. |
| F-15 | Environment file support | ⬜ | Implement with integration and UI acceptance evidence. |
| F-16 | Hot Reload | ⬜ | Implement with integration and UI acceptance evidence. |
| F-17 | dotnet watch integration | ⬜ | Implement with integration and UI acceptance evidence. |
| F-18 | Publish profiles | ⬜ | Implement with integration and UI acceptance evidence. |
| F-19 | Deployment targets | ⬜ | Implement with integration and UI acceptance evidence. |
| F-20 | Windows service execution | ⬜ | Implement with integration and UI acceptance evidence. |
| F-21 | Elevated execution | ⬜ | Implement with integration and UI acceptance evidence. |
| F-22 | External tools configuration | ⬜ | Implement with integration and UI acceptance evidence. |
| F-23 | Scratch files and C# interactive | ⬜ | Implement with integration and UI acceptance evidence. |

## G. Debugger

Source / reuse boundary: `crates/rc-dap; crates/rc-app/src/debugger.rs`.

| ID | Capability | Status | Next acceptance step |
| --- | --- | --- | --- |
| G-01 | Launch managed debugger | 🟢 | Exercise on a real .NET fixture; record evidence. |
| G-02 | Line breakpoints | 🟢 | Exercise on a real .NET fixture; record evidence. |
| G-03 | Conditional breakpoints | 🟢 | Exercise on a real .NET fixture; record evidence. |
| G-04 | Step over | 🟢 | Exercise on a real .NET fixture; record evidence. |
| G-05 | Step into | 🟢 | Exercise on a real .NET fixture; record evidence. |
| G-06 | Step out | 🟢 | Exercise on a real .NET fixture; record evidence. |
| G-07 | Run to cursor | 🟢 | Exercise on a real .NET fixture; record evidence. |
| G-08 | Pause and resume | 🟢 | Exercise on a real .NET fixture; record evidence. |
| G-09 | Thread list | 🟢 | Exercise on a real .NET fixture; record evidence. |
| G-10 | Stack frames | 🟢 | Exercise on a real .NET fixture; record evidence. |
| G-11 | Variables tree | 🟢 | Exercise on a real .NET fixture; record evidence. |
| G-12 | Watches | 🟢 | Exercise on a real .NET fixture; record evidence. |
| G-13 | Evaluate expression | 🟢 | Exercise on a real .NET fixture; record evidence. |
| G-14 | Exception breakpoints | 🟢 | Exercise on a real .NET fixture; record evidence. |
| G-15 | Debug hover values | 🟢 | Exercise on a real .NET fixture; record evidence. |
| G-16 | Attach and reattach to process | ⬜ | Implement with integration and UI acceptance evidence. |
| G-17 | Tracepoints | ⬜ | Implement with integration and UI acceptance evidence. |
| G-18 | Hit-count breakpoints | ⬜ | Implement with integration and UI acceptance evidence. |
| G-19 | Dependent breakpoints | ⬜ | Implement with integration and UI acceptance evidence. |
| G-20 | Temporary breakpoints | ⬜ | Implement with integration and UI acceptance evidence. |
| G-21 | Breakpoint groups | ⬜ | Implement with integration and UI acceptance evidence. |
| G-22 | Smart step into | ⬜ | Implement with integration and UI acceptance evidence. |
| G-23 | Force step actions | ⬜ | Implement with integration and UI acceptance evidence. |
| G-24 | Set next statement | ⬜ | Implement with integration and UI acceptance evidence. |
| G-25 | Async stack traces | ⬜ | Implement with integration and UI acceptance evidence. |
| G-26 | Parallel stacks | ⬜ | Implement with integration and UI acceptance evidence. |
| G-27 | Collection and table visualizers | ⬜ | Implement with integration and UI acceptance evidence. |
| G-28 | Memory view | ⬜ | Implement with integration and UI acceptance evidence. |
| G-29 | Registers and disassembly | ⬜ | Implement with integration and UI acceptance evidence. |
| G-30 | Debugger Hot Reload | ⬜ | Implement with integration and UI acceptance evidence. |
| G-31 | Remote debugging | ⬜ | Implement with integration and UI acceptance evidence. |
| G-32 | .NET Framework debugging | ⬜ | Implement with integration and UI acceptance evidence. |
| G-33 | Mono debugging | ⬜ | Implement with integration and UI acceptance evidence. |
| G-34 | Child process debugging | ⬜ | Implement with integration and UI acceptance evidence. |
| G-35 | Debug unit tests | ⬜ | Implement with integration and UI acceptance evidence. |
| G-36 | Debugger symbol configuration | ⬜ | Implement with integration and UI acceptance evidence. |
| G-37 | Source server integration | ⬜ | Implement with integration and UI acceptance evidence. |
| G-38 | Concurrent debug sessions | ⬜ | Implement with integration and UI acceptance evidence. |

## H. Testing, coverage and performance

Source / reuse boundary: `crates/rc-dotnet/src/test.rs; crates/rc-app/src/ui/run_windows.rs`.

| ID | Capability | Status | Next acceptance step |
| --- | --- | --- | --- |
| H-01 | xUnit execution | 🟢 | Exercise on a real .NET fixture; record evidence. |
| H-02 | NUnit execution | 🟢 | Exercise on a real .NET fixture; record evidence. |
| H-03 | MSTest execution | 🟢 | Exercise on a real .NET fixture; record evidence. |
| H-04 | Test discovery | 🟢 | Exercise on a real .NET fixture; record evidence. |
| H-05 | Test result tree | 🟢 | Exercise on a real .NET fixture; record evidence. |
| H-06 | Test gutter icons | 🟢 | Exercise on a real .NET fixture; record evidence. |
| H-07 | TRX results | 🟢 | Exercise on a real .NET fixture; record evidence. |
| H-08 | Test output | 🟢 | Exercise on a real .NET fixture; record evidence. |
| H-09 | Named test sessions | ⬜ | Implement with integration and UI acceptance evidence. |
| H-10 | Test grouping and filtering | ⬜ | Implement with integration and UI acceptance evidence. |
| H-11 | Rerun failed tests | ⬜ | Implement with integration and UI acceptance evidence. |
| H-12 | Continuous testing | ⬜ | Implement with integration and UI acceptance evidence. |
| H-13 | Parameterized test navigation | ⬜ | Implement with integration and UI acceptance evidence. |
| H-14 | Test playlists | ⬜ | Implement with integration and UI acceptance evidence. |
| H-15 | Test impact analysis | ⬜ | Implement with integration and UI acceptance evidence. |
| H-16 | Code coverage collection | ⬜ | Implement with integration and UI acceptance evidence. |
| H-17 | Coverage gutter | ⬜ | Implement with integration and UI acceptance evidence. |
| H-18 | Coverage reports | ⬜ | Implement with integration and UI acceptance evidence. |
| H-19 | Coverage thresholds | ⬜ | Implement with integration and UI acceptance evidence. |
| H-20 | CPU profiling | ⬜ | Implement with integration and UI acceptance evidence. |
| H-21 | Allocation profiling | ⬜ | Implement with integration and UI acceptance evidence. |
| H-22 | Memory snapshots | ⬜ | Implement with integration and UI acceptance evidence. |
| H-23 | Profiling comparison | ⬜ | Implement with integration and UI acceptance evidence. |
| H-24 | Application monitoring | ⬜ | Implement with integration and UI acceptance evidence. |
| H-25 | Performance issue detection | ⬜ | Implement with integration and UI acceptance evidence. |
| H-26 | BenchmarkDotNet integration | ⬜ | Implement with integration and UI acceptance evidence. |

## I. Version control and collaboration

Source / reuse boundary: `crates/rc-vcs; crates/rc-app/src/vcs.rs`.

| ID | Capability | Status | Next acceptance step |
| --- | --- | --- | --- |
| I-01 | Git repository discovery | 🟢 | Exercise on a real .NET fixture; record evidence. |
| I-02 | Changed files | 🟢 | Exercise on a real .NET fixture; record evidence. |
| I-03 | Git staging | 🟢 | Exercise on a real .NET fixture; record evidence. |
| I-04 | Git commit | 🟢 | Exercise on a real .NET fixture; record evidence. |
| I-05 | Branches | 🟢 | Exercise on a real .NET fixture; record evidence. |
| I-06 | Side-by-side diff | 🟢 | Exercise on a real .NET fixture; record evidence. |
| I-07 | Gutter change markers | 🟢 | Exercise on a real .NET fixture; record evidence. |
| I-08 | Git blame | 🟢 | Exercise on a real .NET fixture; record evidence. |
| I-09 | Clone repositories | 🔁 | Inspect sibling implementation, adapt and verify in .NET. |
| I-10 | Git read safety | 🔁 | Inspect sibling implementation, adapt and verify in .NET. |
| I-11 | Shelves | 🔁 | Inspect sibling implementation, adapt and verify in .NET. |
| I-12 | History editing | 🔁 | Inspect sibling implementation, adapt and verify in .NET. |
| I-13 | Hosting CLI integration | 🔁 | Inspect sibling implementation, adapt and verify in .NET. |
| I-14 | Git log and graph | ⬜ | Implement with integration and UI acceptance evidence. |
| I-15 | Fetch pull and push workflow audit | ⬜ | Implement with integration and UI acceptance evidence. |
| I-16 | Interactive rebase | ⬜ | Implement with integration and UI acceptance evidence. |
| I-17 | Cherry-pick | ⬜ | Implement with integration and UI acceptance evidence. |
| I-18 | Three-way merge editor | ⬜ | Implement with integration and UI acceptance evidence. |
| I-19 | Conflict resolution | ⬜ | Implement with integration and UI acceptance evidence. |
| I-20 | Changelists | ⬜ | Implement with integration and UI acceptance evidence. |
| I-21 | Partial commit | ⬜ | Implement with integration and UI acceptance evidence. |
| I-22 | Git worktrees | ⬜ | Implement with integration and UI acceptance evidence. |
| I-23 | Submodules | ⬜ | Implement with integration and UI acceptance evidence. |
| I-24 | Git LFS | ⬜ | Implement with integration and UI acceptance evidence. |
| I-25 | GitHub pull requests | ⬜ | Implement with integration and UI acceptance evidence. |
| I-26 | GitLab merge requests | ⬜ | Implement with integration and UI acceptance evidence. |
| I-27 | Review comments | ⬜ | Implement with integration and UI acceptance evidence. |
| I-28 | Issue tracker tasks | ⬜ | Implement with integration and UI acceptance evidence. |
| I-29 | Shared editing sessions | ⬜ | Implement with integration and UI acceptance evidence. |
| I-30 | Subversion integration | ⬜ | Implement with integration and UI acceptance evidence. |
| I-31 | Mercurial integration | ⬜ | Implement with integration and UI acceptance evidence. |
| I-32 | Perforce integration | ⬜ | Implement with integration and UI acceptance evidence. |
| I-33 | Azure DevOps integration | ⬜ | Implement with integration and UI acceptance evidence. |
| I-34 | Local history | ⬜ | Implement with integration and UI acceptance evidence. |

## J. Web, services and databases

Source / reuse boundary: `crates/rc-syntax; crates/rc-app/src/ui`.

| ID | Capability | Status | Next acceptance step |
| --- | --- | --- | --- |
| J-01 | Razor syntax parsing | 🟢 | Exercise on a real .NET fixture; record evidence. |
| J-02 | XML syntax parsing | 🟢 | Exercise on a real .NET fixture; record evidence. |
| J-03 | JSON syntax parsing | 🟢 | Exercise on a real .NET fixture; record evidence. |
| J-04 | Markdown preview | 🟢 | Exercise on a real .NET fixture; record evidence. |
| J-05 | HTTP client | 🔁 | Inspect sibling implementation, adapt and verify in .NET. |
| J-06 | HTTP environment variables | 🔁 | Inspect sibling implementation, adapt and verify in .NET. |
| J-07 | Database connections and SQL console | 🔁 | Inspect sibling implementation, adapt and verify in .NET. |
| J-08 | SQL injection support | 🔁 | Inspect sibling implementation, adapt and verify in .NET. |
| J-09 | ASP.NET endpoints explorer | 🔁 | Inspect sibling implementation, adapt and verify in .NET. |
| J-10 | Container tools | 🔁 | Inspect sibling implementation, adapt and verify in .NET. |
| J-11 | ASP.NET MVC navigation | ⬜ | Implement with integration and UI acceptance evidence. |
| J-12 | ASP.NET route analysis | ⬜ | Implement with integration and UI acceptance evidence. |
| J-13 | Blazor component assistance | ⬜ | Implement with integration and UI acceptance evidence. |
| J-14 | JavaScript and TypeScript intelligence | ⬜ | Implement with integration and UI acceptance evidence. |
| J-15 | HTML and CSS intelligence | ⬜ | Implement with integration and UI acceptance evidence. |
| J-16 | React support | ⬜ | Implement with integration and UI acceptance evidence. |
| J-17 | Angular support | ⬜ | Implement with integration and UI acceptance evidence. |
| J-18 | Vue support | ⬜ | Implement with integration and UI acceptance evidence. |
| J-19 | Node.js run and debug | ⬜ | Implement with integration and UI acceptance evidence. |
| J-20 | npm yarn and pnpm | ⬜ | Implement with integration and UI acceptance evidence. |
| J-21 | Browser debugging | ⬜ | Implement with integration and UI acceptance evidence. |
| J-22 | Web test runners | ⬜ | Implement with integration and UI acceptance evidence. |
| J-23 | REST request generation | ⬜ | Implement with integration and UI acceptance evidence. |
| J-24 | OpenAPI support | ⬜ | Implement with integration and UI acceptance evidence. |
| J-25 | gRPC client | ⬜ | Implement with integration and UI acceptance evidence. |
| J-26 | WebSocket client | ⬜ | Implement with integration and UI acceptance evidence. |
| J-27 | Entity Framework tooling | ⬜ | Implement with integration and UI acceptance evidence. |
| J-28 | Database schema explorer | ⬜ | Implement with integration and UI acceptance evidence. |
| J-29 | Table data editor | ⬜ | Implement with integration and UI acceptance evidence. |
| J-30 | SQL completion and inspections | ⬜ | Implement with integration and UI acceptance evidence. |
| J-31 | Database migrations | ⬜ | Implement with integration and UI acceptance evidence. |
| J-32 | Schema diff | ⬜ | Implement with integration and UI acceptance evidence. |
| J-33 | Database diagrams | ⬜ | Implement with integration and UI acceptance evidence. |
| J-34 | Query plans | ⬜ | Implement with integration and UI acceptance evidence. |
| J-35 | Docker Compose | ⬜ | Implement with integration and UI acceptance evidence. |
| J-36 | Kubernetes | ⬜ | Implement with integration and UI acceptance evidence. |
| J-37 | .NET Aspire | ⬜ | Implement with integration and UI acceptance evidence. |
| J-38 | Azure toolkit | ⬜ | Implement with integration and UI acceptance evidence. |
| J-39 | AWS toolkit | ⬜ | Implement with integration and UI acceptance evidence. |
| J-40 | Deployment synchronization | ⬜ | Implement with integration and UI acceptance evidence. |

## K. Languages, desktop, mobile and games

Source / reuse boundary: `crates/rc-syntax; crates/rc-lsp; future framework adapters`.

| ID | Capability | Status | Next acceptance step |
| --- | --- | --- | --- |
| K-01 | C# language server integration | 🟢 | Exercise on a real .NET fixture; record evidence. |
| K-02 | F# language intelligence | ⬜ | Implement with integration and UI acceptance evidence. |
| K-03 | VB.NET language intelligence | ⬜ | Implement with integration and UI acceptance evidence. |
| K-04 | C++ language intelligence | ⬜ | Implement with integration and UI acceptance evidence. |
| K-05 | XAML intelligence | ⬜ | Implement with integration and UI acceptance evidence. |
| K-06 | WPF assistance and preview | ⬜ | Implement with integration and UI acceptance evidence. |
| K-07 | WinForms designer | ⬜ | Implement with integration and UI acceptance evidence. |
| K-08 | Avalonia assistance and preview | ⬜ | Implement with integration and UI acceptance evidence. |
| K-09 | .NET MAUI support | ⬜ | Implement with integration and UI acceptance evidence. |
| K-10 | Xamarin project support | ⬜ | Implement with integration and UI acceptance evidence. |
| K-11 | Unity project model | ⬜ | Implement with integration and UI acceptance evidence. |
| K-12 | Unity event methods and inspections | ⬜ | Implement with integration and UI acceptance evidence. |
| K-13 | Unity editor connection | ⬜ | Implement with integration and UI acceptance evidence. |
| K-14 | Unity debugging | ⬜ | Implement with integration and UI acceptance evidence. |
| K-15 | Unity tests | ⬜ | Implement with integration and UI acceptance evidence. |
| K-16 | Unity shader support | ⬜ | Implement with integration and UI acceptance evidence. |
| K-17 | Unity profiling integration | ⬜ | Implement with integration and UI acceptance evidence. |
| K-18 | Unreal Engine project model | ⬜ | Implement with integration and UI acceptance evidence. |
| K-19 | Unreal C++ inspections | ⬜ | Implement with integration and UI acceptance evidence. |
| K-20 | Unreal reflection assistance | ⬜ | Implement with integration and UI acceptance evidence. |
| K-21 | Blueprints navigation | ⬜ | Implement with integration and UI acceptance evidence. |
| K-22 | Unreal debugging | ⬜ | Implement with integration and UI acceptance evidence. |
| K-23 | Unreal tests | ⬜ | Implement with integration and UI acceptance evidence. |
| K-24 | Unreal shader support | ⬜ | Implement with integration and UI acceptance evidence. |
| K-25 | Godot project support | ⬜ | Implement with integration and UI acceptance evidence. |
| K-26 | Godot debugging | ⬜ | Implement with integration and UI acceptance evidence. |
| K-27 | Localization resource editor | ⬜ | Implement with integration and UI acceptance evidence. |
| K-28 | Assembly metadata explorer | ⬜ | Implement with integration and UI acceptance evidence. |

## L. Terminal, remote development and extensibility

Source / reuse boundary: `crates/rc-term; crates/rc-core; CrabCraft shared modules`.

| ID | Capability | Status | Next acceptance step |
| --- | --- | --- | --- |
| L-01 | Integrated terminal | 🟢 | Exercise on a real .NET fixture; record evidence. |
| L-02 | Persistent workspace sessions | 🟢 | Exercise on a real .NET fixture; record evidence. |
| L-03 | File system watching | 🟢 | Exercise on a real .NET fixture; record evidence. |
| L-04 | Background task runtime | 🟢 | Exercise on a real .NET fixture; record evidence. |
| L-05 | Terminal search and split panes | 🔁 | Inspect sibling implementation, adapt and verify in .NET. |
| L-06 | Terminal completion | 🔁 | Inspect sibling implementation, adapt and verify in .NET. |
| L-07 | Remote SSH workflows | 🔁 | Inspect sibling implementation, adapt and verify in .NET. |
| L-08 | WSL workspace workflows | 🔁 | Inspect sibling implementation, adapt and verify in .NET. |
| L-09 | Development containers | 🔁 | Inspect sibling implementation, adapt and verify in .NET. |
| L-10 | Remote file access | 🔁 | Inspect sibling implementation, adapt and verify in .NET. |
| L-11 | Local plugin lifecycle | 🔁 | Inspect sibling implementation, adapt and verify in .NET. |
| L-12 | Local AI providers | 🔁 | Inspect sibling implementation, adapt and verify in .NET. |
| L-13 | Remote IDE backend | ⬜ | Implement with integration and UI acceptance evidence. |
| L-14 | Gateway-style connection management | ⬜ | Implement with integration and UI acceptance evidence. |
| L-15 | Remote SDK configuration | ⬜ | Implement with integration and UI acceptance evidence. |
| L-16 | Remote reconnect and recovery | ⬜ | Implement with integration and UI acceptance evidence. |
| L-17 | Port forwarding | ⬜ | Implement with integration and UI acceptance evidence. |
| L-18 | Proxy configuration | ⬜ | Implement with integration and UI acceptance evidence. |
| L-19 | Credentials store | ⬜ | Implement with integration and UI acceptance evidence. |
| L-20 | Plugin SDK | ⬜ | Implement with integration and UI acceptance evidence. |
| L-21 | Plugin permissions | ⬜ | Implement with integration and UI acceptance evidence. |
| L-22 | Plugin compatibility checks | ⬜ | Implement with integration and UI acceptance evidence. |
| L-23 | Settings export and import | ⬜ | Implement with integration and UI acceptance evidence. |
| L-24 | Accessibility audit | ⬜ | Implement with integration and UI acceptance evidence. |
| L-25 | Performance diagnostics | ⬜ | Implement with integration and UI acceptance evidence. |
| L-26 | Crash reporting controls | ⬜ | Implement with integration and UI acceptance evidence. |
| L-27 | Offline tool installation | ⬜ | Implement with integration and UI acceptance evidence. |
| L-28 | Platform packaging verification | ⬜ | Implement with integration and UI acceptance evidence. |
