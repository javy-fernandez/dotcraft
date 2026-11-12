# Editor tab docking guides

## Behavior

Dragging a tab over an unsplit editor with another available document shows a
bounded destination map, a half-pane or full-pane preview, and a file card.
Dropping near an edge creates DotCraft's existing two-pane split in the shown
orientation and order. Dropping in the center activates the dragged document.
Dropping on the tab strip continues to reorder tabs, including the existing
pinned-tab boundary. Escape cancels a drag.

The preview is limited to destinations that this layout can represent. When a
split already exists, tab dragging keeps its existing reorder behavior; the
multi-group docking model in CrabCraft is not present in DotCraft.

## Provenance

The destination map and visual treatment are adapted from CrabCraft commit
`8ee8d1d` (`crates/rc-app/src/ui/editor_area.rs`), introduced by `95e6a42` and
refined by `0584222`. CrabCraft is MIT OR Apache-2.0; DotCraft already retains
the CrabCraft MIT notice in `docs/third-party/CrabCraft-LICENSE-MIT.txt`. No
images or other assets are copied. DotCraft's document model and .NET behavior
remain unchanged.

## Validation

Focused tests check guide geometry and hit targets at narrow and standard
sizes, plus each edge's split orientation, pane order, and active document on
the Shop C# fixture. The inherited split regression also passed:

```sh
CARGO_PROFILE_TEST_DEBUG=0 cargo test -p rc-app ui::editor_area::split_tests --locked --offline --target-dir target/validation-visual-docking --jobs 2
```

Result: 3 passed, 0 failed. `git diff --check` also passed.

## Remaining limitations

Nested docking, moving tabs between existing split panes, and floating-window
drop targets require a richer layout model. Live GPU captures and platform
checks remain pending.
