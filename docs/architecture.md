# DotCraft architecture

DotCraft is a Cargo workspace (`crates/*`, Rust edition 2024) for a native .NET IDE built
with eframe/egui 0.36 + wgpu 30. The UI thread never blocks: background work runs on threads
or on the shared tokio runtime and reports back through channels that wake the UI.

| Crate | Role |
| --- | --- |
| `rc-app` (bin `dotcraft`) | Window, tool windows, editor tabs, keymaps, session. Only crate that wires everything together. |
| `rc-kit` | Islands theme (`theme`), New UI icons (`icons`, SVGs in `assets/icons`), window chrome, branding, i18n (`i18n::tr`). |
| `rc-core` | Shared tokio `Runtime`, `Inbox`/`Outbox`/`Waker`, settings, per-workspace session, workspace file tree, file watcher. |
| `rc-text` | Rope buffer, change sets, multi-cursor selections, grouped undo, UTF-16 positions. No UI. |
| `rc-syntax` | tree-sitter parsing on a background thread, highlighting, folding, bracket matching. |
| `rc-editor` | The code editor widget (egui), keymaps (`keymap::Action`), code-insight popups. |
| `rc-lsp` | JSON-RPC/LSP client and the Roslyn adapter. |
| `rc-dap` | DAP client and netcoredbg session. |
| `rc-dotnet` | SDKs, solutions/projects, MSBuild, build/run/test, TRX, NuGet. |
| `rc-vcs` | Git through the `git` CLI. |
| `rc-search` | File index, fuzzy search, find/replace in files. |
| `rc-term` | Terminal emulator (alacritty_terminal + portable-pty) rendered in egui. |

## Conventions

- **Threads.** Never block the UI thread on I/O or processes. Long work runs on
  `rc_core::Runtime::spawn` (tokio) or a dedicated thread and sends results through an
  `rc_core::Outbox<T>` (its `send` wakes the UI with `request_repaint`). The UI drains its
  `rc_core::Inbox<T>` once per frame.
- **Cancellation.** Every long operation (build, test run, search, LSP request) can be
  cancelled; dropping a handle or calling `cancel()` must stop the process / task.
- **Text.** Everything user-facing goes through `rc_kit::i18n::tr` / `trf` (English source
  text). Translations live in `locales/*.json`, imported from `scripts/i18n/*.tsv` with
  `python3 scripts/i18n_tsv.py <area> scripts/i18n/<file>.tsv` (columns: en, es, fr, pt-BR,
  de, pl, zh-Hans, ja). The test `every_text_in_the_code_has_a_translation` fails on any
  missing entry.
- **Look.** Colours come from `rc_kit::pal(ui)` (UI) and `rc_kit::theme::scheme(dark)`
  (editor/terminal), icons from `rc_kit::ico::*` painted with `rc_kit::icons::paint` (16 pt,
  pixel-snapped). Row height 24 pt for trees/lists, 36 pt tool window headers, 13 pt text.
- **Tests.** No synthetic input to real windows. UI is tested with `egui_kittest` (headless;
  GPU captures are `#[ignore]` tests writing to `target/snapshots/`). Integration tests run
  against the real `dotnet` SDK (8.0 and 10.0 installed), Roslyn and netcoredbg, with the
  fixtures in `tests/fixtures/` (`Shop`: console, class library, ASP.NET API and xUnit /
  NUnit / MSTest projects; `TenProjects`: ten libraries in a `.slnx`).
- **Commits.** One commit per finished task, short one-line Spanish messages, capitalized
  (e.g. "Añade parser de TRX"), no attribution trailers.

## Packaging

Everything the app needs (fonts, icons, translations) is embedded in the binary.
`scripts/bundle.sh` builds `DotCraft.app` (`--universal`, `--dmg`, `--zip`) and the Windows
x64/ARM64 executables with cargo-xwin (`--windows`, DirectX 12, statically linked C runtime,
icon from `assets/icon.ico` via `crates/rc-app/build.rs`). Application icons are generated
from `assets/icon.svg` by `scripts/make_icon.py`. The Roslyn language server and netcoredbg
are not bundled: they are located or downloaded on first use into the data folder.
