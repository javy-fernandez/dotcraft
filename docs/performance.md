# Editor performance (phase 0 go/no-go)

Measured with `cargo bench -p rc-syntax` and `cargo bench -p rc-editor --bench frame` on an
Apple Silicon Mac, on a generated 20,000-line C# file (`rc_syntax::sample::big_csharp`).
Frame times are CPU time of a full egui frame of the editor (layout, painting and
tessellation) at 1400×900.

| Measure | Result | Target |
| --- | --- | --- |
| Frame while scrolling (3 lines per frame) | 0.37 ms | < 8 ms |
| Frame after a far jump (every visible line new) | 0.89 ms | < 8 ms |
| Frame while typing (edit + re-highlight) | 1.3 ms | < 8 ms |
| Idle frame (caret blink) | 0.30 ms | — |
| Incremental reparse after a keystroke (background) | 0.41 ms | < 5 ms |
| Full parse (first open, background) | 100 ms | — |
| Highlight 80 visible lines | 0.25 ms | — |
| Folding ranges (background, after each parse) | 14 ms | — |

Result: **go** — egui with a custom editor widget is well within budget; no need for GPUI.
