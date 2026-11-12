//! Member picker for C# generation.

use crate::app::DotCraftApp;
use crate::generate::{Dialog, Kind};
use rc_kit::i18n::tr;

pub fn show(app: &mut DotCraftApp, ctx: &egui::Context) {
    let Some(mut dialog) = app.generate.take() else {
        return;
    };
    let mut keep = true;
    let mut generated = None;
    let mut use_roslyn = false;
    egui::Window::new(tr("Generate"))
        .id(egui::Id::new("generate_members"))
        .collapsible(false)
        .resizable(true)
        .default_width(470.0)
        .show(ctx, |ui| {
            ui.label(format!("{}: {}", tr("Type"), dialog.class_name));
            ui.horizontal_wrapped(|ui| {
                for kind in Kind::ALL {
                    ui.selectable_value(&mut dialog.kind, kind, kind.label());
                }
            });
            ui.separator();
            ui.label(tr("Choose members"));
            egui::ScrollArea::vertical()
                .max_height(280.0)
                .show(ui, |ui| {
                    if matches!(
                        dialog.kind,
                        Kind::Implement | Kind::Override | Kind::Delegate
                    ) {
                        if !dialog
                            .targets
                            .iter()
                            .any(|member| member.kind == dialog.kind)
                        {
                            ui.label(tr("No local members found"));
                            if ui.button(tr("Try Roslyn code actions")).clicked() {
                                use_roslyn = true;
                                keep = false;
                            }
                        }
                        for member in dialog
                            .targets
                            .iter_mut()
                            .filter(|member| member.kind == dialog.kind)
                        {
                            ui.checkbox(&mut member.selected, &member.label);
                        }
                    } else {
                        let candidates = dialog
                            .candidates()
                            .map(|(index, _)| index)
                            .collect::<Vec<_>>();
                        for index in candidates {
                            let member = &mut dialog.members[index];
                            ui.checkbox(
                                &mut member.selected,
                                format!("{}: {}", member.name, member.ty),
                            );
                        }
                    }
                });
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button(tr("Generate")).clicked() {
                    match dialog.render() {
                        Ok(code) => {
                            generated = Some(code);
                            keep = false;
                        }
                        Err(error) => app.notify(tr(error), true),
                    }
                }
                if ui.button(tr("Cancel")).clicked() {
                    keep = false;
                }
            });
        });
    if use_roslyn {
        app.run_insight_action(
            rc_editor::Action::ShowIntentions,
            app.caret_screen.unwrap_or(egui::Pos2::ZERO),
        );
    }
    if let Some(code) = generated {
        if let Some(doc) = app.docs.active_doc_mut() {
            if doc.editor.id == dialog.doc_id
                && doc.editor.buffer.version() == dialog.version
                && !doc.editor.read_only
            {
                let index = doc.editor.buffer.rope().byte_to_char(dialog.insert_byte);
                doc.editor
                    .apply_text_edits(vec![rc_text::Change::insert(index, code)]);
                app.focus_editor = true;
            } else {
                app.notify(tr("The document changed; reopen Generate"), true);
            }
        }
    }
    if keep {
        app.generate = Some(dialog);
    }
}

pub fn open(app: &mut DotCraftApp) {
    let Some(doc) = app.docs.active_doc() else {
        return;
    };
    if let Some(dialog) = Dialog::from_doc(doc) {
        app.generate = Some(dialog);
        app.focus_editor = false;
    } else {
        app.notify(
            tr("Place the caret inside a C# type to generate members"),
            true,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generate::Member;
    use egui_kittest::kittest::Queryable;

    #[test]
    fn picker_inserts_generated_constructor_into_editor() {
        let _lock = crate::screens::HOME_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let home = tempfile::tempdir().unwrap();
        let source = tempfile::tempdir().unwrap();
        let path = source.path().join("Person.cs");
        std::fs::write(&path, "class Person\n{\n}\n").unwrap();
        let mut app = crate::screens::test_app(home.path(), true);
        app.open_file(&path);
        let id = app.docs.active_doc().unwrap().editor.id;
        app.generate = Some(Dialog {
            doc_id: id,
            version: app.docs.active_doc().unwrap().editor.buffer.version(),
            class_name: "Person".into(),
            type_name: "Person".into(),
            insert_byte: "class Person\n{\n".len(),
            indent: String::new(),
            kind: Kind::Constructor,
            members: vec![Member {
                name: "_name".into(),
                ty: "string".into(),
                field: true,
                readonly: false,
                selected: true,
            }],
            targets: Vec::new(),
            existing: Vec::new(),
        });
        let mut harness = crate::screens::harness(app, egui::vec2(1100.0, 700.0));
        harness.run_steps(3);
        harness
            .get_by_role_and_label(egui::accesskit::Role::Button, tr("Generate"))
            .click();
        harness.run_steps(2);
        let text = harness
            .state()
            .docs
            .active_doc()
            .unwrap()
            .editor
            .buffer
            .rope()
            .to_string();
        assert!(text.contains("public Person(string name)"));
        assert!(text.contains("this._name = name;"));
    }
}
