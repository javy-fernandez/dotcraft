# Solution filters

DotCraft loads JSON `.slnf` solution filters through the implementation in existing
[PR 2](https://github.com/RafaelMendes1407/dotcraft/pull/2), commit
`4cca4ff63aa65c28cebac79608592fd2422f2b8d`, with follow-up workspace and SDK-context fixes.
The loader resolves the referenced `.sln` or `.slnx` and displays selected projects in the
solution explorer.

```json
{
  "solution": {
    "path": "../Example.slnx",
    "projects": ["src/Example.Api/Example.Api.csproj"]
  }
}
```

The solution path is relative to the filter file. Project paths are relative to the
referenced solution, with Windows path separators normalized on other hosts. Absolute
paths are supported according to the host platform. The filter remains the loaded
solution's path and supplies its display name, preserving it as the command target.

## Implemented scope

- Load selected projects from classic `.sln` and XML `.slnx` solutions.
- Retain selected projects in their original solution order and folder hierarchy.
- Remove folders emptied by filtering while preserving loose solution files.
- Avoid duplicating projects when the filter repeats an entry; accept an empty selection.
- Return errors for malformed JSON, missing required fields, absent source solutions,
  unsupported source extensions, and projects outside the referenced solution's membership.
- Render selected real projects in the native explorer without requiring an SDK or
  language-server process for the regression test.
- Route `.slnf` through normal file, drop, recent-workspace, and solution-picker opening.
- Keep the filter as session/recent identity and build/test target, while using the source
  solution's directory for file indexing, recursive watching, Git, and the language-server root.
- Watch an external filter's directory nonrecursively. Changes to either the filter or source
  solution refresh the filtered model and run configurations; filter changes restart its language session.
- Retarget index/watchers when a filter points to another source directory, preserving open
  documents and unsaved text.
- Run filter builds from the underlying solution directory so CLI SDK selection follows its
  `global.json`. The filter remains an absolute CLI target even when requested using a relative path.

The referenced source must be `.sln` or `.slnx`, preventing recursive filter loading.
Loading the solution/filter model reads files and does not itself execute project code.
Path normalization is lexical; files outside the filter directory are valid references.

## Validation

The PR2 source and scoped follow-up changes were tested independently on pristine DotCraft
commit `f30ac7180ce8dd492633235f3c3dbf2f4107e61b`, using a dedicated Cargo target directory:

```sh
cargo test -p rc-dotnet -p rc-core --lib --locked --offline --jobs 2
cargo test -p rc-app solution_filter_ --locked --offline --jobs 2 -- --nocapture
cargo test -p rc-dotnet --test solution_filter --locked --offline --jobs 2 -- --ignored --nocapture
```

Results: 47 library tests, two headless application tests, and one actual SDK build test passed.
The SDK test requires an installed SDK and net8.0 reference pack. Set `DOTCRAFT_TEST_DOTNET`
to the dotnet executable when it is outside standard locations. The library regressions
cover relative filter/source paths, Windows separators, `.slnx`, invalid inputs, duplicate
and empty selections, and real TenProjects fixture metadata. The explorer test displays
Module01 and Module10 and excludes Module02 and Module09 from that fixture.

`solution_filter_open_route_uses_source_root_and_watches_both_locations` exercises the real
`open_path` route with the filter outside its source directory. It verifies file index and
Git roots, source file notifications, filter selection changes, source solution reloads,
source-root retargeting, unchanged filter identity, and survival of unsaved editor text.

`solution_filter_builds_only_selected_projects_from_an_external_directory` uses a private
fixture and the product's `start_build` path. A selected valid class library builds while
an unrelated excluded project containing invalid C# does not. The source has a valid pinned
SDK policy and the filter directory deliberately has an unusable policy, proving the CLI
uses the source working directory. Package sources are cleared, and shared Shop outputs are
never touched.

## Remaining scope

Filter-specific Roslyn project loading and diagnostics, actual unit-test execution, and
cross-platform filesystem case/path behavior on Windows/macOS remain unvalidated. The
headless opening test intentionally uses a non-executable language-server placeholder.
Test discovery's separate `vstest` invocation retains its existing working-directory policy.

File indexing covers the underlying source tree, rather than only selected project files.
MSBuild owns referenced-project dependency behavior; the build regression excludes an
unrelated project without references and does not establish dependency-closure policy.
