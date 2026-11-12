use egui::{Key, Modifiers, vec2};
use egui_kittest::Harness;
use rc_editor::{EditorConfig, EditorState, EditorView, Keymap, KeymapKind};
use rc_text::Selection;

const CALCULATOR: &str = include_str!("../../../tests/fixtures/Shop/src/Shop.Core/PriceCalculator.cs");

#[test]
fn shop_method_fold_unfold_and_fold_all_keep_visible_caret() {
    let config = EditorConfig::default();
    let keymap = Keymap::new(KeymapKind::Rider);
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/Shop/src/Shop.Core/PriceCalculator.cs");
    assert!(path.exists());
    let mut state = EditorState::new(1, Some(path), CALCULATOR);
    state.parse_blocking();
    let interior = CALCULATOR[..CALCULATOR.find("var discount = DiscountFor").unwrap()].chars().count();
    let interior_line = state.buffer.rope().char_to_line(interior);
    let method = state.syntax.folds().iter()
        .filter(|fold| fold.start_line < interior_line && interior_line < fold.end_line)
        .min_by_key(|fold| fold.end_line - fold.start_line)
        .copied().expect("method fold in Shop fixture");
    state.selection = Selection::point(interior);
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
    let ctrl_shift = Modifiers { ctrl: true, shift: true, ..Default::default() };
    harness.key_press_modifiers(ctrl, Key::Minus);
    harness.run_steps(1);
    assert!(harness.state().is_folded(method.start_line));
    assert_eq!(harness.state().caret_line_col().0, method.start_line);
    assert_eq!(harness.state().buffer.rope().to_string(), CALCULATOR);
    harness.key_press_modifiers(ctrl, Key::Equals);
    harness.run_steps(1);
    assert!(!harness.state().is_folded(method.start_line));

    let fold_start = harness.state().buffer.rope().line_to_char(method.start_line);
    harness.state_mut().selection = Selection::point(fold_start);
    harness.key_press_modifiers(ctrl_shift, Key::Minus);
    harness.run_steps(1);
    assert!(harness.state().is_folded(method.start_line));
    assert_eq!(harness.state().caret_line_col().0, method.start_line);
    harness.key_press_modifiers(ctrl_shift, Key::Equals);
    harness.run_steps(1);
    assert!(!harness.state().is_folded(method.start_line));

    harness.state_mut().selection = Selection::point(interior);
    harness.key_press_modifiers(ctrl_shift, Key::Minus);
    harness.run_steps(1);
    assert!(!harness.state().is_folded(method.start_line));
    assert_eq!(harness.state().caret_line_col().0, interior_line);
    harness.key_press_modifiers(ctrl_shift, Key::Equals);
    harness.run_steps(1);
    assert_eq!(harness.state().buffer.rope().to_string(), CALCULATOR);
}
