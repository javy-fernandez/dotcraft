use egui::{Color32, Shape, vec2};
use egui_kittest::Harness;
use rc_editor::{EditorConfig, EditorState, EditorView, Keymap, KeymapKind};
use rc_syntax::Lang;
use rc_text::Selection;

const SHOP: &str = include_str!("../../../tests/fixtures/Shop/src/Shop.Api/appsettings.json");
const DEVELOPMENT: &str =
    include_str!("../../../tests/fixtures/Shop/src/Shop.Api/appsettings.Development.json");

fn painted_color(shapes: &[egui::epaint::ClippedShape], line: &str, token: &str) -> Color32 {
    shapes
        .iter()
        .find_map(|clipped| match &clipped.shape {
            Shape::Text(text) if text.galley.job.text == line => {
                let start = line.find(token)?;
                text.galley
                    .job
                    .sections
                    .iter()
                    .find(|section| {
                        section
                            .byte_range
                            .contains(&egui::epaint::text::ByteIndex(start))
                    })
                    .map(|section| section.format.color)
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing painted JSON token {token:?} in {line:?}"))
}

#[test]
fn shop_appsettings_parse_and_paint_in_both_themes() {
    for (name, source) in [
        ("appsettings.json", SHOP),
        ("appsettings.Development.json", DEVELOPMENT),
    ] {
        for dark in [false, true] {
            let mut state = EditorState::new(1, Some(name.into()), source);
            assert_eq!(state.lang(), Lang::Json);
            state.parse_blocking();
            assert!(!state.syntax.tree().unwrap().root_node().has_error());
            let information = source.find("Information").unwrap();
            state.selection = Selection::point(state.buffer.rope().byte_to_char(information));
            let config = EditorConfig::default();
            let keymap = Keymap::new(KeymapKind::RiderMac);
            let mut first = true;
            let mut h = Harness::builder()
                .with_size(vec2(900.0, 500.0))
                .build_ui_state(
                    move |ui, state: &mut EditorState| {
                        if std::mem::take(&mut first) {
                            rc_kit::theme::install_fonts(ui.ctx());
                            rc_kit::theme::apply(ui.ctx(), dark);
                            return;
                        }
                        let rect = ui.ctx().content_rect();
                        let mut root = ui.new_child(egui::UiBuilder::new().max_rect(rect));
                        EditorView::new(state, &config, &keymap)
                            .focus(true)
                            .show(&mut root);
                    },
                    state,
                );
            h.run_steps(3);
            let scheme = rc_kit::theme::scheme(dark);
            assert_eq!(
                painted_color(&h.output().shapes, "  \"Logging\": {", "\"Logging\""),
                scheme.field
            );
            assert_eq!(
                painted_color(
                    &h.output().shapes,
                    "      \"Default\": \"Information\",",
                    "\"Information\""
                ),
                scheme.string
            );

            h.event(egui::Event::Text("X".into()));
            h.run_steps(1);
            h.state_mut().parse_blocking();
            h.run_steps(1);
            assert!(
                h.state()
                    .buffer
                    .rope()
                    .to_string()
                    .contains("\"XInformation\"")
            );
            assert!(!h.state().syntax.tree().unwrap().root_node().has_error());
            assert_eq!(
                painted_color(
                    &h.output().shapes,
                    "      \"Default\": \"XInformation\",",
                    "\"XInformation\""
                ),
                scheme.string
            );
        }
    }
}
