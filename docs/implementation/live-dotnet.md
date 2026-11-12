# Live .NET integration validation

Three Rust integration tests passed against the combined DotCraft source on Linux. They exercised a real Roslyn language server, the app's C# pipeline, and the Shop fixture's build, executable, and test projects. The Shop run executed **13 .NET test cases** across three frameworks; those cases are separate from the **three Rust integration tests** that orchestrated the validation.

## Prerequisites and environment

The validation installation at `/tmp/dotcraft-dotnet` contained SDKs 8.0.427 and 10.0.403, plus .NET and ASP.NET runtimes 8.0.33 and 10.0.14. These runs used the default SDK selection and net8.0 fixtures; they are not a separate test matrix of every installed SDK/runtime combination.

The environment prepended `/tmp/dotcraft-dotnet` to `PATH` and set:

```text
DOTNET_ROOT=/tmp/dotcraft-dotnet
DOTNET_CLI_HOME=/tmp/dotcraft-dotnet-home
NUGET_PACKAGES=/tmp/dotcraft-nuget
CARGO_TARGET_DIR=/home/rafael-mendes/Documentos/dotcraft/target/validation-sdk
```

The first test downloaded Roslyn through the existing `rc_lsp::roslyn::download` bootstrap into the workspace's `target/test-home`. First-use server downloads and NuGet restores require network access. The sandbox initially blocked download DNS and the app fixture's restore; approved execution outside those restrictions passed without source changes. The runtime and server also require working local process/IPC support.

## Commands and results

Run these commands sequentially with the environment above, especially when using shared fixture build outputs:

```bash
cargo test -p rc-lsp --test roslyn --locked --offline --jobs 2 roslyn_completion_rename_and_diagnostics -- --nocapture
cargo test -p rc-app --locked --offline --jobs 2 csharp_insight_end_to_end -- --nocapture
cargo test -p rc-app --locked --offline --jobs 2 shop_builds_runs_and_passes_its_tests -- --nocapture
```

`--offline` applies to Cargo dependency resolution; it does not disable downloads or restores performed by the integration tests.

| Rust integration test | Result | Measured behavior |
| --- | --- | --- |
| `roslyn_completion_rename_and_diagnostics` | 1 passed, 0 failed; 24.22 s | Real two-project `.slnx` load, C# completion, rename edits for both projects, and CS1061 diagnostics. Solution loaded in 1.586 s; first completion took 53.61 ms; warm median was 2.10 ms. |
| `csharp_insight_end_to_end` | 1 passed, 0 failed; 5.00 s | App semantic class coloring, CS0103 diagnostic range, completion popup, rename applied to an open document and a closed file, missing using insertion, and XML documentation skeleton insertion. Completion popup appeared after 101.14 ms. |
| `shop_builds_runs_and_passes_its_tests` | 1 passed, 0 failed; 8.77 s | App loads classic `.sln`, discovers generated app/test run configurations, builds Shop, runs Shop.Cli to exit code 0 with `Total` in its console, and runs all fixture tests with passing outcomes. |

The timings describe this run, rather than performance guarantees.

## Fixture test results

The final Shop run retained these TRX results:

| Framework | Passed | Failed | Artifact |
| --- | --- | --- | --- |
| MSTest | 4 | 0 | `/tmp/dotcraft-tests-1reV09/0-Shop.Core.MSTests/results.trx` |
| NUnit | 4 | 0 | `/tmp/dotcraft-tests-1reV09/1-Shop.Core.NunitTests/results.trx` |
| xUnit | 5 | 0 | `/tmp/dotcraft-tests-1reV09/2-Shop.Core.XunitTests/results.trx` |
| Total | 13 | 0 | Three framework result files |

Tool output established the Rust test results; the TRX counters independently record the fixture case counts. No complete raw stdout log was retained. A recorded result summary is available at `/tmp/dotcraft-live-dotnet-evidence.json`; it is a summary rather than a raw execution transcript. Temporary paths are local validation artifacts and may disappear after cleanup.

## Scope

These checks support the tested C# semantic coloring, completion, editor diagnostics, symbol rename across projects, C# language-server integration, solution loading, Shop build/run flow, and xUnit/NUnit/MSTest execution with TRX results. They also verify automatic using insertion and documentation skeleton insertion for the tested examples.

They do not establish F# or VB.NET intelligence, every refactoring or code action, signature help, hover documentation, inlay hints, test discovery/result-tree/gutter presentation, all launch-profile variants, other operating systems, or Rider parity. Existing debugger and SDK-selection checks have their own evidence. No source changes were required to make these three flows pass.
