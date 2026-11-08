# Upstream 0.4.0 integration

This integration resolves the overlap between the fork batch at `da51a73` and original
DotCraft `main` at `9e849cda99043fce68e8896d64a2608ff78141c8`. Delivery is
[upstream PR 108](https://github.com/javy-fernandez/dotcraft/pull/108). The original default
branch is not modified by preparing or updating this PR.

## Conflict decisions

- Document collections retain the background EditorConfig worker together with upstream
  bookmark storage and opt-in Local History. Production initialization uses the repaint
  waker and enables history; test collections keep history disabled.
- Solution filters keep their source-directory workspace and separate filter watcher.
  The same workspace watcher setup also starts upstream TODO scanning.
- All upstream coverage, monitoring, Endpoints, HTTP Client, hierarchy, bookmark, template,
  update-check, and history code is retained, including existing GPU captures.
- The fork's bounded HTTP workflow is explicitly named **HTTP Scratchpad** and has its own
  state. The upstream HTTP Client retains gutter execution, environments, dynamic/file
  variables, and Endpoints integration. Scratchpad Send/Cancel does not mutate its state.
  Closing a workspace resets both clients. No shared product field is replaced with a
  different client type.
- Tool-window registration contains all 23 tools, unique persistent IDs, and distinct
  HTTP window names. Native egui menus expose the new upstream bookmark/hierarchy/template
  actions and Run All Tests with Coverage as well as the existing fork actions.
- Global translation keys are deduplicated across areas, preserving upstream HTTP Client translations rather than the fork fallback. The scratchpad TSV uses its distinct name.
- Cargo retains the upstream version and dependencies, with one app Tokio entry and one
  Reqwest entry whose features are the union of the two implementations. Locale catalogs
  preserve upstream translations and add explicit English fallback for the scratchpad name.

## Current validation

Validation uses an isolated source checkout `/tmp/dotcraft-upstream-integration` and unique
Cargo target `/home/rafael-mendes/Documentos/dotcraft/target/upstream-integration`.
Fresh combined results on Linux / Rust 1.99.1: all workspace test targets compiled, 247 library tests passed across 11 crates, 77 selected application tests, 18 real-Git integration tests, 3 web grammar tests, and 2 solution-filter screen regressions passed. The library group includes five translation invariants and the new 19-test DAP suite. Historical fork counts in
[validation.md](validation.md) remain historical and are not evidence for this merge.

A merge regression checks EditorConfig indentation and upstream mnemonic bookmarks across
close/reopen. HTTP UI coverage checks the independent scratchpad client and workspace reset.
The independent web, Git, and attach slices also keep their own source checkouts and targets. The combined source includes multiline JSX folding from `5fd8403`, Git-operation cancellation from `73d3a43`, and detach-target survival coverage from `bb9f12f`.

## Provenance and limitations

Fork feature PRs and CrabCraft source commits/licenses remain in their feature notes and
retained MIT notices. Original upstream changes remain in merge history. No proprietary
Rider source or assets are copied. The application remains native Rust/egui.

A complete live .NET suite, GPU captures, and Windows/macOS execution require their own
host prerequisites. Source presence in upstream does not establish verified Rider parity.
FEATURES.md records newly located upstream capabilities as existing/partial; its summary
is regenerated without promoting historical verified entries to broader tested scope.

## Independent upstream review branches

Each branch below is based directly on original `main` at `9e849cd`, with an identical
published Git tree to its tested isolated checkout. All remain separate from the default
branch. Their feature notes record commands, source commits, licenses, and remaining scope.

| Branch in RafaelMendes1407/dotcraft | Fresh isolated validation |
| --- | --- |
| `upstream/git-deleted-conflicts` | 21 VCS unit and 18 real Git tests |
| `upstream/git-stash-management` | 4 app VCS and 5 translation tests |
| `upstream/workspace-resource-edit-safety` | 2 app, 7 LSP, and 5 translation tests |
| `upstream/settings-search` | 2 headless app and 5 translation tests |
| `upstream/web-language-syntax` | 6 syntax unit, 3 grammar, and 1 editor test |
| `upstream/attach-to-process` | 19 DAP, 7 attachment app, and 5 translation tests |
| `upstream/tool-window-overflow` | 1 headless stripe and 5 translation tests |

These counts overlap by design and should not be summed as unique test cases. Source
branch publication is separate from PR creation; original-repository PR creation requires
GitHub access for that repository. Updating the fork head updates the existing PR 108.

Reproduce the combined checks with:

```sh
cargo test --workspace --locked --offline --no-run --target-dir /home/rafael-mendes/Documentos/dotcraft/target/upstream-integration --jobs 1
cargo test --workspace --lib --locked --offline --target-dir /home/rafael-mendes/Documentos/dotcraft/target/upstream-integration --jobs 1 -- --test-threads=1
cargo test --workspace --bins --locked --offline --target-dir /home/rafael-mendes/Documentos/dotcraft/target/upstream-integration --jobs 1 -- --skip insight_tests:: --skip runner_tests:: --skip debugger_tests:: --skip screens:: --test-threads=1
cargo test --workspace --test repo --test web --locked --offline --target-dir /home/rafael-mendes/Documentos/dotcraft/target/upstream-integration --jobs 1 -- --test-threads=1
cargo test --workspace --bins solution_filter_ --locked --offline --target-dir /home/rafael-mendes/Documentos/dotcraft/target/upstream-integration --jobs 1 -- --test-threads=1
```

The application selection explicitly excludes runtime integration modules and screen
captures; it does not claim execution of the entire workspace suite. Local HTTP sockets
and subprocesses require host permissions outside the restricted sandbox. The locale
merge first exposed a globally duplicated HTTP Client key; the corrected catalogs passed
all library checks and preserve every pre-existing upstream translation value. No .NET
SDK is available in this session, so earlier live SDK evidence remains historical.
