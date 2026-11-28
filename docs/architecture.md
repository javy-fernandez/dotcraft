# DotCraft architecture

DotCraft is a Cargo workspace (`crates/*`, Rust edition 2024) for a native .NET IDE built
with eframe/egui 0.36 + wgpu 30. The UI thread never blocks: background work runs on threads
or on the shared tokio runtime and reports back through channels that wake the UI.

| Crate | Role |
| --- | --- |
| `rc-app` (bin `dotcraft`) | Window, tool windows, editor tabs, keymaps, session. Only crate that wires everything together. |
| `rc-kit` | Islands theme (`theme`), New UI icons (`icons`, SVGs in `assets/icons`), window chrome, branding, i18n (`i18n::tr`). |
| `rc-core` | Shared tokio `Runtime`, `Inbox`/`Outbox`/`Waker`, settings, per-workspace session, workspace file tree, file watcher, Unity project detection and editor lookup (`unity`). |
| `rc-text` | Rope buffer, change sets, multi-cursor selections, grouped undo, UTF-16 positions. No UI. |
| `rc-syntax` | tree-sitter parsing on a background thread, highlighting, folding, bracket matching. |
| `rc-editor` | The code editor widget (egui), keymaps (`keymap::Action`), code-insight popups. |
| `rc-lsp` | JSON-RPC/LSP client and the Roslyn adapter. |
| `rc-dap` | DAP client and netcoredbg session; which evaluations may run code in the program (`side_effects`). |
| `rc-dotnet` | SDKs, solutions/projects, MSBuild, build/run/test, TRX, NuGet; Aspire app hosts (`aspire`: IDE session endpoint, resource service client); the Avalonia XAML previewer (`avalonia_preview`). |
| `rc-vcs` | Git through the `git` CLI; pull requests on GitHub and Azure DevOps (REST, tokens in the OS keychain). |
| `rc-search` | File index, fuzzy search, find/replace in files. |
| `rc-term` | Terminal emulator (alacritty_terminal + portable-pty) rendered in egui. |
| `rc-remote` (bin `dotcraft-remote-agent`) | Remote folder agent: JSON-line protocol over the stdin/stdout of `ssh` or `wsl.exe`, its client, and agent installation (`deploy`). |

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
- **Commits.** One commit per finished task, short one-line English messages, capitalized
  (e.g. "Add TRX parser"), no attribution trailers. Code comments in English too.
- **Blame.** `vcs_blame.rs` blames the editor's text (`git blame --contents -`) once per text
  version and HEAD, for the annotation column and Code Vision's author labels (declaration
  ranges from tree-sitter, so they work before Roslyn is ready).
- **Remote folders.** `ui/remote_explorer.rs` owns one worker per connection. Before it
  starts the agent, `rc_remote::deploy::ensure_agent` runs one-line `sh -c` scripts over the
  same launcher (`Endpoint::shell_command`): probe `uname -sm` and
  `~/.dotcraft/remote/<version>/`, then install the release asset (downloaded by the remote
  with curl/wget, else fetched here with reqwest and piped over stdin), checked against its
  `.sha256` and moved into place last. The agent serves list/read/write/run/cancel requests
  (1 MiB per file or output stream); Run and the Terminal are `ssh -t` / `wsl.exe` sessions in
  `rc-term`, not agent requests. `deploy::LocalShell` runs the same scripts on this machine
  for tests. The release workflows publish `rc-remote-<version>-{linux-x64,linux-arm64,
  macos-universal}.tar.gz` and their `.sha256`.
- **Hidden folders.** `rc_core::workspace::is_hidden_in` decides what the explorer, the file
  index, Find in Files and the watcher skip: `HIDDEN_DIRS` everywhere, plus
  `rc_core::unity::GENERATED_DIRS` only directly below a Unity project root (`Assets` and
  `ProjectSettings`). Use it (or `unity::in_generated_dir` for event paths) for any new walker.
- **Third-party code.** Parts of the HTTP client, EditorConfig, SQLite browser, egui
  menu, the editor tab docking guides and the title bar project glow are adapted from
  CrabCraft (MIT); its notice is in `docs/third-party/`.
- **XAML preview.** `rc_dotnet::avalonia_preview` speaks Avalonia's remote designer protocol
  (`tcp-bson`: an `i32` length, the message GUID, a BSON body; the small BSON subset is
  hand-written in `bson.rs`). DotCraft listens on a local port and starts
  `dotnet exec --runtimeconfig … --depsfile … Avalonia.Designer.HostApp.dll --transport
  tcp-bson://127.0.0.1:<port>/ --method avalonia-remote <app.dll>`, with the paths from
  `dotnet msbuild -getProperty` (`AvaloniaPreviewerNetCoreToolPath` comes from Avalonia's
  props). `rc-app/src/xaml_preview.rs` keeps one host per previewed file: restarted after a
  build or a crash, stopped when out of sight for two minutes, killed when the file closes.
  Frames are acknowledged only when the UI takes them, so a hidden or animated preview never
  queues frames.
- **Spelling.** Typo checks use the `spellbook` crate (MPL-2.0, unmodified, source on
  crates.io) with Hunspell `.aff`/`.dic` pairs found in `<data folder>/dictionaries`,
  `DOTCRAFT_DICTIONARIES` and the system folders (`~/Library/Spelling`, `/usr/share/hunspell`…).
  When none provides `en_US`, SCOWL's en_US pair from wooorm/dictionaries (MIT AND BSD, notice
  in `docs/third-party/SCOWL-en_US-LICENSE.txt`), embedded from `assets/dictionaries/`, is
  written to `<data folder>/dictionaries/bundled` and used. Saved words go to
  `<data folder>/spelling/user-dictionary.txt` and to the solution's Rider `.DotSettings` file.

## Aspire

`rc_dotnet::aspire` speaks to an app host without the `aspire` CLI. The app host is started
with the resource service endpoint of its launch profile (or a free port) and a fresh API key
(`ASPIRE_DASHBOARD_RESOURCESERVICE_APIKEY`), so `resources` can stream its resources over gRPC
(`aspire.v1.DashboardService`, hand-written protobuf over reqwest's HTTP/2). When debugging,
`ide_session` listens on loopback and the app host gets `DEBUG_SESSION_PORT`/`_TOKEN`/
`_RUN_MODE`: its orchestrator (DCP) then sends `PUT /run_session` for every project instead of
starting it. `rc-app/src/aspire.rs` turns those into extra sessions of the debugger
(`Debugger::others`, one tab each), which report the pid, output lines and exit back over the
notification WebSocket. `aspire_tests.rs` checks a breakpoint in a service of a real
`aspire-starter` solution (ignored: it needs nuget.org).

## Packaging

Everything the app needs (fonts, icons, translations) is embedded in the binary.
`scripts/bundle.sh` builds `DotCraft.app` (`--universal`, `--dmg`, `--zip`) and the Windows
x64/ARM64 executables with cargo-xwin (`--windows`, DirectX 12, statically linked C runtime,
icon from `assets/icon.ico` via `crates/rc-app/build.rs`). Application icons are generated
from `assets/icon.svg` by `scripts/make_icon.py`. The Roslyn language server and netcoredbg
are not bundled: they are located or downloaded on first use into the data folder. Only the
en_US spelling dictionary is (see Spelling above).
