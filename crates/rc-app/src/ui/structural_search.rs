//! Preview and apply C# syntax-tree search results.

use crate::app::DotCraftApp;
use rc_kit::i18n::tr;
use rc_search::structural::NodeKind;

pub fn show(app: &mut DotCraftApp, ctx: &egui::Context) {
    app.structural.update();
    if !app.structural.visible {
        return;
    }
    let mut open = true;
    let mut start = false;
    let mut apply = false;
    let mut cancel = false;
    let mut inputs_changed = false;
    egui::Window::new(tr("Structural Search and Replace"))
        .id(egui::Id::new("structural_search"))
        .open(&mut open)
        .resizable(true)
        .default_width(760.0)
        .default_height(500.0)
        .show(ctx, |ui| {
            ui.label(tr("C# pattern"));
            inputs_changed |= ui
                .add_enabled(
                    !app.structural.applying,
                    egui::TextEdit::multiline(&mut app.structural.query)
                        .desired_rows(2)
                        .desired_width(f32::INFINITY),
                )
                .changed();
            ui.label(tr("Replacement"));
            inputs_changed |= ui
                .add_enabled(
                    !app.structural.applying,
                    egui::TextEdit::multiline(&mut app.structural.replacement)
                        .desired_rows(2)
                        .desired_width(f32::INFINITY),
                )
                .changed();
            for name in app.structural.placeholder_names() {
                let constraint = app.structural.constraints.entry(name.clone()).or_default();
                ui.add_enabled_ui(!app.structural.applying, |ui| {
                    ui.horizontal(|ui| {
                        ui.monospace(format!("${name}$"));
                        egui::ComboBox::from_id_salt(("structural_kind", &name))
                            .selected_text(kind_label(constraint.kind))
                            .show_ui(ui, |ui| {
                                for kind in [
                                    NodeKind::Any,
                                    NodeKind::Identifier,
                                    NodeKind::Expression,
                                    NodeKind::Statement,
                                    NodeKind::Type,
                                ] {
                                    inputs_changed |= ui
                                        .selectable_value(
                                            &mut constraint.kind,
                                            kind,
                                            kind_label(kind),
                                        )
                                        .changed();
                                }
                            });
                        let regex = constraint.text_regex.get_or_insert_with(String::new);
                        inputs_changed |= ui
                            .add(egui::TextEdit::singleline(regex).hint_text(tr("Text regex")))
                            .changed();
                    });
                });
            }
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        !app.structural.busy && !app.structural.applying,
                        egui::Button::new(tr("Search")),
                    )
                    .clicked()
                {
                    start = true;
                }
                if app.structural.busy && ui.button(tr("Cancel search")).clicked() {
                    cancel = true;
                }
                if ui
                    .add_enabled(
                        !app.structural.busy
                            && !app.structural.applying
                            && app.structural.hits.iter().any(|hit| hit.selected),
                        egui::Button::new(tr("Replace selected")),
                    )
                    .clicked()
                {
                    apply = true;
                }
                if app.structural.busy || app.structural.applying {
                    ui.spinner();
                    ctx.request_repaint_after(std::time::Duration::from_millis(100));
                }
            });
            if let Some(error) = &app.structural.error {
                ui.colored_label(rc_kit::pal(ui).error, error);
            }
            if let Some(summary) = &app.structural.summary {
                ui.label(summary);
            }
            ui.separator();
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for (index, hit) in app.structural.hits.iter_mut().enumerate() {
                        ui.horizontal(|ui| {
                            ui.checkbox(
                                &mut hit.selected,
                                format!("{}:{}", hit.path.display(), hit.line),
                            );
                            ui.label(format!("#{}", index + 1));
                        });
                        ui.monospace(format!("{} → {}", hit.before, hit.after));
                        ui.separator();
                    }
                });
        });
    if inputs_changed {
        app.structural.invalidate();
    }
    if cancel {
        app.structural.cancel();
    }
    if start {
        if let Some(index) = &app.files.index {
            app.structural.search((*index.snapshot()).clone());
        } else {
            app.structural.error = Some(tr("Open a workspace to search C# files").into());
        }
    }
    if apply {
        let conflict = app.docs.list.iter().any(|doc| {
            doc.is_modified()
                && app
                    .structural
                    .hits
                    .iter()
                    .any(|hit| hit.selected && hit.path == doc.path)
        });
        if conflict {
            app.structural.error =
                Some(tr("Save modified documents before replacing their matches").into());
        } else {
            app.structural.apply();
        }
    }
    if !open && !app.structural.applying {
        app.structural.cancel();
        app.structural.visible = false;
    }
}

fn kind_label(kind: NodeKind) -> &'static str {
    match kind {
        NodeKind::Any => tr("Any node"),
        NodeKind::Identifier => tr("Identifier"),
        NodeKind::Expression => tr("Expression"),
        NodeKind::Statement => tr("Statement"),
        NodeKind::Type => tr("Type"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::kittest::Queryable;

    #[test]
    fn headless_dialog_replaces_selected_csharp_match() {
        let _lock = crate::screens::HOME_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let home = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let file = workspace.path().join("A.cs");
        std::fs::write(&file, "class A { void M() { Console.WriteLine(value); } }").unwrap();
        let mut app = crate::screens::test_app(home.path(), true);
        app.open_workspace(workspace.path());
        app.structural.visible = true;
        app.structural.query = "Console.WriteLine($value$)".into();
        app.structural.replacement = "Log($value$)".into();
        app.structural
            .search(rc_search::IndexSnapshot::from_rel_paths(
                workspace.path(),
                ["A.cs"],
            ));
        let mut harness = crate::screens::harness(app, egui::vec2(1100.0, 720.0));
        for _ in 0..80 {
            harness.run_steps(2);
            if !harness.state().structural.busy {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(harness.state().structural.hits.len(), 1);
        harness
            .get_by_role_and_label(egui::accesskit::Role::Button, tr("Replace selected"))
            .click();
        for _ in 0..80 {
            harness.run_steps(2);
            if !harness.state().structural.applying {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(
            harness.state().structural.error.is_none(),
            "{:?}",
            harness.state().structural.error
        );
        assert!(
            std::fs::read_to_string(file)
                .unwrap()
                .contains("Log(value)")
        );
    }
}
