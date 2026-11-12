# I-34: local history under concurrent saves

## Behavior

Local History creates revision files exclusively. If another save already claimed the same millisecond and revision number, the writer tries the next number instead of overwriting that revision. A failed write removes its partial revision file. Existing duplicate suppression and retention remain in place.

## Validation

- `CARGO_TARGET_DIR=/tmp/dotcraft-core-history-target cargo test -p rc-core` — 13 passed on Linux.
- A .NET project fixture containing `App.csproj` and `Program.cs` received 24 concurrent, distinct C# revisions. All 24 contents were present and readable afterward.
- A retention test recorded 101 revisions and confirmed that the newest 100 remained while the oldest was pruned.
- Existing duplicate suppression and per-file isolation tests passed.
- `git diff --check` passed.

## Provenance and limits

This extends DotCraft's existing local-history implementation from upstream commit `becfb0fe316203df2d8d7c1e99c6a54bd37cd13b`; no external source or assets were copied. Live compare and restore UI flows were not exercised. A revision becomes visible when its file is created, before its write finishes, so a concurrent reader can briefly observe incomplete content. This is a tested I-34 storage scope, not full Rider parity.
