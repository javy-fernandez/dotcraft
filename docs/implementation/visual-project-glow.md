# Project identity glow

## Behavior

When a workspace is open, the title bar draws a soft color wash behind the
project widget. The color follows the existing project badge selection and
fades toward the run controls. The no-workspace title bar keeps its current
appearance.

## Provenance

The egui mesh was adapted from CrabCraft commit
`95e6a4200da83f009b14f431216446202e82414b` at repository commit
`8ee8d1d747bc009f951a5551d9eff6da705c7327`,
`crates/rc-app/src/ui/header.rs`. CrabCraft is licensed MIT OR Apache-2.0;
DotCraft's CrabCraft MIT notice is in
`docs/third-party/CrabCraft-LICENSE-MIT.txt`. No bitmap or font asset is copied.

## Validation and limits

`cargo check -p rc-app --locked --offline --target-dir
target/validation-visual-glow --jobs 2` passed on Linux with Rust 1.99.1.
`git diff --check` also passed. The effect uses the same project-name hash as
the badge and adds no process or I/O work to the UI thread. A captured visual
comparison on Windows and macOS is pending.
