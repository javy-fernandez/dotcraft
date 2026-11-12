# C-18: EditorConfig numeric-range globs

## Behavior

EditorConfig section names now expand inclusive integer ranges such as `Generated{1..3}.cs` and `Compat{-2..1}.cs` before glob matching. Escaped braces and character classes retain their literal meaning. Invalid descending ranges do not match numeric file names. Expansion is limited to 256 generated values across one pattern to bound work on background configuration reads.

The range behavior follows the [EditorConfig specification](https://spec.editorconfig.org/#glob-expressions), which requires the first integer to be less than the second and allows negative endpoints.

## Validation

- `CARGO_TARGET_DIR=/tmp/dotcraft-core-editorconfig-target cargo test -p rc-core` — 13 passed on Linux.
- Matching tests cover inclusive endpoints, negative values, nested alternatives, escapes, character classes, invalid descending ranges, and the 256-value bound.
- A real .NET project fixture resolves rules for `Generated2.cs`, `Generated4.cs`, and `Compat-1.cs` from a `.editorconfig` file next to `App.csproj`.
- `git diff --check` passed.

## Provenance and limits

This change extends DotCraft's existing EditorConfig parser from upstream commit `becfb0fe316203df2d8d7c1e99c6a54bd37cd13b`. The parser's earlier glob normalization cites CrabCraft's `code_style.rs` under MIT OR Apache-2.0; no CrabCraft code or assets were copied in this change. Ranges wider than 256 values are intentionally ignored. Save-time formatting, complete EditorConfig property coverage, and live editor UI behavior remain unverified.
