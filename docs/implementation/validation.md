# Validation evidence

The implementation batch is validated on Linux with Rust 1.99.1. Each independent feature snapshot uses its own Cargo target directory. The combined working tree uses `target/validation-sdk`; the temporary `/tmp/dotcraft-sdk-validation-target` path points to that directory. Keeping local-crate artifacts separate across checkouts avoids stale dependency reuse.

## Combined implementation checks

All workspace test targets compiled successfully with:

```sh
cargo test --workspace --locked --offline --no-run --target-dir target/validation-sdk --jobs 2
```

The following test groups passed on the combined implementation:

| Group | Passed | Scope |
| --- | ---: | --- |
| Core, text, syntax, .NET, DAP, LSP, search, and VCS libraries | 169 | Library unit tests |
| Application | 69 | Native UI, documents, HTTP, SQLite, Git, settings, menus, startup/session handling, and solution-filter lifecycle |
| Real Git repositories | 18 | Repository integration tests |
| Web syntax | 3 | HTML, CSS, JavaScript, TypeScript, JSX, and TSX integration tests |
| Editor views | 3 | HTML typing, mixed indentation, and snippet behavior |
| Translation invariants | 5 | Locale coverage and formatting checks |

Reproduce the groups with:

```sh
cargo test -p rc-core -p rc-text -p rc-syntax -p rc-dotnet -p rc-dap -p rc-lsp -p rc-search -p rc-vcs --lib --offline --target-dir target/validation-sdk --jobs 2
cargo test -p rc-app --locked --offline --target-dir target/validation-sdk --jobs 2 -- --skip insight_tests:: --skip runner_tests:: --skip debugger_tests:: --test-threads=1
cargo test -p rc-vcs --test repo --offline --target-dir target/validation-sdk --jobs 2
cargo test -p rc-syntax --test web --offline --target-dir target/validation-sdk --jobs 2
cargo test -p rc-app -p rc-editor view:: --offline --target-dir target/validation-sdk --jobs 2
cargo test -p rc-app -p rc-kit i18n::tests --offline --target-dir target/validation-sdk --jobs 2
```

The application command excludes the three runtime integration modules, which have separate live evidence below. Six GPU capture tests remain ignored by default. The final combined application run passed all 69 selected tests; its localhost HTTP checks required execution outside sandbox socket restrictions. This is not a claim that the full workspace test suite has executed.

## Live .NET checks

- [Debugger attachment](attach-debugger.md): the real ASP.NET fixture remains alive and serves HTTP 200 after the IDE detaches. The inherited CLI/API debugger regression also passed.
- [SDK selection](sdk-selection.md): real SDKs 8.0.427 and 10.0.403 were selected through global.json, and a copied net8.0 project built under each without external package sources.
- The real Roslyn integration `roslyn_completion_rename_and_diagnostics` passed completion, diagnostics, and cross-project rename. The native application regression `csharp_insight_end_to_end` also passed semantic colors, CS0103 diagnostics, completion, rename, and automatic usings. First-use language-server download and fixture restore required network-enabled execution; Cargo dependency resolution remained `--locked --offline`.
- The Shop build/run regression passed with 5 xUnit, 4 NUnit, and 4 MSTest fixture cases. These are 13 .NET cases inside one Rust integration test, separate from the library/application counts above. [Live .NET evidence](live-dotnet.md) records commands, prerequisites, assertions, and result-artifact paths for all three runtime flows.

The SDKs were installed temporarily under `/tmp/dotcraft-dotnet`, with separate temporary CLI-home and NuGet directories. Native product settings and the host PATH were not changed globally.

## Independent feature snapshots

Feature notes contain exact commands, scope, and remaining limitations. Settings search, workspace-edit safety, stashes, web syntax, attachment, solution filters, SDK selection, and HTTP slices were also checked independently of unrelated work. Final EditorConfig and SQLite snapshots passed `--locked --offline` checks in separate dedicated workspace targets.

- [EditorConfig](editorconfig.md): 10 core, 26 text, 8 document tests, and application/editor mixed-indentation regressions.
- [SQLite](sqlite-browser.md): 7 database tests, 5 translation checks, and 2 dock tests, with only the eight SQLite dependency additions in its lockfile.
- [Responsive Settings](responsive-settings.md): the existing PR 4 production layout is preserved; its original tests and three added regressions all pass.
- [Scrollable main menu](scrollable-menu.md): six headless regressions cover compact navigation, the actual TenProjects model, clipboard dispatch, and context guards.
- [Solution filters](solution-filters.md): 47 library tests, two headless application tests, and a real SDK build passed in an independent snapshot. The external filter's unusable SDK policy proves the product build uses the source solution directory.

## Delivery and remaining scope

The detailed status record is [FEATURES.md](../../FEATURES.md); [the implementation checklist](../features.md) links the feature PRs. Verified entries describe their stated tested scope in delivery branches. The default branch remains unchanged until the PRs are reviewed and merged.

PR 5 depends on the responsive Settings branch, PR 15 depends on HTTP Client PR 13, SQLite PR 16 depends on status/documentation PR 6, and main menu PR 17 depends on attachment PR 14. The metadata branch builds on inventory PR 1. HTTP and SQLite both extend the app state and tool-window catalog; combined code retains both tools and a 15-entry `ToolWindow::ALL` array.

Coverage, profiling, advanced refactoring, other languages and database engines, game engines, containers, remote development, plugins, AI integrations, and the remaining inventory entries still require implementation or additional validation. Windows, macOS, and minimum-Rust-version execution were not validated by this Linux batch.
