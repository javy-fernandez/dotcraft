//! Phase 0 go/no-go: CPU time of a full editor frame (layout, painting and tessellation)
//! on a 20k-line C# file, while scrolling and while typing.

use criterion::{Criterion, criterion_group, criterion_main};
use egui::{Event, RawInput, Rect, pos2, vec2};
use rc_editor::{EditorConfig, EditorState, EditorView, Keymap, KeymapKind};
use std::hint::black_box;

fn setup() -> (egui::Context, EditorState) {
    let ctx = egui::Context::default();
    rc_kit::theme::install_fonts(&ctx);
    rc_kit::theme::apply(&ctx, true);
    let mut state = EditorState::new(1, Some("Big.cs".into()), &rc_syntax::sample::big_csharp(20_000));
    state.parse_blocking();
    (ctx, state)
}

fn input(events: Vec<Event>) -> RawInput {
    RawInput {
        screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1400.0, 900.0))),
        events,
        ..Default::default()
    }
}

/// One frame: run the UI and tessellate what it painted, like eframe does.
fn frame(ctx: &egui::Context, state: &mut EditorState, config: &EditorConfig, keymap: &Keymap, events: Vec<Event>) {
    let out = ctx.run_ui(input(events), |ui| {
        EditorView::new(state, config, keymap).focus(true).show(ui);
    });
    let prims = ctx.tessellate(out.shapes, out.pixels_per_point);
    black_box(prims);
}

fn benches(c: &mut Criterion) {
    let config = EditorConfig::default();
    let keymap = Keymap::new(KeymapKind::RiderMac);

    let (ctx, mut state) = setup();
    for _ in 0..3 {
        frame(&ctx, &mut state, &config, &keymap, vec![]);
    }
    let mut y = 0.0f32;
    c.bench_function("frame while scrolling 20k lines", |b| {
        b.iter(|| {
            // Three-line wheel steps through the whole file, so most lines are new each frame.
            y = (y + 63.0) % (20_000.0 * 21.0);
            state.set_scroll_offset(vec2(0.0, y));
            frame(&ctx, &mut state, &config, &keymap, vec![]);
        })
    });

    let mut k = 0u64;
    c.bench_function("frame after a far jump (every line new)", |b| {
        b.iter(|| {
            // Pseudo-random far positions: no line of the previous frame is visible.
            k = k.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let line = (k >> 33) % 19_900;
            state.set_scroll_offset(vec2(0.0, line as f32 * 21.0));
            frame(&ctx, &mut state, &config, &keymap, vec![]);
        })
    });

    let (ctx, mut state) = setup();
    state.goto(10_000, 8);
    for _ in 0..3 {
        frame(&ctx, &mut state, &config, &keymap, vec![]);
    }
    c.bench_function("frame while typing in a 20k-line file", |b| {
        b.iter(|| frame(&ctx, &mut state, &config, &keymap, vec![Event::Text("x".into())]))
    });

    let (ctx, mut state) = setup();
    for _ in 0..3 {
        frame(&ctx, &mut state, &config, &keymap, vec![]);
    }
    c.bench_function("idle frame (caret blink)", |b| b.iter(|| frame(&ctx, &mut state, &config, &keymap, vec![])));
}

criterion_group! {
    name = frame_benches;
    config = Criterion::default().sample_size(40).measurement_time(std::time::Duration::from_secs(4));
    targets = benches
}
criterion_main!(frame_benches);
