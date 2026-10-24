//! Renders the terminal offscreen (wgpu) with a shell printing colours, styles, wide chars and
//! links, in both themes, for visual review. Captures go to `target/snapshots/`.
#![cfg(unix)]

use egui::vec2;
use egui_kittest::Harness;
use rc_core::Waker;
use rc_term::{SelectKind, SpawnSpec, TermConfig, TermSession, TerminalView};
use std::time::Duration;

const SCRIPT: &str = r#"
printf '\033[1mDotCraft terminal\033[0m  \033[3mitalic\033[0m  \033[4munderline\033[0m  \033[2mdim\033[0m  \033[7minverse\033[0m  \033[9mstrike\033[0m  \033[1;3;32mbold italic\033[0m\n'
i=0; while [ $i -lt 8 ]; do printf "\033[3${i}m color$i \033[0m"; i=$((i+1)); done; printf '\n'
i=0; while [ $i -lt 8 ]; do printf "\033[9${i}m bright$i\033[0m"; i=$((i+1)); done; printf '\n'
i=0; while [ $i -lt 8 ]; do printf "\033[4${i}m  bg$i  \033[0m"; i=$((i+1)); done; printf '\n'
i=0; while [ $i -lt 8 ]; do printf "\033[30;10${i}m  hi$i  \033[0m"; i=$((i+1)); done; printf '\n'
i=16; while [ $i -lt 232 ]; do printf "\033[48;5;${i}m "; i=$((i+3)); done; printf '\033[0m\n'
i=0; while [ $i -lt 72 ]; do printf "\033[48;2;$((i*3));$((120+i));$((255-i*3))m "; i=$((i+1)); done; printf '\033[0m\n'
printf '中文 日本語 한국어 | wide: [中] ok  ┌──┬──┐ │ab│cd│ └──┴──┘  λ → ≠ ✓\n'
printf '/repo/Shop/Program.cs(12,5): \033[31merror\033[0m CS1002: ; expected  https://learn.microsoft.com/dotnet\n'
printf '\033[32m✓\033[0m Passed!  - Failed: 0, Passed: 42, Skipped: 1 \033[2m(1.2s)\033[0m\n'
printf '\033[36m~/dotcraft\033[0m \033[35mmain\033[0m $ dotnet build'
sleep 30
"#;

fn session() -> TermSession {
    let mut spec = SpawnSpec::command("/bin/sh", ["-c", SCRIPT], None).size(96, 14);
    spec.announce_exit = false;
    let s = TermSession::spawn(&spec, Waker::noop()).expect("spawn /bin/sh");
    assert!(s.wait_for_text("dotnet build", Duration::from_secs(10)), "{:#?}", s.screen_lines());
    s
}

fn harness(dark: bool, s: TermSession) -> Harness<'static> {
    let config = TermConfig { cursor_blink: false, ..TermConfig::default() };
    let mut frame = 0;
    let mut h = Harness::builder().with_size(vec2(900.0, 300.0)).with_pixels_per_point(2.0).build_ui(move |ui| {
        frame += 1;
        if frame == 1 {
            rc_kit::theme::install_fonts(ui.ctx());
            rc_kit::theme::apply(ui.ctx(), dark);
            return;
        }
        if frame == 3 {
            // A selection over part of the "Passed" line (after the first layout sized the grid).
            s.select(SelectKind::Simple, (9, 2), (9, 8));
        }
        let rect = ui.ctx().content_rect();
        let mut root = ui.new_child(egui::UiBuilder::new().max_rect(rect));
        TerminalView::new(&s, &config).focus(true).show(&mut root);
    });
    h.run_steps(6);
    h
}

#[test]
#[ignore = "Requires a GPU; writes terminal captures to target/snapshots for review"]
fn render_terminal_snapshots() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/snapshots");
    std::fs::create_dir_all(&dir).unwrap();
    for dark in [true, false] {
        let mut h = harness(dark, session());
        let name = if dark { "terminal-dark.png" } else { "terminal-light.png" };
        h.render().unwrap().save(dir.join(name)).unwrap();
    }
}
