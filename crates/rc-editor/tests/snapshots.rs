//! Renders the editor offscreen (wgpu) with decorations, in both themes, for visual review.
//! Captures go to `target/snapshots/`.

use egui::vec2;
use egui_kittest::Harness;
use rc_editor::{BreakpointKind, Diagnostic, EditorConfig, EditorState, EditorView, GutterIcon, InlayHint, Keymap, KeymapKind, LineChange, Severity};

const SAMPLE: &str = r#"using System;
using System.Collections.Generic;
using System.Linq;

namespace DotCraft.Samples;

/// <summary>Greets people and remembers who it greeted.</summary>
public sealed class Greeter : IGreeter
{
    private readonly List<string> _seen = new();
    private const int MaxNames = 10;

    public string Prefix { get; init; } = "Hello";

    public string Greet(string name, bool shout = false)
    {
        if (string.IsNullOrWhiteSpace(name))
            throw new ArgumentException("Name required", nameof(name));

        _seen.Add(name);
        var text = $"{Prefix}, {name}!\n";
        return shout ? text.ToUpperInvariant() : text;
    }

    public IEnumerable<string> Recent(int count) =>
        _seen.TakeLast(Math.Min(count, MaxNames));

    // TODO: persist the history
    public void Reset()
    {
        _seen.Clear();
    }
}
"#;

fn state() -> EditorState {
    let mut s = EditorState::new(1, Some("Greeter.cs".into()), SAMPLE);
    s.parse_blocking();
    let find = |s: &EditorState, t: &str| s.buffer.rope().to_string().find(t).map(|b| s.buffer.rope().byte_to_char(b)).unwrap();
    let unknown = find(&s, "IGreeter");
    s.set_diagnostics(vec![
        Diagnostic { start: unknown, end: unknown + 8, severity: Severity::Error, message: "The type or namespace name 'IGreeter' could not be found".into(), code: Some("CS0246".into()), unnecessary: false, deprecated: false },
        Diagnostic { start: find(&s, "using System.Linq"), end: find(&s, "using System.Linq") + 18, severity: Severity::Hint, message: "Unnecessary using".into(), code: None, unnecessary: true, deprecated: false },
    ]);
    let call = find(&s, "count, MaxNames");
    s.inlays = vec![InlayHint { pos: call, label: "val1:".into(), pad_left: false, pad_right: true }];
    s.breakpoints.insert(19, BreakpointKind::Normal);
    s.breakpoints.insert(21, BreakpointKind::Conditional);
    s.gutter_icons.insert(14, GutterIcon::RunTest);
    s.gutter_icons.insert(7, GutterIcon::Implementing);
    s.line_changes.insert(20, LineChange::Modified);
    s.line_changes.insert(22, LineChange::Added);
    s.fold_at(30);
    let caret = find(&s, "Add(name)") + 4;
    s.selection = rc_text::Selection::point(caret);
    s
}

fn harness(dark: bool) -> Harness<'static, EditorState> {
    let config = EditorConfig::default();
    let keymap = Keymap::new(KeymapKind::RiderMac);
    let mut first = true;
    let mut h = Harness::builder().with_size(vec2(900.0, 760.0)).build_ui_state(
        move |ui, state: &mut EditorState| {
            if std::mem::take(&mut first) {
                rc_kit::theme::install_fonts(ui.ctx());
                rc_kit::theme::apply(ui.ctx(), dark);
                return;
            }
            let rect = ui.ctx().content_rect();
            let mut root = ui.new_child(egui::UiBuilder::new().max_rect(rect));
            EditorView::new(state, &config, &keymap).focus(true).show(&mut root);
        },
        state(),
    );
    h.run_steps(4);
    h
}

#[test]
#[ignore = "Requires a GPU; writes editor captures to target/snapshots for review"]
fn render_editor_snapshots() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/snapshots");
    std::fs::create_dir_all(&dir).unwrap();
    for dark in [true, false] {
        let mut h = harness(dark);
        let name = if dark { "editor-dark.png" } else { "editor-light.png" };
        h.render().unwrap().save(dir.join(name)).unwrap();
    }
}

/// Markup typing assists, typed through the editor in an offscreen harness: quotes after `=`,
/// the closing tag after `>`, Enter between the tags, and `</` completing the open element.
#[test]
fn markup_typing_assists() {
    let config = EditorConfig::default();
    let keymap = Keymap::new(KeymapKind::RiderMac);
    let state = EditorState::new(1, Some("Index.razor".into()), "");
    let mut h = Harness::builder().with_size(vec2(600.0, 300.0)).build_ui_state(
        move |ui, state: &mut EditorState| {
            let rect = ui.ctx().content_rect();
            let mut root = ui.new_child(egui::UiBuilder::new().max_rect(rect));
            EditorView::new(state, &config, &keymap).focus(true).show(&mut root);
        },
        state,
    );
    h.run_steps(2);
    let mut typed = |h: &mut Harness<'static, EditorState>, text: &str| {
        for ch in text.chars() {
            if ch == '\n' {
                h.key_press(egui::Key::Enter);
            } else {
                h.event(egui::Event::Text(ch.to_string()));
            }
            h.run_steps(1);
        }
    };
    typed(&mut h, "<div class=");
    assert_eq!(h.state().buffer.rope().to_string(), "<div class=\"\"");
    typed(&mut h, "box");
    h.state_mut().selection = rc_text::Selection::point(h.state().buffer.len_chars());
    typed(&mut h, ">");
    assert_eq!(h.state().buffer.rope().to_string(), "<div class=\"box\"></div>");
    typed(&mut h, "\n");
    assert_eq!(h.state().buffer.rope().to_string(), "<div class=\"box\">\n    \n</div>");
    typed(&mut h, "<ul><li>a</");
    assert_eq!(h.state().buffer.rope().to_string(), "<div class=\"box\">\n    <ul><li>a</li></ul>\n</div>");
}
