# DotCraft feature inventory and implementation plan

Status of record for Rider-inspired development. Written in English; the original
inventory was reviewed on 2026-11-05 against public Rider 2026.2 documentation.
The source baseline for ongoing work is `javy-fernandez/dotcraft` at `becfb0f`
(November 2026). Existing status marks retain their tested scope; newer upstream
features are classified as source-located until their acceptance checks are repeated.
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
| ✅ Verified | 76 |
| 🚧 In progress | 44 |
| 🟢 Existing | 34 |
| 🔁 Reuse candidate | 17 |
| ⬜ Backlog | 202 |
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

Verification now includes parser and explorer regressions, actual `open_path` lifecycle,
index/Git roots, both watcher locations, dirty-document preservation on retarget, and a real
SDK build with a deliberately invalid policy beside the filter. The workspace uses the
source solution root; the filter remains its session identity and command target.
[Solution-filter evidence](docs/implementation/solution-filters.md) records commands and
remaining filter-specific Roslyn, test-execution, dependency-closure, and platform checks.
B-14 remains 🚧 for that broader scope.

The test-project selection follow-up was closed without merge at the contributor's
request ([PR 153](https://github.com/javy-fernandez/dotcraft/pull/153)). Filter-specific
test selection and execution therefore remain unverified in the original branch.

L-05: a terminal search slice was validated on an isolated branch, then closed without
merge at the contributor's request ([PR 152](https://github.com/javy-fernandez/dotcraft/pull/152)).
The original branch has neither that search code nor split terminal panes. The
[historical evidence](docs/implementation/terminal-search.md) does not count as delivery.

D-14: the terminal parser now recognizes canonical `.NET` stack-trace file and line
locations in Build, Run, and Terminal output. Parser and terminal tests pass, and the
desktop app compiles with the existing Ctrl/Cmd+click navigation route. Wrapped
frames, remote source mapping, symbol-only frames, and live cross-platform UI
validation remain pending. D-14 remains 🚧. See [stack-trace evidence](docs/implementation/stack-trace-navigation.md).


## Current delivery evidence

Combined build and test commands are recorded in [validation evidence](docs/implementation/validation.md).
Live C# language-service, application, and build/run/test-framework checks are recorded in [live .NET evidence](docs/implementation/live-dotnet.md). These three Rust integration tests include 13 successful .NET fixture test cases.

The inventory began in fork PR 1 and was reconciled from fork backlog commit
`9ddea6f0c3a9ccf0841e172327eedde334726f5c`. The original repository incorporated its combined
implementation through upstream PRs [108](https://github.com/javy-fernandez/dotcraft/pull/108)
and [109](https://github.com/javy-fernandez/dotcraft/pull/109)
at `a0f31fb`. The original has since added coverage, profiling, monitoring, local
history, live templates, HTTP environments, and NuGet Central Package Management.
These newer capabilities need scoped validation before they can be marked complete.
The high-level checklist and delivery index are in [docs/features.md](docs/features.md).
Historical fork PR numbers in individual rows identify the original evidence slices.
They do not imply those capabilities are absent from the original repository.

## A. Desktop interface and workflow

Source / reuse boundary: `crates/rc-app/src/ui; crates/rc-kit`.

| ID | Capability | Status | Next acceptance step |
| --- | --- | --- | --- |
| A-01 | Native desktop workspace | ✅ | [PR 206](https://github.com/javy-fernandez/dotcraft/pull/206) verifies Welcome drop of six-project Shop.sln, C# tab opening, Close Project, and recent-project reopen with tab restoration in egui on Linux. Native title bars, OS file associations and other platforms remain untested. |
| A-02 | Light and dark themes | ✅ | [PR 191](https://github.com/javy-fernandez/dotcraft/pull/191) verifies live light → dark → light egui visual changes while six-project `Shop.sln` remains open on Linux. System-theme notifications, native chrome and other platforms remain untested. |
| A-03 | Tool window stripes | ✅ | Existing compact overflow scrolling passes; [PR 197](https://github.com/javy-fernandez/dotcraft/pull/197) verifies Problems/Notifications stripe toggles and dock sides in real `Shop.sln` egui UI on Linux. Context-menu Move To, native chrome and other platforms remain untested. |
| A-04 | Docked tool windows | ✅ | [PR 193](https://github.com/javy-fernandez/dotcraft/pull/193) verifies Explorer stripe hide/reopen, left-to-right dock move and session-map round-trip in a real `Shop.sln` egui UI harness on Linux. Pointer drag, context-menu Move To and other platforms remain untested. |
| A-05 | Editor tabs | ✅ | [PR 174](https://github.com/javy-fernandez/dotcraft/pull/174) names close controls for accessibility; egui UI harness activates and closes C# tabs from real `Shop.sln` on Linux. Pinned/preview tabs, drag ordering, compact widths and other platforms remain untested. |
| A-06 | Split editors | ✅ | [PR 179](https://github.com/javy-fernandez/dotcraft/pull/179) tests tab split commands in both orientations and Unsplit in an egui UI harness with real `Shop.sln` C# files on Linux. Divider drag, context-menu pointer flow, narrow layouts and persistence remain untested. |
| A-07 | Search Everywhere | 🚧 | [PR 182](https://github.com/javy-fernandez/dotcraft/pull/182) disambiguates accessible file results and verifies `Shop.Cli/Program.cs` selection from a real `Shop.sln` index in egui on Linux. Type/symbol search, live index updates and larger workspaces remain unverified. |
| A-08 | Recent projects | ✅ | [PR 176](https://github.com/javy-fernandez/dotcraft/pull/176) exposes full recent paths to accessibility; egui UI harness opens real `Shop.sln` from Welcome and verifies recent-history order on Linux. Missing-path removal, compact widths and other platforms remain untested. |
| A-09 | Recent files | ✅ | [PR 170](https://github.com/javy-fernandez/dotcraft/pull/170) keeps filtered selection valid and exposes file rows to accessibility; egui UI harness opens three real `Shop.sln` C# files and selects one with Enter on Linux. Missing files and native menus remain untested. |
| A-10 | Recent locations | ✅ | [PR 171](https://github.com/javy-fernandez/dotcraft/pull/171) bounds dialog selection and exposes path/line to accessibility; egui UI harness navigates to line four of real `Shop.sln` C# source on Linux. Closed/deleted files and native menus remain untested. |
| A-11 | Settings dialog | ✅ | [PR 198](https://github.com/javy-fernandez/dotcraft/pull/198) verifies applying a persisted Editor setting and cancelling a later draft while real six-project `Shop.sln` remains open in egui on Linux. Responsive layout has separate A-16 evidence; other settings pages and OS integrations remain untested. |
| A-12 | Rider and Visual Studio keymaps | 🚧 | [PR 199](https://github.com/javy-fernandez/dotcraft/pull/199) verifies both Go to File shortcuts after switching keymaps in the open Shop solution on Linux; it also refreshes macOS native menu accelerators on change. Other shortcuts and macOS behavior remain untested. |
| A-13 | Localization | 🚧 | [PR 203](https://github.com/javy-fernandez/dotcraft/pull/203) verifies live English, Portuguese and Spanish Explorer labels while Shop.sln remains open in egui on Linux. Other UI surfaces, catalog completeness, native menus and platforms remain unverified. |
| A-14 | Status bar | ✅ | [PR 167](https://github.com/javy-fernandez/dotcraft/pull/167) exposes custom metadata controls to accessibility; egui harness verifies breadcrumb, UTF-8 BOM, LF, caret and Go to Line dispatch with real `Shop.sln` C# file on Linux. Compact widths and other platforms remain untested. |
| A-15 | Notifications | ✅ | [PR 190](https://github.com/javy-fernandez/dotcraft/pull/190) checks visible status messages and 8-second informational / 15-second error expiry in a real `Shop.sln` egui UI harness on Linux; [PR 161](https://github.com/javy-fernandez/dotcraft/pull/161) covers expiry while Zen hides the status bar. History, compact layouts and other platforms remain untested. |
| A-16 | Responsive settings layout | ✅ | Existing PR 4 preserved; 5 responsive settings tests pass, including TenProjects, scrolling, long paths and Cancel. |
| A-17 | Scrollable main menu | ✅ | PR 17 verifies compact category/command/tool scrolling, real TenProjects integration, clipboard actions and project-gated attachment. |
| A-18 | Tab docking previews | 🔁 | Inspect sibling implementation, adapt and verify in .NET. |
| A-19 | Background document saving | 🔁 | Inspect sibling implementation, adapt and verify in .NET. |
| A-20 | Safe reload from disk | ✅ | [PR 205](https://github.com/javy-fernandez/dotcraft/pull/205) routes conflict-banner disk loading through parse/style refresh and verifies both Load from Disk and Keep Mine in the open Shop solution egui UI on Linux. Watcher timing, large/binary files and other platforms remain untested. |
| A-21 | Accessible keyboard navigation | 🚧 | [PR 214](https://github.com/javy-fernandez/dotcraft/pull/214) verifies Enter on the accessible recent Shop solution and Space to hide/reopen Explorer in egui on Linux. Editor tabs, dialogs, menus, screen readers and other platforms need broader acceptance. |
| A-22 | Distraction-free and Zen modes | ✅ | [PR 161](https://github.com/javy-fernandez/dotcraft/pull/161) provides View-menu toggles, a visible exit control, and dock-preserving layout changes. Two rc-app UI harness tests passed with `Shop.sln` on Linux. Modes are process-local; native window chrome and other platforms remain untested. |
| A-23 | Presentation mode | ✅ | [PR 211](https://github.com/javy-fernandez/dotcraft/pull/211) adds View-menu entry/exit, larger editor text, fullscreen request and full editor area while preserving the dock; Shop.sln egui flow passes on Linux, including settings changes and exit after Close Project. OS fullscreen, display scaling and other platforms remain untested. |
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
| B-01 | Open solutions and folders | ✅ | [PR 175](https://github.com/javy-fernandez/dotcraft/pull/175) verifies solution/folder traversal and safe internal directory symlinks on Unix; 13 rc-core tests pass. [PR 188](https://github.com/javy-fernandez/dotcraft/pull/188) opens `Shop/src` and six-project `Shop.sln` as distinct egui workspaces on Linux. SDK execution, Explorer symlink UI and other platforms remain untested. |
| B-02 | Classic .sln solutions | ✅ | Live Shop build/run and C# application flows load classic solutions; see live-dotnet.md. |
| B-03 | XML .slnx solutions | ✅ | Live Roslyn and application flows load a real two-project XML solution; see live-dotnet.md. |
| B-04 | Standalone .NET projects | ✅ | [PR 169](https://github.com/javy-fernandez/dotcraft/pull/169) discovers one top-level C#, F#, or VB project in a solution-free folder while preserving the session key; 13 rc-core tests pass. [PR 185](https://github.com/javy-fernandez/dotcraft/pull/185) verifies Shop.Cli folder opening, runnable configuration and egui app rendering on Linux. SDK build/run and other platforms remain untested. |
| B-05 | Solution folders and items | ✅ | [PR 177](https://github.com/javy-fernandez/dotcraft/pull/177) restricts classic `.sln` loose items to `SolutionItems` sections; Shop `.sln`, TenProjects `.slnx`, and nested-folder fixtures verify parser membership in seven tests. UI and SDK execution remain untested. |
| B-06 | Project templates | 🚧 | [PR 187](https://github.com/javy-fernandez/dotcraft/pull/187) validates `.sln`/`.slnx` targets before `dotnet new` and rejects missing, directory and `.slnf` targets; Shop/TenProjects plans pass three rc-dotnet tests. Actual SDK creation and UI acceptance remain. |
| B-07 | File and class templates | 🚧 | [PR 217](https://github.com/javy-fernandez/dotcraft/pull/217) verifies New Class creation in a copied Shop.Core project, namespace generation, opened tab and duplicate refusal in egui on Linux. Other item kinds, project-system integration, background file creation and platforms remain. |
| B-08 | .NET SDK discovery | ✅ | [PR 225](https://github.com/javy-fernandez/dotcraft/pull/225) fixes extensionless explicit host discovery and verifies explicit executable/directory/PATH discovery, SDK 8.0.428 listing/version and a successful build of a temporary Shop.Core copy on Linux. Windows/macOS and managed SDK installation remain untested. |
| B-09 | Runtime discovery | ✅ | [PR 226](https://github.com/javy-fernandez/dotcraft/pull/226) parses whitespace-tolerant runtime listings and verifies .NETCore.App/AspNetCore.App 8.0.34 installed by official local SDK 8.0.428 against Shop.Api net8.0 on Linux. UI/server launch and other platforms remain untested. |
| B-10 | MSBuild property evaluation | ✅ | [PR 230](https://github.com/javy-fernandez/dotcraft/pull/230) rejects undeclared target frameworks and verifies Debug/Release, single/multi-target outer and inner property evaluation on Shop.Core/Shop.Api with official SDK 8.0.428 on Linux. UI, other platforms and build/restore behavior remain separate. |
| B-11 | Launch settings profiles | 🚧 | [PR 181](https://github.com/javy-fernandez/dotcraft/pull/181) fixes URL scheme detection and verifies Shop.Api profile order, URL, and environment in four rc-dotnet tests. Browser opening, SDK run, and UI profile selection remain unverified. |
| B-12 | NuGet search | 🚧 | [PR 221](https://github.com/javy-fernandez/dotcraft/pull/221) bounds query, pagination and response size before/during NuGet V3 requests; a Shop.Api package ID is tested through a local fake feed. Live nuget.org and app UI remain unverified. |
| B-13 | NuGet installation and removal | 🚧 | [PR 219](https://github.com/javy-fernandez/dotcraft/pull/219) preflights project path, bounded package ID and version before add/update/remove process spawn; a copied Shop.Api project and fake host verify valid arguments and no spawn on invalid input. Real SDK restore, project modification and UI remain. |
| B-14 | JSON .slnf solution filters | 🚧 | Parser, explorer and SDK build are verified in the original baseline; filter-specific Roslyn, test-project selection and actual test execution remain pending. The test-selection [PR 153](https://github.com/javy-fernandez/dotcraft/pull/153) was closed without merge. |
| B-15 | Interactive filter creation and editing | 🚧 | [PR 209](https://github.com/javy-fernandez/dotcraft/pull/209) adds bounded `.slnf` create/update backend API, preserving unknown JSON fields and BOM, tested on TenProjects.slnx. UI workflow, persistence, SDK build and language-service update remain. |
| B-16 | Unload and reload individual projects | ⬜ | Implement with integration and UI acceptance evidence. |
| B-17 | Project references editor | 🚧 | [PR 204](https://github.com/javy-fernandez/dotcraft/pull/204) adds bounded backend add/remove with canonical path checks and formatting preservation, tested on Shop projects. UI, disk write, MSBuild evaluation and cycle detection remain. |
| B-18 | Target frameworks editor | 🚧 | [PR 200](https://github.com/javy-fernandez/dotcraft/pull/200) adds a bounded backend edit for one unconditional SDK-style project property, tested with Shop.Core and Shop.Api; UI integration, disk write, and SDK build acceptance remain. |
| B-19 | Build configurations and platforms | 🚧 | [PR 156](https://github.com/javy-fernandez/dotcraft/pull/156) parses `.sln` / `.slnf` configuration-platform pairs and lets the backend pass an explicit platform; 134 rc-dotnet unit tests pass. UI selection, persistence, `.slnx` discovery, and a real nondefault-platform build remain. |
| B-20 | global.json SDK management | 🚧 | PR 5 verifies real SDK 8.0.427/10.0.403 selection and builds; ancestor policy display, UI installation and language-service restart remain pending. |
| B-21 | .NET workload management | ⬜ | Implement with integration and UI acceptance evidence. |
| B-22 | NuGet dependency graph | 🚧 | [PR 216](https://github.com/javy-fernandez/dotcraft/pull/216) builds a bounded static graph of direct packages with CPM, override and unresolved versions, tested on CentralShop and Shop.Api. Restore assets/transitives, MSBuild evaluation, UI and SDK validation remain. |
| B-23 | Central package management | 🚧 | [PR 192](https://github.com/javy-fernandez/dotcraft/pull/192) resolves parent CPM imports and child `PackageVersion Update` without inventing missing packages; CentralShop fixture and 132 rc-dotnet unit tests pass. SDK/MSBuild evaluation, arbitrary import order and UI acceptance remain. |
| B-24 | NuGet feed credentials | ⬜ | Implement with integration and UI acceptance evidence. |
| B-25 | Vulnerable and outdated packages | ⬜ | Implement with integration and UI acceptance evidence. |
| B-26 | NuGet Package Manager Console | ⬜ | Implement with integration and UI acceptance evidence. |
| B-27 | Project migration and upgrade | ⬜ | Implement with integration and UI acceptance evidence. |
| B-28 | Solution dependency diagrams | 🚧 | [PR 212](https://github.com/javy-fernandez/dotcraft/pull/212) builds a bounded solution graph and escaped DOT export, tested on six-project Shop.sln, filtered Shop.slnf and cycle cases. UI rendering, MSBuild evaluation and build behavior remain. |
| B-29 | Source generators inspection | ⬜ | Implement with integration and UI acceptance evidence. |

## C. Editor and language intelligence

Source / reuse boundary: `crates/rc-editor; crates/rc-text; crates/rc-syntax; crates/rc-lsp`.

| ID | Capability | Status | Next acceptance step |
| --- | --- | --- | --- |
| C-01 | C# syntax highlighting | ✅ | [PR 229](https://github.com/javy-fernandez/dotcraft/pull/229) preserves lexical comment/string color against semantic tokens and verifies actual Shop C# EditorView paint in built-in light and dark themes. Custom themes, full snapshots and live Roslyn semantic delivery remain untested. |
| C-02 | Roslyn semantic highlighting | ✅ | Live C# application regression verifies Roslyn semantic color for a class; see live-dotnet.md. |
| C-03 | Code completion | ✅ | Live Roslyn and application regressions verify C# completion; advanced completion variants remain separate inventory targets. |
| C-04 | Signature help | 🚧 | [PR 224](https://github.com/javy-fernandez/dotcraft/pull/224) updates the active argument at the caret in Shop C# EditorView, including nested and quoted commas. Metadata is supplied directly; Roslyn/app delivery and other C# forms remain unverified. |
| C-05 | Quick documentation | 🟢 | Exercise on a real .NET fixture; record evidence. |
| C-06 | Automatic brackets | ✅ | [PR 168](https://github.com/javy-fernandez/dotcraft/pull/168) fixes C# comment/string context and verifies pairing, type-over, selection wrapping, and undo through egui editor events on a real Shop C# fixture; 17 rc-editor unit and one fixture test pass. No SDK build or broader Rider parity claim. |
| C-07 | Multiple carets | ✅ | [PR 178](https://github.com/javy-fernandez/dotcraft/pull/178) fixes data loss with mixed selection and empty caret during bracket typing; 27 rc-text, 17 rc-editor and one real Shop C# editor-event test verify per-range edits and one-step undo. No SDK build or broader input matrix claim. |
| C-08 | Code folding | ✅ | [PR 196](https://github.com/javy-fernandez/dotcraft/pull/196) fixes Fold All when the caret is on a visible fold-start line; real Shop C# EditorView test covers Fold/Unfold/Fold All/Unfold All, caret relocation and unchanged buffer, plus 17 unit tests. No SDK build or other-language claim. |
| C-09 | Inlay hints | 🚧 | [PR 202](https://github.com/javy-fernandez/dotcraft/pull/202) fixes requested left/right padding, verified in Shop C# layout and EditorView caret navigation. Roslyn hint delivery and other hint types remain unverified. |
| C-10 | Line and block comments | ✅ | [PR 172](https://github.com/javy-fernandez/dotcraft/pull/172) fixes repeated block-comment toggle and verifies Rider line/block shortcuts, selection and undo through editor events on a real Shop C# file; 17 unit and two fixture tests pass. No SDK build or broader Rider parity claim. |
| C-11 | Indentation | ✅ | [PR 189](https://github.com/javy-fernandez/dotcraft/pull/189) indents C# `case`/`default` statements one level on Enter as one undoable edit; real Shop C# EditorView event and guard tests pass (18 unit, one fixture). Other language-specific indentation remains outside this scope. |
| C-12 | Undo and redo | ✅ | [PR 183](https://github.com/javy-fernandez/dotcraft/pull/183) restores diagnostic/inlay anchors on history edits and clears stale search highlights; real Shop C# EditorView test verifies text, caret, undo/redo and divergent-edit invalidation. 17 unit and one fixture test pass. No SDK build claim. |
| C-13 | Code actions and quick fixes | 🟢 | Exercise on a real .NET fixture; record evidence. |
| C-14 | Rename symbol | ✅ | Live C# application and Roslyn tests rename symbols across App and Lib projects; see live-dotnet.md. |
| C-15 | Diagnostics in editor | ✅ | Live application verifies CS0103 ranges; Roslyn regression verifies diagnostics; see live-dotnet.md. |
| C-16 | Large-file safeguards | 🚧 | [PR 163](https://github.com/javy-fernandez/dotcraft/pull/163) bounds editor line layout to 4,096 characters while preserving the full buffer; 18 rc-editor unit tests pass, including a 100,000-character line. Decorations, exact horizontal placement, and live UI acceptance remain. |
| C-17 | Complete statement | 🚧 | [PR 157](https://github.com/javy-fernandez/dotcraft/pull/157) adds conservative C# single-line statement completion with one undo step; 21 rc-editor unit tests pass. Broader syntax handling and live UI acceptance remain. |
| C-18 | EditorConfig integration | 🚧 | PR 3 verifies the indentation subset, embedded globstars and logical mixed editing. [PR 162](https://github.com/javy-fernandez/dotcraft/pull/162) adds signed numeric-range globs up to 256 values; 13 rc-core tests pass. Save-time/full-formatting and live UI scope remain pending. |
| C-19 | Spelling assistance | 🔁 | Inspect sibling implementation, adapt and verify in .NET. |
| C-20 | Web language editor improvements | 🔁 | Inspect sibling implementation, adapt and verify in .NET. |
| C-21 | Smart and type-matching completion | ⬜ | Implement with integration and UI acceptance evidence. |
| C-22 | Import completion | ⬜ | Implement with integration and UI acceptance evidence. |
| C-23 | Live templates editor | 🚧 | [PR 218](https://github.com/javy-fernandez/dotcraft/pull/218) expands `cw` in Shop C# code with undo and preserves literals in comments/strings through EditorView. App popup, custom templates, advanced strings and very long lines remain unverified. |
| C-24 | Postfix templates | 🟢 | Existing C# postfix expansion is in `rc-editor/src/templates.rs` and invoked by Tab in `view.rs`; unit tests cover the supported forms. Live .NET UI acceptance remains. |
| C-25 | Surround with templates | 🚧 | [PR 210](https://github.com/javy-fernandez/dotcraft/pull/210) verifies Ctrl+Alt+T, built-in `if` surround, indentation, snippet field and undo in Shop C# EditorView; read-only phantom snippets are prevented. App popup and other templates remain untested. |
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
| D-01 | Go to file | ✅ | [PR 228](https://github.com/javy-fernandez/dotcraft/pull/228) verifies indexed `PriceCalculator.cs:12` Go to File selection and exact C# caret position in the real Shop.sln egui UI on Linux. Duplicate basenames, live index updates, remote paths and other platforms remain untested. |
| D-02 | Go to type | 🟢 | Exercise on a real .NET fixture; record evidence. |
| D-03 | Go to symbol | 🟢 | Exercise on a real .NET fixture; record evidence. |
| D-04 | Go to definition | 🟢 | Exercise on a real .NET fixture; record evidence. |
| D-05 | Go to implementation | 🟢 | Exercise on a real .NET fixture; record evidence. |
| D-06 | Find usages | 🟢 | Exercise on a real .NET fixture; record evidence. |
| D-07 | Find in files | ✅ | [PR 222](https://github.com/javy-fernandez/dotcraft/pull/222) gives each result an accessible path/line/text label and verifies literal search plus exact C# source navigation in the real Shop.sln egui UI on Linux. Regex, replacement, huge workspaces and other platforms remain untested. |
| D-08 | Replace in files | ✅ | [PR 232](https://github.com/javy-fernandez/dotcraft/pull/232) moves closed-file replacement off the egui thread and verifies local Shop C# open-buffer Undo, fresh closed-file disk replacement and invalid UTF-8 refusal on Linux. Regex, mid-file cancellation, concurrency, huge workspaces and other platforms remain untested. |
| D-09 | File structure | 🟢 | Exercise on a real .NET fixture; record evidence. |
| D-10 | Navigation history | ✅ | [PR 220](https://github.com/javy-fernandez/dotcraft/pull/220) skips removed files even when their editor buffer remains open, preserving Back/Forward stacks and exact C# caret locations in the real Shop.sln egui UI on Linux. Renames, remote files and other platforms remain untested. |
| D-11 | Type hierarchy | 🔁 | Inspect sibling implementation, adapt and verify in .NET. |
| D-12 | Call hierarchy | 🔁 | Inspect sibling implementation, adapt and verify in .NET. |
| D-13 | Scope-aware search | 🔁 | Inspect sibling implementation, adapt and verify in .NET. |
| D-14 | Stack trace navigation | 🚧 | Canonical .NET file-and-line links are tested; wrapped frames, remote source mapping and live UI validation remain pending. |
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
| E-01 | Cross-file rename | ✅ | Live C# rename updates App and Lib documents through the application; broader refactorings remain separate targets. |
| E-02 | Language-server refactoring actions | 🚧 | PR 10 rejects unsupported resource edits before mutation; text-only refactorings still need transactional validation. |
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
| F-01 | Build solution or project | ✅ | Live Shop solution build and inherited CLI/API debugger project builds pass; see live-dotnet.md. |
| F-02 | Rebuild | 🟢 | Exercise on a real .NET fixture; record evidence. |
| F-03 | Clean | 🟢 | Exercise on a real .NET fixture; record evidence. |
| F-04 | Cancel build | 🟢 | Exercise on a real .NET fixture; record evidence. |
| F-05 | Streamed build output | 🟢 | Exercise on a real .NET fixture; record evidence. |
| F-06 | Compiler diagnostics links | 🟢 | Exercise on a real .NET fixture; record evidence. |
| F-07 | Run configurations | ✅ | [PR 227](https://github.com/javy-fernandez/dotcraft/pull/227) exposes the toolbar selection to accessibility and verifies choosing Shop.Cli plus session restoration after reopening real Shop.sln in egui on Linux. Launch profiles, SDK execution, compound editing and other platforms remain separate acceptance scope. |
| F-08 | Launch profile selection | 🟢 | Exercise on a real .NET fixture; record evidence. |
| F-09 | Stop running processes | ✅ | [PR 231](https://github.com/javy-fernandez/dotcraft/pull/231) verifies live Shop.Api startup on ephemeral loopback, process ID, Stop exit and Run-console report using official .NET SDK 8.0.428 on Linux. Debugger stop, hot reload, other runtimes and platforms remain untested. |
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
| G-01 | Launch managed debugger | ✅ | Verified on .NET 8 Linux by the real CLI/API debugger regression; see docs/implementation/attach-debugger.md. |
| G-02 | Line breakpoints | ✅ | Verified on .NET 8 Linux by the real CLI/API debugger regression; see docs/implementation/attach-debugger.md. |
| G-03 | Conditional breakpoints | ✅ | Verified on .NET 8 Linux by the real CLI/API debugger regression; see docs/implementation/attach-debugger.md. |
| G-04 | Step over | ✅ | Verified on .NET 8 Linux by the real CLI/API debugger regression; see docs/implementation/attach-debugger.md. |
| G-05 | Step into | 🟢 | Existing implementation; the live CLI/API regression does not explicitly validate Step Into. |
| G-06 | Step out | 🟢 | Existing implementation; the live CLI/API regression does not explicitly validate Step Out. |
| G-07 | Run to cursor | ✅ | Verified on .NET 8 Linux by the real CLI/API debugger regression; see docs/implementation/attach-debugger.md. |
| G-08 | Pause and resume | 🟢 | Resume is exercised live; explicit pause validation remains pending. |
| G-09 | Thread list | 🟢 | Existing thread model participates in live debugging; explicit thread-list/UI validation remains pending. |
| G-10 | Stack frames | ✅ | Verified on .NET 8 Linux by the real CLI/API debugger regression; see docs/implementation/attach-debugger.md. |
| G-11 | Variables tree | ✅ | Verified on .NET 8 Linux by the real CLI/API debugger regression; see docs/implementation/attach-debugger.md. |
| G-12 | Watches | ✅ | Verified on .NET 8 Linux by the real CLI/API debugger regression; see docs/implementation/attach-debugger.md. |
| G-13 | Evaluate expression | ✅ | Verified on .NET 8 Linux by the real CLI/API debugger regression; see docs/implementation/attach-debugger.md. |
| G-14 | Exception breakpoints | 🟢 | Exercise on a real .NET fixture; record evidence. |
| G-15 | Debug hover values | ✅ | Verified live line.Subtotal hover value in the CLI/API debugger regression. |
| G-16 | Attach and reattach to process | 🚧 | PR 14 verifies live PID attach/detach on .NET 8 Linux; automatic reattach and broader runtime coverage remain pending. |
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
| H-01 | xUnit execution | ✅ | Live Shop runner executes 5 xUnit cases successfully; see live-dotnet.md. |
| H-02 | NUnit execution | ✅ | Live Shop runner executes 4 NUnit cases successfully; see live-dotnet.md. |
| H-03 | MSTest execution | ✅ | Live Shop runner executes 4 MSTest cases successfully; see live-dotnet.md. |
| H-04 | Test discovery | 🟢 | Exercise on a real .NET fixture; record evidence. |
| H-05 | Test result tree | 🟢 | Exercise on a real .NET fixture; record evidence. |
| H-06 | Test gutter icons | 🟢 | Exercise on a real .NET fixture; record evidence. |
| H-07 | TRX results | ✅ | Live Shop runner parses real TRX outcomes for all 13 fixture cases; see live-dotnet.md. |
| H-08 | Test output | 🟢 | Exercise on a real .NET fixture; record evidence. |
| H-09 | Named test sessions | ⬜ | Implement with integration and UI acceptance evidence. |
| H-10 | Test grouping and filtering | ⬜ | Implement with integration and UI acceptance evidence. |
| H-11 | Rerun failed tests | ⬜ | Implement with integration and UI acceptance evidence. |
| H-12 | Continuous testing | ⬜ | Implement with integration and UI acceptance evidence. |
| H-13 | Parameterized test navigation | ⬜ | Implement with integration and UI acceptance evidence. |
| H-14 | Test playlists | ⬜ | Implement with integration and UI acceptance evidence. |
| H-15 | Test impact analysis | ⬜ | Implement with integration and UI acceptance evidence. |
| H-16 | Code coverage collection | 🟢 | Source located in `rc-dotnet/coverage.rs`; repeat real .NET acceptance checks. |
| H-17 | Coverage gutter | 🟢 | Source located in `rc-app/coverage.rs`; repeat application acceptance checks. |
| H-18 | Coverage reports | 🟢 | Source located in the coverage window; repeat real .NET acceptance checks. |
| H-19 | Coverage thresholds | ⬜ | Implement with integration and UI acceptance evidence. |
| H-20 | CPU profiling | 🟢 | Source located in `rc-dotnet/cpu_profile.rs`; repeat real .NET acceptance checks. |
| H-21 | Allocation profiling | ⬜ | Implement with integration and UI acceptance evidence. |
| H-22 | Memory snapshots | 🟢 | Source located in `rc-dotnet/gcdump.rs`; repeat real .NET acceptance checks. |
| H-23 | Profiling comparison | ⬜ | Implement with integration and UI acceptance evidence. |
| H-24 | Application monitoring | 🟢 | Source located in `rc-app/monitor.rs`; repeat application acceptance checks. |
| H-25 | Performance issue detection | ⬜ | Implement with integration and UI acceptance evidence. |
| H-26 | BenchmarkDotNet integration | ⬜ | Implement with integration and UI acceptance evidence. |

## I. Version control and collaboration

Source / reuse boundary: `crates/rc-vcs; crates/rc-app/src/vcs.rs`.

| ID | Capability | Status | Next acceptance step |
| --- | --- | --- | --- |
| I-01 | Git repository discovery | ✅ | Verified by real-Git backend and app VCS tests; scope is local Git workflows on Linux. |
| I-02 | Changed files | ✅ | Verified by real-Git backend and app VCS tests; scope is local Git workflows on Linux. |
| I-03 | Git staging | ✅ | Verified by real-Git backend and app VCS tests; scope is local Git workflows on Linux. |
| I-04 | Git commit | ✅ | Verified by real-Git backend and app VCS tests; scope is local Git workflows on Linux. |
| I-05 | Branches | ✅ | Verified by real-Git backend and app VCS tests; scope is local Git workflows on Linux. |
| I-06 | Side-by-side diff | ✅ | Verified by real-Git backend and app VCS tests; scope is local Git workflows on Linux. |
| I-07 | Gutter change markers | ✅ | Verified by real-Git backend and app VCS tests; scope is local Git workflows on Linux. |
| I-08 | Git blame | ✅ | Verified by real-Git backend and app VCS tests; scope is local Git workflows on Linux. |
| I-09 | Clone repositories | ✅ | [PR 180](https://github.com/javy-fernandez/dotcraft/pull/180) validates local clone and opening a six-project `Shop.sln` from the Git UI, including path-with-spaces and option-shaped source rejection; one rc-app UI and 45 rc-vcs tests pass on Linux. Remote auth, large clones and cancellation remain untested. |
| I-10 | Git read safety | 🚧 | [PR 160](https://github.com/javy-fernandez/dotcraft/pull/160) resolves caller revisions to verified commit hashes for blame and file reads; 46 rc-vcs tests pass. Other Git read operations still need audit. |
| I-11 | Shelves | ✅ | [PR 186](https://github.com/javy-fernandez/dotcraft/pull/186) validates local stash/pop from Git UI on real `Shop.sln`, including unsaved C# editor changes, untracked C# file, save-before-stash and confirmation; targeted app and rc-vcs tests pass on Linux. Restore conflicts and other platforms remain untested. |
| I-12 | History editing | ✅ | [PR 201](https://github.com/javy-fernandez/dotcraft/pull/201) verifies local Amend and normal Commit in the Shop solution egui UI, including a saved C# edit and an excluded file; interactive rebase remains separate I-16 scope. Linux Git fixture only. |
| I-13 | Hosting CLI integration | 🔁 | Inspect sibling implementation, adapt and verify in .NET. |
| I-14 | Git log and graph | ✅ | [PR 165](https://github.com/javy-fernandez/dotcraft/pull/165) draws real branch and merge lanes and supports commit selection; a real local Git merge fixture and egui UI harness passed on Linux. The local log is limited to 500 commits; pagination, huge histories, and other platforms remain untested. |
| I-15 | Fetch pull and push workflow audit | ✅ | [PR 194](https://github.com/javy-fernandez/dotcraft/pull/194) verifies initial Push, peer Fetch/Pull, local Push, ahead/behind, divergent Push rejection and recovery through egui Git UI against a local bare remote with `Shop.sln` on Linux; targeted app and rc-vcs tests pass. Hosted auth, network cancellation and other platforms remain untested. |
| I-16 | Interactive rebase | ⬜ | Implement with integration and UI acceptance evidence. |
| I-17 | Cherry-pick | ✅ | [PR 208](https://github.com/javy-fernandez/dotcraft/pull/208) verifies selected-commit apply, conflict detection and Abort Cherry-pick through the real Shop.sln egui UI on Linux; cancellable backend preflight is tested. Conflict continuation, in-flight cancellation, merge commits and other platforms remain untested. |
| I-18 | Three-way merge editor | ⬜ | Implement with integration and UI acceptance evidence. |
| I-19 | Conflict resolution | 🚧 | PR 8 verifies deleted-side backend resolutions; visual merge tooling remains pending. |
| I-20 | Changelists | ⬜ | Implement with integration and UI acceptance evidence. |
| I-21 | Partial commit | ✅ | [PR 223](https://github.com/javy-fernandez/dotcraft/pull/223) verifies staging one text hunk and committing only the index in real Shop.sln egui UI on Linux; unsaved editor buffers block the operation and other edits stay unstaged. Line UI, binary/rename/concurrent index and other platforms remain untested. |
| I-22 | Git worktrees | ✅ | Existing backend/UI plus [PR 215](https://github.com/javy-fernandez/dotcraft/pull/215) verify create/list, dirty-worktree refusal, active-primary refusal and clean removal in real Shop.sln egui UI on Linux. Locked/symlink/network worktrees and other platforms remain untested. |
| I-23 | Submodules | 🚧 | [PR 158](https://github.com/javy-fernandez/dotcraft/pull/158) adds typed recursive status and cancellable update; 47 rc-vcs tests pass. UI, nested/network cases, and cross-platform acceptance remain. |
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
| I-34 | Local history | 🚧 | [PR 166](https://github.com/javy-fernandez/dotcraft/pull/166) uses exclusive revision creation; 24 concurrent C# revisions are retained in a real .NET fixture, with deduplication and 100-revision retention tested. 13 rc-core tests pass. Live compare/restore UI and concurrent reader behavior remain. |

## J. Web, services and databases

Source / reuse boundary: `crates/rc-syntax; crates/rc-app/src/ui`.

| ID | Capability | Status | Next acceptance step |
| --- | --- | --- | --- |
| J-01 | Razor syntax parsing | 🟢 | Exercise on a real .NET fixture; record evidence. |
| J-02 | XML syntax parsing | 🟢 | Exercise on a real .NET fixture; record evidence. |
| J-03 | JSON syntax parsing | 🟢 | Exercise on a real .NET fixture; record evidence. |
| J-04 | Markdown preview | 🟢 | Exercise on a real .NET fixture; record evidence. |
| J-05 | HTTP client | 🚧 | PRs 13 and 15 verify basic HTTP execution, cancellation and inline variables; advanced HTTP integrations remain pending. |
| J-06 | HTTP environment variables | 🚧 | Inline variables were tested in fork PR 15. [PR 195](https://github.com/javy-fernandez/dotcraft/pull/195) combines parent public and child private/user files with documented precedence; real `Shop.Api.http`/`Shop.sln` integration and 132 rc-dotnet unit tests pass. Environment UI, dynamic values and live requests remain. |
| J-07 | Database connections and SQL console | 🚧 | PR 16 verifies read-only SQLite connections and explicit bounded SQL; writes, persistent sessions and other engines remain pending. |
| J-08 | SQL injection support | 🔁 | Inspect sibling implementation, adapt and verify in .NET. |
| J-09 | ASP.NET endpoints explorer | 🚧 | [PR 184](https://github.com/javy-fernandez/dotcraft/pull/184) excludes nested C#, F#, and VB projects from endpoint scan; Shop.Api still yields four endpoints and 12 rc-dotnet tests pass. Endpoints UI and live server acceptance remain. |
| J-10 | Container tools | 🔁 | Inspect sibling implementation, adapt and verify in .NET. |
| J-11 | ASP.NET MVC navigation | ⬜ | Implement with integration and UI acceptance evidence. |
| J-12 | ASP.NET route analysis | ⬜ | Implement with integration and UI acceptance evidence. |
| J-13 | Blazor component assistance | ⬜ | Implement with integration and UI acceptance evidence. |
| J-14 | JavaScript and TypeScript intelligence | 🚧 | PR 9 verifies JS/TS/TSX syntax, highlighting and folding; semantic language intelligence remains pending. |
| J-15 | HTML and CSS intelligence | 🚧 | PR 9 verifies native HTML/CSS syntax and HTML typing assists; semantic intelligence and preprocessors remain pending. |
| J-16 | React support | ⬜ | Implement with integration and UI acceptance evidence. |
| J-17 | Angular support | ⬜ | Implement with integration and UI acceptance evidence. |
| J-18 | Vue support | ⬜ | Implement with integration and UI acceptance evidence. |
| J-19 | Node.js run and debug | ⬜ | Implement with integration and UI acceptance evidence. |
| J-20 | npm yarn and pnpm | ⬜ | Implement with integration and UI acceptance evidence. |
| J-21 | Browser debugging | ⬜ | Implement with integration and UI acceptance evidence. |
| J-22 | Web test runners | ⬜ | Implement with integration and UI acceptance evidence. |
| J-23 | REST request generation | 🚧 | [PR 207](https://github.com/javy-fernandez/dotcraft/pull/207) converts four scanned Shop.Api endpoints into bounded parseable `.http` requests with method/route/origin checks. UI, disk write, body/auth inference and live send remain. |
| J-24 | OpenAPI support | 🚧 | [PR 213](https://github.com/javy-fernandez/dotcraft/pull/213) exports bounded OpenAPI 3 operations from Shop.Api endpoints and round-trips four requests through the existing importer. Schemas, auth, status inference, UI, file write and live API remain. |
| J-25 | gRPC client | ⬜ | Implement with integration and UI acceptance evidence. |
| J-26 | WebSocket client | ⬜ | Implement with integration and UI acceptance evidence. |
| J-27 | Entity Framework tooling | ⬜ | Implement with integration and UI acceptance evidence. |
| J-28 | Database schema explorer | 🚧 | PR 16 verifies SQLite tables/views and CREATE previews; broader schema navigation and other engines remain pending. |
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
| K-01 | C# language server integration | ✅ | Actual Roslyn server and native application C# flows pass; see live-dotnet.md. |
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
| L-01 | Integrated terminal | ✅ | [PR 173](https://github.com/javy-fernandez/dotcraft/pull/173) verifies a PTY in the real `Shop.sln` workspace cwd, terminal output/exit, and a second egui terminal tab on Linux; 25 rc-term tests and one targeted rc-app UI test pass. SDK commands, other platforms and GPU rendering remain untested. |
| L-02 | Persistent workspace sessions | 🚧 | [PR 159](https://github.com/javy-fernandez/dotcraft/pull/159) hardens concurrent atomic saves and round-trips complete session state in two .NET fixtures; 13 rc-core tests pass. Live UI restoration, crash durability, and other platforms remain unverified. |
| L-03 | File system watching | 🚧 | [PR 155](https://github.com/javy-fernandez/dotcraft/pull/155) classifies renames and atomic saves as source removal plus destination creation; 13 rc-core tests pass, including a real filesystem .NET-project fixture. UI, index/LSP integration, and other platforms remain unverified. |
| L-04 | Background task runtime | 🚧 | [PR 164](https://github.com/javy-fernandez/dotcraft/pull/164) adds `spawn_blocking`; four concurrent `.csproj` reads do not starve an async task, and pending async cancellation is tested. 13 rc-core tests pass. UI callers, process cancellation, and running blocking-task cancellation remain. |
| L-05 | Terminal search and split panes | ⬜ | Search and split panes are absent from original main. The tested search [PR 152](https://github.com/javy-fernandez/dotcraft/pull/152) was closed without merge at the contributor's request. |
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

