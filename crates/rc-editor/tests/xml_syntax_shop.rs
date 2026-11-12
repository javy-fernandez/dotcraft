use egui::{Color32, Shape, vec2};
use egui_kittest::Harness;
use rc_editor::{EditorConfig, EditorState, EditorView, Keymap, KeymapKind};
use rc_syntax::Lang;
use rc_text::Selection;

const CORE: &str = include_str!("../../../tests/fixtures/Shop/src/Shop.Core/Shop.Core.csproj");
const API: &str = include_str!("../../../tests/fixtures/Shop/src/Shop.Api/Shop.Api.csproj");

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
        .unwrap_or_else(|| panic!("missing painted XML token {token:?} in {line:?}"))
}

#[test]
fn shop_csproj_xml_parses_and_paints_after_edit() {
    assert!(CORE.starts_with('\u{feff}'));
    for (name, source) in [("Shop.Core.csproj", CORE), ("Shop.Api.csproj", API)] {
        for dark in [false, true] {
            let mut state = EditorState::new(1, Some(name.into()), source);
            assert_eq!(state.lang(), Lang::Xml);
            state.parse_blocking();
            assert!(
                !state.syntax.tree().unwrap().root_node().has_error(),
                "{name}"
            );
            let framework = source.find("net8.0").unwrap();
            state.selection = Selection::point(state.buffer.rope().byte_to_char(framework));
            let config = EditorConfig::default();
            let keymap = Keymap::new(KeymapKind::RiderMac);
            let mut first = true;
            let mut h = Harness::builder()
                .with_size(vec2(950.0, 800.0))
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
            let project = source.lines().next().unwrap();
            assert_eq!(
                painted_color(&h.output().shapes, project, "Project"),
                scheme.class
            );
            assert_eq!(
                painted_color(&h.output().shapes, project, "Sdk"),
                scheme.xml_attr
            );
            assert_eq!(
                painted_color(&h.output().shapes, project, "Microsoft.NET.Sdk"),
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
                    .contains("<TargetFramework>Xnet8.0</TargetFramework>")
            );
            assert!(
                !h.state().syntax.tree().unwrap().root_node().has_error(),
                "{name} after edit"
            );
            assert_eq!(
                painted_color(
                    &h.output().shapes,
                    "    <TargetFramework>Xnet8.0</TargetFramework>",
                    "TargetFramework"
                ),
                scheme.class
            );
        }
    }
}
