use egui::vec2;
use egui_kittest::Harness;
use rc_editor::{EditorConfig, EditorState, EditorView, Keymap, KeymapKind};
use rc_text::Selection;

const SHOP_CALCULATOR: &str = include_str!("../../../tests/fixtures/Shop/src/Shop.Core/PriceCalculator.cs");

fn harness() -> Harness<'static, EditorState> {
    let config = EditorConfig::default();
    let keymap = Keymap::new(KeymapKind::Rider);
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/Shop/src/Shop.Core/PriceCalculator.cs");
    assert!(path.exists());
    let mut state = EditorState::new(1, Some(path), SHOP_CALCULATOR);
    state.parse_blocking();
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

fn char_at(source: &str, needle: &str) -> usize {
    source[..source.find(needle).expect("fixture marker")].chars().count()
}

#[test]
fn csharp_brackets_in_shop_fixture_follow_editor_typing_path() {
    let mut harness = harness();
    let comment_end = char_at(SHOP_CALCULATOR, "grow.") + "grow.".chars().count();
    harness.state_mut().selection = Selection::point(comment_end);
    harness.event(egui::Event::Text("(".into()));
    harness.run_steps(1);
    assert!(harness.state().buffer.rope().to_string().contains("grow.(\n"));
    harness.state_mut().undo();
    assert_eq!(harness.state().buffer.rope().to_string(), SHOP_CALCULATOR);

    let code_end = char_at(SHOP_CALCULATOR, "ThrowIfNull(order);") + "ThrowIfNull(order);".chars().count();
    harness.state_mut().selection = Selection::point(code_end);
    harness.state_mut().parse_blocking();
    harness.event(egui::Event::Text("(".into()));
    harness.run_steps(1);
    assert!(harness.state().buffer.rope().to_string().contains("ThrowIfNull(order);()\n"));
    harness.event(egui::Event::Text(")".into()));
    harness.run_steps(1);
    assert!(harness.state().buffer.rope().to_string().contains("ThrowIfNull(order);()\n"));
    assert_eq!(harness.state().caret(), code_end + 2);
    harness.state_mut().undo();
    assert_eq!(harness.state().buffer.rope().to_string(), SHOP_CALCULATOR);

    let selected = char_at(SHOP_CALCULATOR, "PriceBreakdown(subtotal,") + "PriceBreakdown(".chars().count();
    harness.state_mut().selection = Selection::single(rc_text::Range::new(selected, selected + "subtotal".chars().count()));
    harness.state_mut().parse_blocking();
    harness.event(egui::Event::Text("(".into()));
    harness.run_steps(1);
    assert!(harness.state().buffer.rope().to_string().contains("PriceBreakdown((subtotal), discount"));
}
