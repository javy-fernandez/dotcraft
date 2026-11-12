use egui::vec2;
use egui_kittest::Harness;
use rc_editor::{EditorConfig, EditorState, EditorView, Keymap, KeymapKind};
use rc_text::{Range, Selection};

const CALCULATOR: &str = include_str!("../../../tests/fixtures/Shop/src/Shop.Core/PriceCalculator.cs");

fn at(needle: &str) -> usize {
    CALCULATOR[..CALCULATOR.find(needle).expect("fixture marker")].chars().count()
}

#[test]
fn mixed_selection_and_caret_preserve_shop_expression_and_undo() {
    let config = EditorConfig::default();
    let keymap = Keymap::new(KeymapKind::Rider);
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/Shop/src/Shop.Core/PriceCalculator.cs");
    assert!(path.exists());
    let mut state = EditorState::new(1, Some(path), CALCULATOR);
    let selected = at("var taxable = subtotal") + "var taxable = ".chars().count();
    let caret = at("var tax = Math.Round") + "var tax = Math.Round(taxable * TaxRate, 2, MidpointRounding.AwayFromZero);".chars().count();
    state.selection = Selection::new([Range::new(selected, selected + "subtotal".chars().count()), Range::point(caret)], 0);
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
    harness.event(egui::Event::Text("(".into()));
    harness.run_steps(1);
    let edited = harness.state().buffer.rope().to_string();
    assert!(edited.contains("var taxable = (subtotal) - discount;"));
    assert!(edited.contains("var tax = Math.Round(taxable * TaxRate, 2, MidpointRounding.AwayFromZero);()"));
    assert_eq!(harness.state().selection.len(), 2);
    assert_eq!(harness.state().selected_text(), "subtotal");
    assert_eq!(harness.state().selection.ranges()[1].head, caret + 3);
    harness.state_mut().undo();
    assert_eq!(harness.state().buffer.rope().to_string(), CALCULATOR);
}
