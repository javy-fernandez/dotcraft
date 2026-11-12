# L-02: persistent workspace sessions under concurrent saves

## Behavior

DotCraft now writes each session or settings update through a uniquely named temporary file in the destination directory. A concurrent save cannot overwrite another writer's temporary data. A failed write or rename removes its temporary file. Successful saves still replace the destination through a same-directory rename.

## Validation

- `CARGO_TARGET_DIR=/tmp/dotcraft-core-session-target cargo test -p rc-core` — 13 passed on Linux.
- Twelve concurrent writers saved distinct 64 KiB payloads to one destination. The final content matched one complete payload, and the directory contained no temporary files.
- A failed replacement of a directory cleaned up its temporary file.
- Two independent .NET project fixtures retained distinct session files. Editor position, favorites, bookmarks, tool-window layout, run options, and templates round-tripped in the first fixture.
- `git diff --check` passed.

## Provenance and limits

This change extends DotCraft's existing `rc-core` persistence from upstream commit `becfb0fe316203df2d8d7c1e99c6a54bd37cd13b`. No external source or assets were copied. Live UI session restoration, process-crash durability, and Windows/macOS behavior were not exercised. This is a tested L-02 persistence scope, not full Rider parity.
