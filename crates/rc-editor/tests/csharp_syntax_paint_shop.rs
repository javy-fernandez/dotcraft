use egui::{Color32, Shape, vec2};
use egui_kittest::Harness;
use rc_editor::{EditorConfig, EditorState, EditorView, Keymap, KeymapKind};
use rc_syntax::{Hl, Span};

const SHOP: &str = include_str!("../../../tests/fixtures/Shop/src/Shop.Core/PriceCalculator.cs");

fn painted_color(shapes: &[egui::epaint::ClippedShape], line: &str, token: &str) -> Color32 {
    fn visit(shape: &Shape, line: &str, token: &str) -> Option<Color32> {
        match shape {
            Shape::Text(text) if text.galley.job.text == line => {
                let start = line.find(token)?;
                text.galley
                    .job
                    .sections
                    .iter()
                    .find(|s| s.byte_range.contains(&egui::epaint::text::ByteIndex(start)))
                    .map(|s| s.format.color)
            }
            Shape::Vec(shapes) => shapes.iter().find_map(|s| visit(s, line, token)),
            _ => None,
        }
    }
    shapes
        .iter()
        .find_map(|s| visit(&s.shape, line, token))
        .unwrap_or_else(|| panic!("missing painted token {token:?} in {line:?}"))
}

#[test]
fn shop_csharp_tokens_keep_syntax_colors_in_both_themes() {
    for dark in [true, false] {
        let mut state = EditorState::new(1, Some("PriceCalculator.cs".into()), SHOP);
        state.parse_blocking();
        let comment_line = 2;
        let comment = state.buffer.line_text(comment_line);
        let string_line = SHOP
            .lines()
            .position(|line| line.contains("case \"SAVE10\""))
            .unwrap();
        let string = state.buffer.line_text(string_line);
        state.semantic = vec![Vec::new(); state.buffer.len_lines()];
        state.semantic[comment_line] = vec![Span {
            start: 0,
            end: comment.chars().count(),
            hl: Hl::Class,
        }];
        state.semantic[string_line] = vec![Span {
            start: 0,
            end: string.chars().count(),
            hl: Hl::Class,
        }];

        let config = EditorConfig::default();
        let keymap = Keymap::new(KeymapKind::RiderMac);
        let mut first = true;
        let h = {
            let mut h = Harness::builder()
                .with_size(vec2(1100.0, 1200.0))
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
            h
        };
        let shapes = &h.output().shapes;
        let scheme = rc_kit::theme::scheme(dark);
        assert_eq!(
            painted_color(shapes, "namespace Shop.Core;", "namespace"),
            scheme.keyword
        );
        assert_eq!(
            painted_color(
                shapes,
                "public sealed record PriceBreakdown(decimal Subtotal, decimal Discount, decimal Tax, decimal Shipping)",
                "public"
            ),
            scheme.keyword
        );
        assert_eq!(painted_color(shapes, &comment, "///"), scheme.comment);
        assert_eq!(painted_color(shapes, &string, "\"SAVE10\""), scheme.string);
    }
}
