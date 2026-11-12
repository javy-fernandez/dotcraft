# B-01: solution folders and bounded directory links

## Behavior

The workspace tree treats a symbolic link to a directory as navigable when its canonical target is strictly below the directory currently being listed. Links to ancestors and outside directories remain non-navigable, preventing this tree view from recursing through cycles or walking outside the listed subtree. A `.git` link remains hidden even when it points outside the workspace. The existing solution-folder selection behavior is retained.

## Validation

- `CARGO_TARGET_DIR=/tmp/dotcraft-core-workspace-tree-target cargo test -p rc-core` — 13 passed on Linux.
- DotCraft's real `tests/fixtures/Shop` .NET solution opened as `Shop.sln`; the root listed `src`, `tests`, and the solution, and the `Shop.Api` project file was reachable through the tree.
- A Unix fixture showed an internal directory alias as navigable and its C# source visible. Links back to the root and to an external directory were non-navigable. A `.git` link was hidden in normal and Show All Files listings.
- `git diff --check` passed.

## Provenance and limits

This extends DotCraft's existing workspace tree at upstream commit `becfb0fe316203df2d8d7c1e99c6a54bd37cd13b`; no external source or assets were copied. Symlink navigation was tested on Unix only. Links to sibling directories are intentionally non-navigable because this API has no workspace-root parameter. The application UI, SDK build/run, and other operating systems were not exercised. This is a tested B-01 workspace-tree scope, not full Rider parity.
