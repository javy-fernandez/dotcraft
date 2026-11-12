# Bundled CJK fallback font

## Behavior

DotCraft embeds Noto Sans CJK SC Regular after Inter and JetBrains Mono in
the proportional, monospace, bold, and italic font families. Chinese and
Japanese interface and editor text remains readable when the host has no CJK
font. Other script fallbacks may still use the installed system fonts.
The About dialog exposes the font license.

## Provenance and licensing

The unmodified `NotoSansCJKsc-Regular.otf` is version 2.004, copyright
© 2014–2021 Adobe, licensed under the SIL Open Font License 1.1. It comes
from [notofonts/noto-cjk commit f8d157532fbfaeda587e826d4cd5b21a49186f7c](https://github.com/notofonts/noto-cjk/blob/f8d157532fbfaeda587e826d4cd5b21a49186f7c/Sans/OTF/SimplifiedChinese/NotoSansCJKsc-Regular.otf).
SHA-256: `2c76254f6fc379fddfce0a7e84fb5385bb135d3e399294f6eeb6680d0365b74b`.
The full license is `assets/fonts/NotoSansCJK-LICENSE.txt` and is copied into
Linux, macOS, and Windows release packages.

The embedding pattern and glyph test were adapted from CrabCraft commit
`8ceb070154eea3adfd37d849da82a6cea65c95cf` at CrabCraft `8ee8d1d`,
with release notice handling informed by `e4e4f5f3cfd32a4162f714d7588be533daa80a74`.
CrabCraft is MIT OR Apache-2.0; DotCraft retains its contributor notice in
`docs/third-party/CrabCraft-LICENSE-MIT.txt`.

## Validation and limitations

The glyph test creates font definitions without system fallbacks and checks
all five font families with the supported interface scripts:

```sh
env CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0 \
  cargo test -p rc-kit --features eframe/x11,eframe/wayland \
  embedded_fonts_cover_every_interface_language_without_system_fonts \
  --locked --offline --target-dir target/validation-visual-cjk --jobs 2
```

Result: 1 passed, 0 failed on Linux with Rust 1.99.1. A native application
check also passed:

```sh
env CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0 \
  cargo check -p rc-app --locked --offline \
  --target-dir target/validation-visual-cjk --jobs 2
```

The font SHA-256 matches the source. All seven locale JSON files parsed and
the five `rc-kit` i18n tests passed with:

```sh
env CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0 \
  cargo test -p rc-kit --features eframe/x11,eframe/wayland i18n::tests \
  --locked --offline --target-dir target/validation-visual-cjk --jobs 2
```

`git diff --check` and `bash -n scripts/bundle.sh` passed.
A full release package build and visual checks on Windows and macOS remain
pending. The embedded font adds 16,437,364 bytes to the source and binary
before packaging compression.
