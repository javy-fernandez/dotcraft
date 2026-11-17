//! Generate (⌘N / Alt+Insert), as in Rider: a popup at the caret lists what can be generated,
//! then a dialog lists the members to use. Both work from the keyboard: ↑/↓ move, Space
//! toggles a member, ⌘A / Ctrl+A toggles them all, Enter generates and Esc closes.

use crate::app::DotCraftApp;
use crate::generate::{Dialog, Kind};
use crate::ui::dialogs::{buttons, modal, title};
use crate::ui::kit;
use egui::{Align2, Context, FontId, Id, Key, Rect, Sense, Ui, pos2, vec2};
use rc_kit::i18n::{tr, trf};
use rc_kit::{icons, ico};

enum Outcome {
    Keep,
    Close,
    Generate,
    /// Nothing was found here: offer Roslyn's code actions instead.
    Roslyn,
}

/// Opens the picker for the type at the caret of the active document.
pub fn open(app: &mut DotCraftApp) {
    let Some(doc) = app.docs.active_doc() else { return };
    if let Some(dialog) = Dialog::from_doc(doc) {
        let discovery = crate::generate_semantic::GenerateDiscovery::start(doc, &dialog, app.insight.session.as_ref(), app.waker.clone());
        app.generate = Some(dialog);
        app.generate_discovery = discovery;
        app.focus_editor = false;
    } else {
        app.notify(tr("Place the caret inside a C# class, struct or record to generate members"), true);
    }
}

pub fn show(app: &mut DotCraftApp, ctx: &Context) {
    let Some(mut dialog) = app.generate.take() else {
        app.generate_discovery = None;
        return;
    };
    if let Some(discovery) = app.generate_discovery.as_mut() {
        discovery.update(&mut dialog);
    }
    let busy = app.generate_discovery.as_ref().is_some_and(|discovery| discovery.busy());
    let outcome = if dialog.choosing_kind { kind_popup(app, ctx, &mut dialog) } else { members_dialog(ctx, &mut dialog, busy) };
    if !matches!(outcome, Outcome::Keep) {
        app.generate_discovery = None;
    }
    match outcome {
        Outcome::Keep => app.generate = Some(dialog),
        Outcome::Close => app.focus_editor = true,
        Outcome::Roslyn => {
            app.focus_editor = true;
            app.run_insight_action(rc_editor::Action::ShowIntentions, app.caret_screen.unwrap_or(pos2(200.0, 200.0)));
        }
        Outcome::Generate => match insert(app, &dialog) {
            Ok(()) => app.focus_editor = true,
            Err(error) => {
                dialog.error = Some(error);
                app.generate = Some(dialog);
            }
        },
    }
}

/// Keeps the keyboard in the picker (not in the editor below) while it is open.
fn hold_focus(ui: &mut Ui, id: &str) {
    let holder = ui.interact(ui.max_rect(), Id::new(id), Sense::focusable_noninteractive());
    if !holder.has_focus() {
        holder.request_focus();
    }
    ui.memory_mut(|m| m.set_focus_lock_filter(holder.id, egui::EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: true }));
}

/// Moves `cursor` with ↑/↓ through `n` rows.
fn arrows(ctx: &Context, cursor: &mut usize, n: usize) {
    let n = n.max(1);
    if ctx.input(|i| i.key_pressed(Key::ArrowDown)) {
        *cursor = (*cursor + 1) % n;
    }
    if ctx.input(|i| i.key_pressed(Key::ArrowUp)) {
        *cursor = (*cursor + n - 1) % n;
    }
}

/// The first step: what to generate, in a popup at the caret.
fn kind_popup(app: &DotCraftApp, ctx: &Context, dialog: &mut Dialog) -> Outcome {
    let p = rc_kit::theme::palette(ctx.global_style().visuals.dark_mode);
    let pos = app.caret_screen.unwrap_or(pos2(200.0, 200.0));
    let kinds = dialog.kinds();
    arrows(ctx, &mut dialog.cursor, kinds.len());
    let mut chosen = ctx.input(|i| i.key_pressed(Key::Enter)).then_some(dialog.cursor);
    let mut close = ctx.input(|i| i.key_pressed(Key::Escape));
    let area = egui::Area::new(Id::new("generate_kinds")).order(egui::Order::Foreground).fixed_pos(pos).show(ctx, |ui| {
        hold_focus(ui, "generate_kinds_menu");
        egui::Frame::NONE.fill(p.popup_bg).stroke(egui::Stroke::new(1.0, p.popup_border)).corner_radius(8).inner_margin(egui::Margin::same(4)).show(ui, |ui| {
            ui.set_width(260.0);
            ui.label(egui::RichText::new(tr("Generate")).size(12.0).color(p.muted));
            for (i, kind) in kinds.iter().enumerate() {
                let (r, resp) = ui.allocate_exact_size(vec2(260.0, 26.0), Sense::click());
                if i == dialog.cursor {
                    ui.painter().rect_filled(r, 4.0, p.selection);
                } else if resp.hovered() {
                    ui.painter().rect_filled(r, 4.0, p.row_hover);
                }
                icons::paint(ui.painter(), Rect::from_center_size(r.left_center() + vec2(14.0, 0.0), vec2(16.0, 16.0)), kind.icon(), &p, true);
                ui.painter().text(r.left_center() + vec2(30.0, 0.0), Align2::LEFT_CENTER, kind.label(), FontId::proportional(13.0), p.text);
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, kind.label()));
                if resp.clicked() {
                    chosen = Some(i);
                }
            }
        });
    });
    // A press outside the list closes it (the rows themselves take the clicks).
    let outside = ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !area.response.rect.contains(p)));
    if outside && chosen.is_none() {
        close = true;
    }
    if let Some(kind) = chosen.and_then(|i| kinds.get(i)) {
        dialog.kind = *kind;
        dialog.choosing_kind = false;
        dialog.cursor = 0;
        dialog.error = None;
        return Outcome::Keep;
    }
    if close { Outcome::Close } else { Outcome::Keep }
}

/// The second step: the members to generate from, with check boxes.
fn members_dialog(ctx: &Context, dialog: &mut Dialog, busy: bool) -> Outcome {
    let targets = dialog.kind.uses_targets();
    let rows: Vec<usize> = if targets {
        dialog.targets.iter().enumerate().filter(|(_, t)| t.kind == dialog.kind).map(|(i, _)| i).collect()
    } else {
        dialog.candidates().map(|(i, _)| i).collect()
    };
    fn selected(dialog: &mut Dialog, targets: bool, i: usize) -> &mut bool {
        if targets { &mut dialog.targets[i].selected } else { &mut dialog.members[i].selected }
    }
    arrows(ctx, &mut dialog.cursor, rows.len());
    dialog.cursor = dialog.cursor.min(rows.len().saturating_sub(1));
    let (space, all, enter) = ctx.input(|i| (i.key_pressed(Key::Space), i.modifiers.command && i.key_pressed(Key::A), i.key_pressed(Key::Enter)));
    if space && let Some(&m) = rows.get(dialog.cursor) {
        *selected(dialog, targets, m) ^= true;
    }
    if all {
        let on = !rows.iter().all(|&m| *selected(dialog, targets, m));
        rows.iter().for_each(|&m| *selected(dialog, targets, m) = on);
    }
    let chosen = rows.iter().filter(|&&m| *selected(dialog, targets, m)).count();
    // A constructor without parameters is fine; everything else needs a member.
    let can_generate = chosen > 0 || dialog.kind == Kind::Constructor;
    let mut roslyn = false;
    let ((ok, cancel), esc) = modal(ctx, "generate_members", 480.0, |ui| {
        hold_focus(ui, "generate_members_list");
        title(ui, dialog.kind.label());
        kit::note(ui, &trf("Members of {type} to use", &[("type", &dialog.type_name)]));
        ui.add_space(6.0);
        let p = rc_kit::pal(ui);
        egui::Frame::new().stroke(egui::Stroke::new(1.0, p.control_border)).corner_radius(6).inner_margin(egui::Margin::same(4)).show(ui, |ui| {
            egui::ScrollArea::vertical().max_height(300.0).min_scrolled_height(120.0).auto_shrink([false, true]).show(ui, |ui| {
                if rows.is_empty() && targets && !busy {
                    ui.add_space(6.0);
                    kit::note(ui, tr("No members found"));
                    roslyn = kit::button(ui, Some(ico::BULB), tr("Try Roslyn code actions"), true).clicked();
                    ui.add_space(6.0);
                } else if rows.is_empty() {
                    ui.add_space(6.0);
                    kit::note(ui, tr("The type has no members to use"));
                    ui.add_space(6.0);
                }
                for (row, &m) in rows.iter().enumerate() {
                    let clicked = if targets {
                        let target = &dialog.targets[m];
                        member_row(ui, row == dialog.cursor, target.selected, dialog.kind.icon(), &target.label, &target.source)
                    } else {
                        let member = &dialog.members[m];
                        let icon = if member.field { ico::FIELD } else { ico::PROPERTY };
                        member_row(ui, row == dialog.cursor, member.selected, icon, &member.name, &member.ty)
                    };
                    if clicked {
                        dialog.cursor = row;
                        *selected(dialog, targets, m) ^= true;
                    }
                }
                if targets && busy {
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.add_space(8.0);
                        ui.spinner();
                        kit::note(ui, tr("Discovering members in other files…"));
                    });
                    ui.add_space(4.0);
                }
            });
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if kit::button(ui, None, tr("Select All"), !rows.is_empty()).clicked() {
                rows.iter().for_each(|&m| *selected(dialog, targets, m) = true);
            }
            if kit::button(ui, None, tr("Select None"), !rows.is_empty()).clicked() {
                rows.iter().for_each(|&m| *selected(dialog, targets, m) = false);
            }
        });
        if let Some(error) = &dialog.error {
            ui.add_space(6.0);
            kit::message(ui, error, true);
        }
        buttons(ui, tr("Generate"), can_generate)
    });
    if roslyn {
        return Outcome::Roslyn;
    }
    if (ok || enter) && can_generate {
        return Outcome::Generate;
    }
    if cancel || esc { Outcome::Close } else { Outcome::Keep }
}

/// A member row: check box, icon, name and grey detail. Returns whether it was clicked (the
/// caller toggles the member).
pub(crate) fn member_row(ui: &mut Ui, highlighted: bool, checked: bool, icon: rc_kit::Ico, name: &str, detail: &str) -> bool {
    let p = rc_kit::pal(ui);
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::click());
    if highlighted {
        ui.painter().rect_filled(r, 4.0, p.selection);
    } else if resp.hovered() {
        ui.painter().rect_filled(r, 4.0, p.row_hover);
    }
    let mut check = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_size(pos2(r.left() + 6.0, r.top() + 3.0), vec2(22.0, 20.0))).id_salt(("generate_check", name)));
    let mut shown = checked;
    let check = check.checkbox(&mut shown, "");
    check.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, checked, name));
    let toggled = check.clicked();
    icons::paint(ui.painter(), Rect::from_center_size(pos2(r.left() + 40.0, r.center().y), vec2(16.0, 16.0)), icon, &p, true);
    let g = ui.painter().layout_no_wrap(name.to_string(), FontId::proportional(13.0), p.text);
    ui.painter().galley(pos2(r.left() + 54.0, (r.center().y - g.size().y / 2.0).round()), g.clone(), p.text);
    if !detail.is_empty() {
        let mut job = egui::text::LayoutJob::simple_singleline(detail.to_string(), FontId::proportional(12.0), p.muted);
        job.wrap.max_width = (r.right() - r.left() - 70.0 - g.size().x).max(20.0);
        job.wrap.max_rows = 1;
        job.wrap.break_anywhere = true;
        job.wrap.overflow_character = Some('…');
        let detail = ui.painter().layout_job(job);
        ui.painter().galley(pos2(r.left() + 62.0 + g.size().x, (r.center().y - detail.size().y / 2.0).round()), detail, p.muted);
    }
    resp.clicked() || toggled
}

/// Inserts the generated members as one undoable edit and puts the caret on the first one.
fn insert(app: &mut DotCraftApp, dialog: &Dialog) -> Result<(), String> {
    let Some(doc) = app.docs.active_doc_mut() else { return Err(tr("The document was closed").into()) };
    if doc.editor.id != dialog.doc_id || doc.editor.buffer.version() != dialog.version || doc.editor.read_only {
        return Err(tr("The document changed; open Generate again").into());
    }
    let (range, text) = dialog.edit()?;
    let rope = doc.editor.buffer.rope();
    let (start, end) = (rope.byte_to_char(range.start), rope.byte_to_char(range.end));
    let first = text.chars().position(|c| !c.is_whitespace()).unwrap_or(0);
    doc.editor.apply_text_edits(vec![rc_text::Change::new(start, end, text)]);
    doc.editor.selection = rc_text::Selection::point(start + first);
    doc.editor.request_scroll_to_caret();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::kittest::Queryable;

    #[test]
    fn keyboard_picks_a_kind_and_generates_a_constructor() {
        let _lock = crate::screens::HOME_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let home = tempfile::tempdir().unwrap();
        let source = tempfile::tempdir().unwrap();
        let path = source.path().join("Person.cs");
        std::fs::write(&path, "class Person\n{\n    private string _name;\n    private int _age;\n}\n").unwrap();
        let mut app = crate::screens::test_app(home.path(), true);
        app.open_file(&path);
        let doc = app.docs.active_doc_mut().unwrap();
        doc.editor.selection = rc_text::Selection::point(20);
        open(&mut app);
        assert!(app.generate.as_ref().unwrap().choosing_kind);
        let mut harness = crate::screens::harness(app, egui::vec2(1100.0, 700.0));
        // Enter on the first kind (Constructor), then Space unticks _name and Enter generates.
        harness.key_press(Key::Enter);
        harness.run_steps(2);
        assert!(!harness.state().generate.as_ref().unwrap().choosing_kind);
        harness.key_press(Key::Space);
        harness.run_steps(2);
        harness.key_press(Key::Enter);
        harness.run_steps(2);
        let state = harness.state();
        assert!(state.generate.is_none());
        let text = state.docs.active_doc().unwrap().editor.buffer.rope().to_string();
        assert!(text.contains("\n\n    public Person(int age)\n    {\n        _age = age;\n    }\n}\n"), "{text}");
    }

    #[test]
    fn a_changed_document_is_not_edited() {
        let _lock = crate::screens::HOME_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let home = tempfile::tempdir().unwrap();
        let source = tempfile::tempdir().unwrap();
        let path = source.path().join("Person.cs");
        std::fs::write(&path, "class Person\n{\n    int _a;\n}\n").unwrap();
        let mut app = crate::screens::test_app(home.path(), true);
        app.open_file(&path);
        app.docs.active_doc_mut().unwrap().editor.selection = rc_text::Selection::point(8);
        open(&mut app);
        let mut dialog = app.generate.take().unwrap();
        dialog.kind = Kind::Constructor;
        app.docs.active_doc_mut().unwrap().editor.apply_text_edits(vec![rc_text::Change::insert(0, "// x\n")]);
        assert!(insert(&mut app, &dialog).is_err());
        assert!(!app.docs.active_doc().unwrap().editor.buffer.rope().to_string().contains("public Person"));
    }

    #[test]
    fn rows_toggle_with_the_mouse() {
        let _lock = crate::screens::HOME_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let home = tempfile::tempdir().unwrap();
        let source = tempfile::tempdir().unwrap();
        let path = source.path().join("Person.cs");
        std::fs::write(&path, "class Person\n{\n    private string _name;\n}\n").unwrap();
        let mut app = crate::screens::test_app(home.path(), true);
        app.open_file(&path);
        app.docs.active_doc_mut().unwrap().editor.selection = rc_text::Selection::point(20);
        open(&mut app);
        let dialog = app.generate.as_mut().unwrap();
        dialog.kind = Kind::Formatting;
        dialog.choosing_kind = false;
        let mut harness = crate::screens::harness(app, egui::vec2(1100.0, 700.0));
        harness.get_by_label("_name").click();
        harness.run_steps(2);
        assert!(!harness.state().generate.as_ref().unwrap().members[0].selected);
        harness.get_by_role_and_label(egui::accesskit::Role::Button, tr("Select All")).click();
        harness.run_steps(2);
        harness.get_by_role_and_label(egui::accesskit::Role::Button, tr("Generate")).click();
        harness.run_steps(2);
        let text = harness.state().docs.active_doc().unwrap().editor.buffer.rope().to_string();
        assert!(text.contains("public override string ToString() => $\"{nameof(_name)}: {_name}\";"), "{text}");
    }
}
