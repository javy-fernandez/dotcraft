use egui::{Key, vec2};
use egui_kittest::Harness;
use rc_editor::{EditorConfig, EditorState, EditorView, Keymap, KeymapKind};
use rc_text::Selection;

const CALCULATOR: &str = include_str!("../../../tests/fixtures/Shop/src/Shop.Core/PriceCalculator.cs");

fn at_end(marker: &str) -> usize {
    let end = CALCULATOR.find(marker).expect("fixture marker") + marker.len();
    CALCULATOR[..end].chars().count()
}

fn harness(caret: usize) -> Harness<'static, EditorState> {
    let config = EditorConfig::default();
    let keymap = Keymap::new(KeymapKind::Rider);
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/Shop/src/Shop.Core/PriceCalculator.cs");
    assert!(path.exists());
    let mut state = EditorState::new(1, Some(path), CALCULATOR);
    state.selection = Selection::point(caret);
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

#[test]
fn shop_code_abbreviation_expands_with_tab_and_undoes() {
    let caret = at_end("ArgumentNullException.ThrowIfNull(order);\n");
    let mut harness = harness(caret);
    harness.event(egui::Event::Text("cw".into()));
    harness.run_steps(1);
    harness.key_press(Key::Tab);
    harness.run_steps(1);
    assert!(harness.state().buffer.rope().to_string().contains("ThrowIfNull(order);\nConsole.WriteLine();\n"));
    harness.state_mut().undo();
    assert!(harness.state().buffer.rope().to_string().contains("ThrowIfNull(order);\ncw\n"));
    harness.state_mut().undo();
    assert_eq!(harness.state().buffer.rope().to_string(), CALCULATOR);
}

#[test]
fn shop_comment_and_string_abbreviations_do_not_expand() {
    for caret in [at_end("grow."), at_end("case \"SAVE10")] {
        let mut harness = harness(caret);
        harness.event(egui::Event::Text(" cw".into()));
        harness.run_steps(1);
        harness.key_press(Key::Tab);
        harness.run_steps(1);
        let text = harness.state().buffer.rope().to_string();
        assert!(!text.contains("Console.WriteLine"));
        assert!(text.contains("grow. cw") || text.contains("\"SAVE10 cw"));
    }
}
