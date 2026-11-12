use egui::{Key, Modifiers, vec2};
use egui_kittest::Harness;
use rc_editor::{EditorConfig, EditorState, EditorView, Keymap, KeymapKind};
use rc_text::{Range, Selection};

const CALCULATOR: &str = include_str!("../../../tests/fixtures/Shop/src/Shop.Core/PriceCalculator.cs");

fn harness() -> Harness<'static, EditorState> {
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
    harness
}

fn at(needle: &str) -> usize {
    CALCULATOR[..CALCULATOR.find(needle).expect("fixture marker")].chars().count()
}

#[test]
fn line_comment_shortcut_toggles_selected_shop_statements_and_undoes() {
    let mut harness = harness();
    harness.state_mut().selection = Selection::single(Range::new(at("        var discount"), at("        var tax =")));
    let ctrl = Modifiers { ctrl: true, ..Default::default() };
    harness.key_press_modifiers(ctrl, Key::Slash);
    harness.run_steps(1);
    let commented = harness.state().buffer.rope().to_string();
    assert!(commented.contains("        // var discount = DiscountFor(order, subtotal);"));
    assert!(commented.contains("        // var taxable = subtotal - discount;"));
    harness.key_press_modifiers(ctrl, Key::Slash);
    harness.run_steps(1);
    assert_eq!(harness.state().buffer.rope().to_string(), CALCULATOR);
    harness.state_mut().undo();
    assert_eq!(harness.state().buffer.rope().to_string(), commented);
}

#[test]
fn block_comment_shortcut_toggles_selected_shop_expression() {
    let mut harness = harness();
    let expression = "subtotal - discount";
    let start = at(expression);
    harness.state_mut().selection = Selection::single(Range::new(start, start + expression.chars().count()));
    let ctrl_shift = Modifiers { ctrl: true, shift: true, ..Default::default() };
    harness.key_press_modifiers(ctrl_shift, Key::Slash);
    harness.run_steps(1);
    let commented = harness.state().buffer.rope().to_string();
    assert!(commented.contains("var taxable = /* subtotal - discount */;"));
    assert_eq!(harness.state().selected_text(), "/* subtotal - discount */");
    harness.key_press_modifiers(ctrl_shift, Key::Slash);
    harness.run_steps(1);
    assert_eq!(harness.state().buffer.rope().to_string(), CALCULATOR);
    harness.state_mut().undo();
    assert_eq!(harness.state().buffer.rope().to_string(), commented);
}
