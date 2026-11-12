# Native application window controls

## Behavior

On Linux and Windows, DotCraft removes the operating system title bar and
places Close, Maximize/Restore, and Minimize controls in its existing header.
The header remains draggable and double-clicking it toggles maximization.
Border resize handles remain available when the window is neither maximized
nor full screen. macOS keeps its native traffic lights over the full-size
content view, matching CrabCraft's platform behavior.

## Provenance

The platform gates are adapted from CrabCraft commit
`ff3c5c322f918a4a1413cc6fcc0bb2cb87d79b1c`, inspected at
`8ee8d1d747bc009f951a5551d9eff6da705c7327`. CrabCraft is licensed
MIT OR Apache-2.0; DotCraft already carries its contributor notice in
`docs/third-party/CrabCraft-LICENSE-MIT.txt`. No source asset is copied.
DotCraft's existing caption widgets, translations, and .NET behavior are reused.

## Validation and limits

The platform viewport test and existing button/resize tests passed on Linux
with Rust 1.99.1:

```sh
env CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0 \
  cargo test -p rc-kit --features eframe/x11,eframe/wayland window_chrome::tests \
  --locked --offline --target-dir target/titlebar-validation --jobs 2
```

Result: 3 passed, 0 failed. A Linux launch verifies that the undecorated
window opens.
Windows and macOS window-manager behavior remains to be checked on those
platforms. Focus modes intentionally hide the header; the existing exit-focus
control and Escape path remain available.
