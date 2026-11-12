use egui::{Key, Modifiers, vec2};
use egui_kittest::Harness;
use rc_editor::{Action, EditorConfig, EditorState, EditorView, Keymap, KeymapKind, templates};
use rc_text::{Range, Selection};

const CALCULATOR: &str = include_str!("../../../tests/fixtures/Shop/src/Shop.Core/PriceCalculator.cs");

fn selected_statements() -> Selection {
    let start = CALCULATOR[..CALCULATOR.find("        var discount =").unwrap()].chars().count();
    let end = CALCULATOR[..CALCULATOR.find("        var tax =").unwrap()].chars().count();
    Selection::single(Range::new(start, end))
}

#[test]
fn shop_surround_action_expands_selected_statements_and_undoes() {
    let config = EditorConfig::default();
    let keymap = Keymap::new(KeymapKind::Rider);
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/Shop/src/Shop.Core/PriceCalculator.cs");
    assert!(path.exists());
    let mut editor = EditorState::new(1, Some(path), CALCULATOR);
    editor.selection = selected_statements();
    let mut first = true;
    let mut harness = Harness::builder().with_size(vec2(800.0, 600.0)).build_ui_state(
        move |ui, state: &mut (EditorState, Vec<Action>)| {
            if std::mem::take(&mut first) {
                rc_kit::theme::install_fonts(ui.ctx());
                return;
            }
            let rect = ui.ctx().content_rect();
            let mut root = ui.new_child(egui::UiBuilder::new().max_rect(rect));
            let output = EditorView::new(&mut state.0, &config, &keymap).focus(true).show(&mut root);
            state.1.extend(output.actions);
        },
        (editor, Vec::new()),
    );
    harness.run_steps(2);
    harness.key_press_modifiers(Modifiers { ctrl: true, alt: true, ..Default::default() }, Key::T);
    harness.run_steps(1);
    assert!(harness.state().1.contains(&Action::SurroundWith));

    let template = templates::SURROUND.iter().find(|template| template.key == "if").unwrap();
    templates::surround(&mut harness.state_mut().0, template);
    let text = harness.state().0.buffer.rope().to_string();
    assert!(text.contains("        if (condition)\n        {\n            var discount = DiscountFor(order, subtotal);\n            var taxable = subtotal - discount;\n        }"));
    assert_eq!(harness.state().0.selected_text(), "condition");
    harness.state_mut().0.undo();
    assert_eq!(harness.state().0.buffer.rope().to_string(), CALCULATOR);
}

#[test]
fn read_only_shop_document_cannot_enter_phantom_surround_snippet() {
    let mut editor = EditorState::new(2, Some("PriceCalculator.cs".into()), CALCULATOR);
    editor.selection = selected_statements();
    editor.read_only = true;
    let selection = editor.selection.clone();
    let template = templates::SURROUND.iter().find(|template| template.key == "if").unwrap();
    templates::surround(&mut editor, template);
    assert_eq!(editor.buffer.rope().to_string(), CALCULATOR);
    assert_eq!(editor.selection, selection);
    assert!(editor.snippet.is_none());
}
