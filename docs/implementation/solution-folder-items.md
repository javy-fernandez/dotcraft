# B-05: solution folders and loose items

## Behavior

The classic `.sln` parser now accepts loose files only within `ProjectSection(SolutionItems)`. Other project sections and stray `name = value` metadata inside a solution folder no longer appear as fake files. Nested folder and project membership remains unchanged. XML `.slnx` folder parsing is covered by the same acceptance suite.

## Validation

- `CARGO_TARGET_DIR=/tmp/dotcraft-solution-items-target cargo test -p rc-dotnet --test solution_folders` — 2 passed on Linux.
- `CARGO_TARGET_DIR=/tmp/dotcraft-solution-items-target cargo test -p rc-dotnet solution::tests` — 5 solution parser tests passed.
- Real Shop `.sln` fixture has `src` and `tests` folders with three existing projects each. Real TenProjects `.slnx` fixture has one `src` folder with ten existing projects.
- A classic `.sln` fixture with a nested folder, a legitimate README solution item, a separate metadata section, and stray metadata retains only the legitimate file and project.
- `git diff --check` passed.

## Provenance and limits

This extends DotCraft's existing solution parser at upstream commit `becfb0fe316203df2d8d7c1e99c6a54bd37cd13b`; no external source or assets were copied. Parser behavior for the tested classic and XML solution folders is validated. The application explorer UI and SDK build/run were not exercised, so this does not establish full Rider parity.
