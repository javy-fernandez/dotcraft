# Single-PR branch consolidation

The feature batch is delivered through [original DotCraft PR 108](https://github.com/javy-fernandez/dotcraft/pull/108).
Its source is `RafaelMendes1407:integration/rider-implementation-batch`; its target is
`javy-fernandez/dotcraft:main`. No additional upstream feature PR is needed for this batch.

## Audit

On November 8, 2026, all 26 published fork branches were fetched. Excluding `main` and the
integration branch itself leaves the 24 source/review branches below. Their feature code
is included in the tested combined tree `891808ad32e441466a5c03458cb36018211ada08`,
published as commit `f6adf62b23c704e646c0aeae387ed8860f3ffe24` before this documentation update.

The source-path and added-function audit found no missing source file or added function
from any of the 24 branches. An independent source review confirmed that all seven
rebased upstream slices are included, with additional changes needed to combine settings,
attachment, EditorConfig, menus, HTTP tools, and the original 0.4.0 implementation safely.
Integration includes adapted source rather than blindly replacing shared files or requiring
every historical branch commit to be a merge parent. Source SHAs below preserve provenance.

| Source branch | Audited head | Delivery |
| --- | --- | --- |
| `docs/rider-feature-backlog` | `9ddea6f0c3a9ccf0841e172327eedde334726f5c` | Included in PR 108 |
| `feat/attach-to-process` | `d675eafa2a1eb857c89fe43cbf528dd3a63f9eee` | Included in PR 108 |
| `feat/editorconfig` | `cbbd4b1534a60660bfe22dc1a8431db1df2a8fd2` | Included in PR 108 |
| `feat/git-stash-management` | `310e0eac5dfe0b58f8f76737ab353a30402e2bcd` | Included in PR 108 |
| `feat/http-client` | `775dd3a85aa4d5978a8fcfafdbf64bc0c9517f6b` | Included in PR 108 |
| `feat/http-file-variables` | `357fa11ff0daec1cd678224fdc82ccb1e3517309` | Included in PR 108 |
| `feat/responsive-settings` | `f097ca790e8081b03b05382004b7f97fc8096ae5` | Included in PR 108 |
| `feat/rider-feature-inventory` | `48a97f3fb4163b84875da52c4565fb0b287cbfbb` | Included in PR 108 |
| `feat/scrollable-main-menu` | `d661714c5ad8f7e37d10a0405c5bc4e0d73f0a74` | Included in PR 108 |
| `feat/sdk-selection` | `df1f3b524a118b66f85b7ad16067119d1c017c88` | Included in PR 108 |
| `feat/settings-search` | `9218232d9c78b964105654ec5323da68118aa2ea` | Included in PR 108 |
| `feat/solution-filters` | `2e01be098e0198a96c855566fff3f1a99dd78d2a` | Included in PR 108 |
| `feat/sqlite-browser` | `5ec8499c0e7e129688ad5e401c99caeb3d0ebaee` | Included in PR 108 |
| `feat/tool-window-overflow` | `35016d5ccb8904f38f5be9b334781403a8918c47` | Included in PR 108 |
| `feat/web-language-syntax` | `0915f2e307ec137138fb1d8d8139dfdc9cdf6eb1` | Included in PR 108 |
| `fix/git-deleted-conflict-sides` | `11b657e2f7dad4b5b7048ac09a48a195057faa5a` | Included in PR 108 |
| `fix/workspace-resource-edit-safety` | `ff4c8734f38ae41a09b79a36a5ba3dd2f0267f39` | Included in PR 108 |
| `upstream/attach-to-process` | `e96b2e1efc43f8fb651366bd161c578e39668dcc` | Included in PR 108 |
| `upstream/git-deleted-conflicts` | `42cd73c311c39a1a76271c0b784ac2b481dd1dad` | Included in PR 108 |
| `upstream/git-stash-management` | `fcae76dc9c5b554b96abb3a958fabfc023b59436` | Included in PR 108 |
| `upstream/settings-search` | `b9056206e47c9fc0681d98a67830150866ff1d7c` | Included in PR 108 |
| `upstream/tool-window-overflow` | `06aba28eaaa1bb3dc3d1d44c42a284aee6099156` | Included in PR 108 |
| `upstream/web-language-syntax` | `bdbad9b89622fc19a3f530c4c1e85768ebe6de23` | Included in PR 108 |
| `upstream/workspace-resource-edit-safety` | `d391cb6b478204e4296d8ed0074d71d6a7a515c3` | Included in PR 108 |

## Validation and remaining scope

This consolidation changes delivery documentation only. Application source remains the
previously tested combined source: 247 library tests, 77 selected application tests,
18 real-Git integration tests, 3 web grammar tests, and 2 solution-filter regressions
passed; all workspace test targets compiled. See [current validation](upstream-integration.md)
for commands and the explicit live .NET, GPU, and platform limitations.

The source branches and their fork PRs remain available as review history. They do not
need separate PRs against the original repository. No default branch is merged or release
published by this consolidation.
