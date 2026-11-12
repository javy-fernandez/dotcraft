# Scrollable main menu

The toolbar provides a Main Menu button for implemented DotCraft commands. Its categories, command lists, and tool-window catalog scroll within a limit based on the viewport height. The menu remains reachable in compact windows with a workspace open.

## Behavior

- Open Main Menu, choose a category, and select a command. Use the Main Menu control inside a category to return to the category list.
- Category navigation stays inside one popup. Commands and the Tool Windows catalog use their own scroll positions.
- Scroll to Help to open About DotCraft, or scroll the Run category to Attach to Process.
- View > Tool Windows lists the current `ToolWindow::ALL` catalog. The same implementation works with the baseline catalog and additional HTTP/Database windows when those features are integrated.
- Tools links to NuGet and the existing Tools Settings page for .NET CLI and SDK configuration.
- Build offers the existing solution build, rebuild, clean, and NuGet restore operations.
- Editor commands use the existing dispatch path. Copy and Cut forward platform events and restore editor focus; Paste requests clipboard contents from the platform.
- Actions that require a document, workspace, run configuration, repository, or paused debugger are disabled when their required context is absent.

The project widget, native OS menu, existing run configuration popup, and .NET actions remain available. No Rust-only commands, unsupported engine controls, or SDK-chooser-specific APIs are imported.

## Compact layout adaptation

CrabCraft's source uses viewport-bounded scroll areas with cascading submenus. In a 480-pixel-wide viewport, cascading egui menus can flip over the category list; scrolling can then hover-open another submenu over the intended category. DotCraft uses explicit category navigation inside the same popup, including the tool-window catalog, to avoid that overlap.

The Main Menu button is reserved before the existing toolbar split row measures its right-hand controls. Those controls can otherwise consume the entire left clip rectangle in a compact workspace window and hide the new button.

Popup height is based on the viewport rather than the current popup anchor, avoiding shrinking feedback as egui repositions the popup. This work covers the menu and its entry point; it does not redesign the rest of the compact toolbar.

## Validation

```bash
cargo test -p rc-app --offline ui::main_menu::tests --target-dir /tmp/dotcraft-sdk-validation-target --jobs 2
cargo test -p rc-app -p rc-kit --offline every_text_in_the_code_has_a_translation --target-dir /tmp/dotcraft-sdk-validation-target --jobs 2
```

Six headless tests pass:

1. Scroll the category list to Help and open About at 480×240 and 640×320.
2. Scroll the Run command list to its final Attach item, click it within the viewport, and dispatch the actual process dialog.
3. Load the actual TenProjects .NET solution fixture, verify its ten-project model, scroll the dynamic tool catalog to its last item at 480×240, and open that tool window. The test loads the model directly without starting SDK or language-server processes.
4. Use Copy and Cut through the actual app UI with selected document text, check clipboard output and resulting buffer/caret, and verify that Paste emits a platform clipboard request.
5. Open NuGet and the existing Tools Settings page from Tools.
6. Verify Attach is disabled without a workspace and clicking it does not dispatch a dialog or debugger action.

Tests use native egui interactions without opening an OS window or touching the real system clipboard. Native OS clipboard integration and macOS/Windows menu placement are not established by these Linux headless checks.

Independent PR validation also passed all six menu tests and all five translation tests with `--locked --offline --jobs 2`. The source tree at `/tmp/dotcraft-menu-pr17-source-z6ovefri` was reconstructed from PR #14 commit `d675eafa2a1eb857c89fe43cbf528dd3a63f9eee` and this menu snapshot. Its unique Cargo target was `/tmp/dotcraft-menu-pr17-target-z6ovefri`, shared with no other checkout. This verifies the menu slice without depending on the combined root workspace's SDK, HTTP, SQLite, or EditorConfig changes. The independent commands were:

```bash
cargo test -p rc-app --locked --offline --target-dir /tmp/dotcraft-menu-pr17-target-z6ovefri --jobs 2 ui::main_menu::tests
cargo test -p rc-app -p rc-kit --locked --offline --target-dir /tmp/dotcraft-menu-pr17-target-z6ovefri --jobs 2 i18n::tests
```

## Provenance and retained license

The scroll-area design, category organization, and command-dispatch approach are selectively adapted from [CrabCraft main_menu.rs at commit 475ade7b5ef0020c953f68a51f1b897380c1ea06](https://github.com/RafaelMendes1407/CrabCraft/blob/475ade7b5ef0020c953f68a51f1b897380c1ea06/crates/rc-app/src/ui/main_menu.rs). DotCraft's implementation adapts the compact navigation and retains only implemented .NET IDE actions. No images, icons, or proprietary JetBrains assets are imported by this change.

The retained MIT notice for the adapted portions follows:

```text
MIT License

Copyright (c) 2026 javy-fernandez and the DotCraft contributors
Copyright (c) 2026 the CrabCraft contributors

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

