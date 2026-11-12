use egui::{Key, Modifiers, vec2};
use egui_kittest::Harness;
use rc_editor::{EditorConfig, EditorState, EditorView, Keymap, KeymapKind};
use rc_text::Selection;

const CALCULATOR: &str = include_str!("../../../tests/fixtures/Shop/src/Shop.Core/PriceCalculator.cs");

fn at_end(needle: &str) -> usize {
    let start = CALCULATOR.find(needle).expect("fixture marker");
    CALCULATOR[..start + needle.len()].chars().count()
}

#[test]
fn enter_indents_shop_switch_case_and_default_then_undoes() {
    let config = EditorConfig::default();
    let keymap = Keymap::new(KeymapKind::Rider);
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/Shop/src/Shop.Core/PriceCalculator.cs");
    assert!(path.exists());
    let state = EditorState::new(1, Some(path), CALCULATOR);
    let mut first = true;
    let mut harness = Harness::builder().with_size(vec2(800.0, 600.0)).build_ui_state(
        move |ui, state: &mut EditorState| {
            if std::mem::take(&mut first) {
                rc_kit::theme::install_fonts(ui.ctx());
                return;
            }
            let rect = ui.ctx().content_rect();
            let mut root = ui.new_child(egui::UiBuilder::new().max_rect(rect));
            EditorView::new(state, &config, &keymap).focus(true).show(&mut root);
        },
        state,
    );
    harness.run_steps(2);
    let ctrl = Modifiers { ctrl: true, ..Default::default() };
    for (label, next) in [
        ("case \"SAVE10\":", "return Math.Round(subtotal * 0.10m"),
        ("default:", "return order.ItemCount >= BulkItems"),
    ] {
        harness.state_mut().selection = Selection::point(at_end(label));
        harness.key_press(Key::Enter);
        harness.run_steps(1);
        let expected = format!("{label}\n                \n                {next}");
        assert!(harness.state().buffer.rope().to_string().contains(&expected), "{label}");
        assert_eq!(harness.state().caret_line_col().1, 16);
        harness.key_press_modifiers(ctrl, Key::Z);
        harness.run_steps(1);
        assert_eq!(harness.state().buffer.rope().to_string(), CALCULATOR);
    }
}
