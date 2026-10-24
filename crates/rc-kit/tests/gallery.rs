//! Renders every icon of the set at 1x, for picking icons and checking their sharpness.
//! `cargo test -p rc-kit --test gallery -- --ignored` → target/snapshots/icons-*.png

use egui::{Align2, FontId, Rect, pos2, vec2};
use egui_kittest::Harness;

#[test]
#[ignore = "Requires a GPU; writes the icon gallery to target/snapshots"]
fn icon_gallery() {
    let names: Vec<&'static str> = rc_kit::icons::all_names().collect();
    let cols = 6;
    let rows = names.len().div_ceil(cols);
    for dark in [true, false] {
        let names = names.clone();
        let mut first = true;
        let mut h = Harness::builder().with_size(vec2(cols as f32 * 200.0, rows as f32 * 22.0 + 10.0)).build_ui(move |ui| {
            if std::mem::take(&mut first) {
                rc_kit::theme::install_fonts(ui.ctx());
                rc_kit::theme::apply(ui.ctx(), dark);
                return;
            }
            let p = rc_kit::pal(ui);
            ui.painter().rect_filled(ui.ctx().content_rect(), 0.0, p.island_bg);
            for (i, n) in names.iter().enumerate() {
                let (c, r) = (i % cols, i / cols);
                let at = pos2(5.0 + c as f32 * 200.0, 5.0 + r as f32 * 22.0);
                rc_kit::icons::paint(ui.painter(), Rect::from_min_size(at, vec2(16.0, 16.0)), rc_kit::Ico(n), &p, true);
                ui.painter().text(at + vec2(22.0, 8.0), Align2::LEFT_CENTER, *n, FontId::proportional(11.0), p.text);
            }
        });
        h.run_steps(3);
        let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/snapshots");
        std::fs::create_dir_all(&out).unwrap();
        h.render().unwrap().save(out.join(if dark { "icons-dark.png" } else { "icons-light.png" })).unwrap();
    }
}
