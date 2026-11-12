use egui::{Key, vec2};
use egui_kittest::Harness;
use rc_editor::{EditorConfig, EditorState, EditorView, Keymap, KeymapKind};
use rc_text::Selection;

const SHOP: &str = include_str!("../../../tests/fixtures/Shop/src/Shop.Core/PriceCalculator.cs");

#[test]
fn shop_postfix_templates_expand_from_tab_and_undo() {
    let insertion = "var taxable = subtotal - discount;";
    for (key, expected) in [
        ("if", "if (taxable)"),
        ("not", "!(taxable)"),
        ("null", "if (taxable is null)"),
        ("notnull", "if (taxable is not null)"),
        ("return", "return taxable;"),
        ("await", "await taxable;"),
    ] {
        let postfix = format!("taxable.{key}");
        let source = SHOP.replace(insertion, &format!("{insertion}\n        {postfix}"));
        let mut state = EditorState::new(1, Some("PriceCalculator.cs".into()), &source);
        let caret_byte = source.find(&postfix).unwrap() + postfix.len();
        state.selection = Selection::point(state.buffer.rope().byte_to_char(caret_byte));
        state.parse_blocking();
        let config = EditorConfig::default();
        let keymap = Keymap::new(KeymapKind::RiderMac);
        let mut first = true;
        let mut h = Harness::builder()
            .with_size(vec2(900.0, 700.0))
            .build_ui_state(
                move |ui, state: &mut EditorState| {
                    if std::mem::take(&mut first) {
                        rc_kit::theme::install_fonts(ui.ctx());
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
        h.run_steps(2);
        h.key_press(Key::Tab);
        h.run_steps(1);
        let expanded = h.state().buffer.rope().to_string();
        assert!(expanded.contains(expected), "{key}: {expanded}");
        assert!(!expanded.contains(&postfix), "{key}: {expanded}");
        h.state_mut().undo();
        assert_eq!(h.state().buffer.rope().to_string(), source, "undo {key}");
    }
}
