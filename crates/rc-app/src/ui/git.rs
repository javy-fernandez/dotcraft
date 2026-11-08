//! Git UI: the Commit tool window (changes, message, commit), the Git tool window (log and
//! the files of a commit), the branches popup of the toolbar and the side-by-side diff.

use crate::app::DotCraftApp;
use crate::toolwindow::ToolWindow;
use crate::ui::toolwindows::{empty_state, header};
use egui::{Align2, Color32, FontId, Rect, Sense, Ui, pos2, vec2};
use rc_kit::i18n::{tr, trf};
use rc_kit::{ico, icons, theme};
use rc_vcs::FileState;

/// IntelliJ's file status colours.
fn state_color(state: FileState, untracked: bool, dark: bool) -> Color32 {
    let rgb = |c: u32| Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8);
    match (state, untracked, dark) {
        (_, true, true) => rgb(0xD1675A),
        (_, true, false) => rgb(0x993300),
        (FileState::Added, _, true) => rgb(0x73BD79),
        (FileState::Added, _, false) => rgb(0x067D17),
        (FileState::Deleted, _, true) => rgb(0x8C8C8C),
        (FileState::Deleted, _, false) => rgb(0x6C6C6C),
        (FileState::Unmerged, _, _) => rgb(0xD5756C),
        (_, _, true) => rgb(0x6F9EF7),
        (_, _, false) => rgb(0x0A54C9),
    }
}

pub fn commit_window(app: &mut DotCraftApp, ui: &mut Ui) {
    if header(ui, tr("Commit")) {
        app.toggle_tool(ToolWindow::Commit);
    }
    if app.vcs.repo.is_none() {
        empty_state(ui, ico::TW_COMMIT, tr("No Git repository"), tr("The workspace is not inside a Git repository."));
        return;
    }
    let p = rc_kit::pal(ui);
    let changes = app.vcs.changes();
    let root = app.vcs.repo.as_ref().map(|r| r.root().to_path_buf()).unwrap_or_default();
    // Toolbar.
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        if theme::icon_button(ui, ico::REFRESH, true, tr("Refresh")).clicked() {
            app.vcs.refresh();
        }
        let included: Vec<String> = changes.iter().filter(|c| !app.vcs.excluded.contains(&c.path)).map(|c| c.path.clone()).collect();
        if theme::icon_button(ui, ico::REVERT, !included.is_empty(), tr("Rollback…")).clicked() {
            app.dialog = Some(crate::ui::dialogs::Dialog::Rollback { paths: included.clone() });
        }
        if theme::icon_button(ui, ico::DIFF, !changes.is_empty(), tr("Show Diff")).clicked()
            && let Some(c) = changes.iter().find(|c| !c.untracked)
        {
            app.vcs.open_diff(&c.path);
        }
    });
    let list_h = (ui.available_height() - 170.0).max(80.0);
    let mut open_diff = None;
    let mut open_file = None;
    egui::ScrollArea::vertical().id_salt("commit_changes").max_height(list_h).auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        if changes.is_empty() {
            ui.add_space(20.0);
            ui.vertical_centered(|ui| ui.label(egui::RichText::new(tr("No changes")).color(p.muted)));
        }
        let groups = [(tr("Changes"), false), (tr("Unversioned Files"), true)];
        for (title, untracked) in groups {
            let items: Vec<&crate::vcs::Change> = changes.iter().filter(|c| c.untracked == untracked).collect();
            if items.is_empty() {
                continue;
            }
            let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::hover());
            ui.painter().text(pos2(r.left() + 12.0, r.center().y), Align2::LEFT_CENTER, format!("{title}  {}", items.len()), theme::bold(12.5), p.text);
            for c in items {
                let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width().max(200.0), 24.0), Sense::click());
                if resp.hovered() {
                    ui.painter().rect_filled(r.shrink2(vec2(6.0, 0.0)), 4.0, p.row_hover);
                }
                // Include checkbox.
                let cb = Rect::from_center_size(pos2(r.left() + 22.0, r.center().y), vec2(14.0, 14.0));
                let included = !app.vcs.excluded.contains(&c.path);
                ui.painter().rect(cb, 3.0, if included { p.accent } else { p.control_bg }, egui::Stroke::new(1.0, if included { p.accent } else { p.control_border }), egui::StrokeKind::Inside);
                if included {
                    let s = egui::Stroke::new(1.6, Color32::WHITE);
                    ui.painter().line_segment([cb.left_center() + vec2(3.0, 0.0), cb.center_bottom() + vec2(-1.0, -3.5)], s);
                    ui.painter().line_segment([cb.center_bottom() + vec2(-1.0, -3.5), cb.right_top() + vec2(-3.0, 3.5)], s);
                }
                let name = c.path.rsplit('/').next().unwrap_or(&c.path).to_string();
                let dir = c.path.rsplit_once('/').map(|(d, _)| d.to_string()).unwrap_or_default();
                icons::paint(ui.painter(), Rect::from_center_size(pos2(r.left() + 46.0, r.center().y), vec2(16.0, 16.0)), icons::for_file(&name), &p, true);
                let color = state_color(c.state, c.untracked, p.dark);
                let g = ui.painter().layout_no_wrap(name.clone(), FontId::proportional(13.0), color);
                ui.painter().galley(pos2(r.left() + 60.0, (r.center().y - g.size().y / 2.0).round()), g.clone(), color);
                ui.painter().text(pos2(r.left() + 68.0 + g.size().x, r.center().y), Align2::LEFT_CENTER, dir, FontId::proportional(12.0), p.muted);
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Checkbox, true, &name));
                if resp.clicked() && ui.input(|i| i.pointer.interact_pos()).is_some_and(|pt| pt.x < r.left() + 34.0) {
                    if included {
                        app.vcs.excluded.insert(c.path.clone());
                    } else {
                        app.vcs.excluded.remove(&c.path);
                    }
                }
                if resp.double_clicked() {
                    if c.untracked || c.state == FileState::Added {
                        open_file = Some(root.join(&c.path));
                    } else {
                        open_diff = Some(c.path.clone());
                    }
                }
                resp.context_menu(|ui| {
                    if theme::menu_item(ui, Some(ico::DIFF), tr("Show Diff"), false).clicked() {
                        open_diff = Some(c.path.clone());
                        ui.close();
                    }
                    if theme::menu_item(ui, Some(ico::OPEN), tr("Open"), false).clicked() {
                        open_file = Some(root.join(&c.path));
                        ui.close();
                    }
                    if theme::menu_item(ui, Some(ico::REVERT), tr("Rollback…"), false).clicked() {
                        app.dialog = Some(crate::ui::dialogs::Dialog::Rollback { paths: vec![c.path.clone()] });
                        ui.close();
                    }
                });
            }
        }
    });
    ui.separator();
    // Message and buttons.
    ui.add_space(4.0);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(ui.available_rect_before_wrap().shrink2(vec2(10.0, 4.0))));
    child.add(egui::TextEdit::multiline(&mut app.vcs.message).hint_text(tr("Commit Message")).desired_rows(4).desired_width(f32::INFINITY));
    child.horizontal(|ui| {
        ui.checkbox(&mut app.vcs.amend, tr("Amend"));
    });
    child.horizontal(|ui| {
        let busy = app.vcs.busy.is_some();
        if ui.add_enabled_ui(!busy, |ui| theme::primary_button(ui, tr("Commit"))).inner.clicked() {
            app.save_all_docs();
            app.vcs.commit(false);
        }
        if ui.add_enabled_ui(!busy, |ui| theme::secondary_button(ui, tr("Commit and Push…"))).inner.clicked() {
            app.save_all_docs();
            app.vcs.commit(true);
        }
        if busy {
            ui.add(egui::Spinner::new().size(14.0));
        }
    });
    if let Some(e) = &app.vcs.error {
        child.label(egui::RichText::new(e).color(p.error).size(12.5));
    }
    ui.advance_cursor_after_rect(ui.available_rect_before_wrap());
    if let Some(path) = open_diff {
        app.vcs.open_diff(&path);
    }
    if let Some(path) = open_file {
        app.open_file(&path);
    }
}

pub fn git_window(app: &mut DotCraftApp, ui: &mut Ui) {
    if header(ui, tr("Git")) {
        app.toggle_tool(ToolWindow::Git);
    }
    if ui.selectable_label(app.vcs.tools.visible, tr("Repository Tools")).clicked() {
        app.vcs.tools.visible = !app.vcs.tools.visible;
    }
    if app.vcs.tools.visible {
        repository_tools(app, ui);
        return;
    }
    if app.vcs.repo.is_none() {
        empty_state(ui, ico::TW_VCS, tr("No Git repository"), tr("The workspace is not inside a Git repository."));
        return;
    }
    ui.horizontal(|ui| {
        ui.selectable_value(&mut app.vcs.show_stashes, false, tr("Log"));
        ui.selectable_value(&mut app.vcs.show_stashes, true, tr("Stashes"));
        if ui.add_enabled(app.vcs.busy.is_none(), egui::Button::new(tr("Refresh"))).clicked() {
            app.vcs.refresh();
        }
        if app.vcs.busy.is_some() {
            ui.spinner();
            if ui.button(tr("Cancel")).clicked() {
                app.vcs.cancel_operation();
            }
        }
    });
    if app.vcs.show_stashes {
        stashes_view(app, ui);
        return;
    }
    let p = rc_kit::pal(ui);
    let avail = ui.available_rect_before_wrap();
    let left = Rect::from_min_size(avail.min, vec2((avail.width() * 0.62).max(300.0), avail.height()));
    let right = Rect::from_min_max(pos2(left.right() + 1.0, avail.top()), avail.max);
    ui.painter().vline(left.right() + 0.5, avail.y_range(), egui::Stroke::new(1.0, p.island_border));
    let mut select = None;
    let mut lu = ui.new_child(egui::UiBuilder::new().max_rect(left));
    lu.set_clip_rect(left.intersect(ui.clip_rect()));
    let log = app.vcs.log.clone();
    egui::ScrollArea::vertical().id_salt("git_log").auto_shrink([false, false]).show_rows(&mut lu, 24.0, log.len(), |ui, range| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for c in &log[range] {
            let selected = app.vcs.selected_commit.as_deref() == Some(c.hash.as_str());
            let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
            if selected {
                ui.painter().rect_filled(r.shrink2(vec2(4.0, 0.0)), 4.0, p.selection);
            } else if resp.hovered() {
                ui.painter().rect_filled(r.shrink2(vec2(4.0, 0.0)), 4.0, p.row_hover);
            }
            let dot = pos2(r.left() + 16.0, r.center().y);
            ui.painter().vline(dot.x, r.y_range(), egui::Stroke::new(1.5, p.accent.gamma_multiply(0.6)));
            ui.painter().circle(dot, 4.0, p.island_bg, egui::Stroke::new(2.0, p.accent));
            let mut x = r.left() + 30.0;
            for rf in c.refs.iter().take(3) {
                let label = match rf {
                    rc_vcs::RefLabel::Head => "HEAD".to_string(),
                    rc_vcs::RefLabel::LocalBranch(n) | rc_vcs::RefLabel::RemoteBranch(n) | rc_vcs::RefLabel::Tag(n) => n.clone(),
                    other => format!("{other:?}"),
                };
                let rect = rc_kit::theme::paint_badge(ui, pos2(x, r.center().y), &label);
                x = rect.right() + 4.0;
            }
            let date = crate::vcs::format_date(c.author_time);
            let meta = format!("{}  {}", c.author_name, date);
            let mg = ui.painter().layout_no_wrap(meta, FontId::proportional(12.0), p.muted);
            let maxw = (r.right() - mg.size().x - 16.0 - x).max(40.0);
            let mut job = egui::text::LayoutJob::simple_singleline(c.subject.clone(), FontId::proportional(13.0), p.text);
            job.wrap.max_width = maxw;
            job.wrap.max_rows = 1;
            job.wrap.break_anywhere = true;
            job.wrap.overflow_character = Some('…');
            let g = ui.painter().layout_job(job);
            ui.painter().galley(pos2(x, (r.center().y - g.size().y / 2.0).round()), g, p.text);
            ui.painter().galley(pos2(r.right() - mg.size().x - 10.0, (r.center().y - mg.size().y / 2.0).round()), mg, p.muted);
            if resp.clicked() {
                select = Some(c.hash.clone());
            }
        }
    });
    if let Some(h) = select {
        app.vcs.select_commit(&h);
    }
    // Details of the selected commit.
    let mut ru = ui.new_child(egui::UiBuilder::new().max_rect(right.shrink2(vec2(10.0, 4.0))));
    ru.set_clip_rect(right.intersect(ui.clip_rect()));
    let mut diff = None;
    if let Some(h) = app.vcs.selected_commit.clone()
        && let Some(c) = app.vcs.log.iter().find(|c| c.hash == h).cloned()
    {
        ru.label(egui::RichText::new(&c.subject).font(theme::bold(13.0)));
        if !c.body.trim().is_empty() {
            ru.label(egui::RichText::new(c.body.trim()).size(12.5));
        }
        ru.label(egui::RichText::new(format!("{}  {} <{}>  {}", c.short_hash(), c.author_name, c.author_email, crate::vcs::format_date(c.author_time))).size(12.0).color(p.muted));
        ru.add_space(6.0);
        let files = app.vcs.commit_files.clone();
        egui::ScrollArea::vertical().id_salt("commit_files").auto_shrink([false, false]).show(&mut ru, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for f in &files {
                let name = f.path.rsplit('/').next().unwrap_or(&f.path).to_string();
                let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
                if resp.hovered() {
                    ui.painter().rect_filled(r, 4.0, p.row_hover);
                }
                icons::paint(ui.painter(), Rect::from_center_size(pos2(r.left() + 10.0, r.center().y), vec2(16.0, 16.0)), icons::for_file(&name), &p, true);
                ui.painter().text(pos2(r.left() + 24.0, r.center().y), Align2::LEFT_CENTER, &f.path, FontId::proportional(12.5), p.text);
                if resp.double_clicked() || resp.clicked() {
                    diff = Some(f.path.clone());
                }
            }
        });
        if let Some(path) = diff {
            app.vcs.open_commit_diff(&h, &path);
        }
    } else {
        ru.add_space(20.0);
        ru.vertical_centered(|ui| ui.label(egui::RichText::new(tr("Select a commit to see its changes")).color(p.muted)));
    }
    ui.advance_cursor_after_rect(avail);
}

/// Stash operations run on the shared runtime. Destructive actions require an explicit
/// confirmation tied to the selected stash hash rather than its displayed index.
fn stashes_view(app: &mut DotCraftApp, ui: &mut Ui) {
    let busy = app.vcs.busy.is_some();
    ui.add_enabled_ui(!busy, |ui| {
        ui.add(egui::TextEdit::singleline(&mut app.vcs.stash_message).hint_text(tr("Stash Message")));
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut app.vcs.stash_include_untracked, tr("Include untracked files"));
            ui.checkbox(&mut app.vcs.stash_keep_index, tr("Keep staged changes"));
            if ui.button(tr("Stash Changes")).clicked() {
                if save_before_stash(app) {
                    app.vcs.stash_push();
                }
            }
        });
        ui.checkbox(&mut app.vcs.stash_restore_index, tr("Restore staged changes"));
    });
    if let Some(error) = &app.vcs.error {
        ui.colored_label(rc_kit::pal(ui).error, error);
    }
    ui.separator();
    if let Some((stash, drop)) = app.vcs.stash_confirmation.clone() {
        ui.group(|ui| {
            ui.label(if drop { tr("Delete this stash permanently?") } else { tr("Apply and remove this stash?") });
            ui.label(&stash.message);
            ui.horizontal(|ui| {
                if ui.add_enabled(!busy, egui::Button::new(if drop { tr("Confirm Drop") } else { tr("Confirm Pop") })).clicked() {
                    app.vcs.stash_confirmation = None;
                    if drop {
                        app.vcs.stash_drop(&stash);
                    } else {
                        if save_before_stash(app) {
                            app.vcs.stash_apply(&stash, true);
                        }
                    }
                }
                if ui.button(tr("Cancel")).clicked() {
                    app.vcs.stash_confirmation = None;
                }
            });
        });
    }
    let stashes = app.vcs.stashes.clone();
    egui::ScrollArea::vertical().id_salt("git_stashes").show(ui, |ui| {
        if stashes.is_empty() {
            ui.label(tr("No stashes"));
        }
        for stash in stashes {
            ui.group(|ui| {
                ui.label(format!("stash@{{{}}}: {}", stash.index, stash.message));
                ui.add_enabled_ui(app.vcs.busy.is_none() && app.vcs.stash_confirmation.is_none(), |ui| {
                    ui.horizontal(|ui| {
                        if ui.button(tr("Apply")).clicked() {
                            if save_before_stash(app) {
                                app.vcs.stash_apply(&stash, false);
                            }
                        }
                        if ui.button(tr("Pop")).clicked() {
                            app.vcs.stash_confirmation = Some((stash.clone(), false));
                        }
                        if ui.button(tr("Drop")).clicked() {
                            app.vcs.stash_confirmation = Some((stash.clone(), true));
                        }
                    });
                });
            });
        }
    });
}

fn save_before_stash(app: &mut DotCraftApp) -> bool {
    app.save_all_docs();
    !app.docs.list.iter().any(|doc| doc.is_modified())
}

/// The branches popup of the toolbar's VCS widget.
pub fn branches_menu(app: &mut DotCraftApp, ui: &mut Ui) {
    ui.set_min_width(280.0);
    let busy = app.vcs.busy.is_some();
    if ui.add_enabled(!busy, egui::Button::new(tr("Update Project…"))).clicked() {
        app.vcs.pull();
        ui.close();
    }
    if theme::menu_item(ui, Some(ico::COMMIT), tr("Commit…"), false).clicked() {
        app.dock.show(ToolWindow::Commit);
        ui.close();
    }
    if ui.add_enabled(!busy, egui::Button::new(tr("Push…"))).clicked() {
        app.vcs.push();
        ui.close();
    }
    if ui.add_enabled(!busy, egui::Button::new(tr("Fetch"))).clicked() {
        app.vcs.fetch();
        ui.close();
    }
    ui.separator();
    if theme::menu_item(ui, Some(ico::ADD), tr("New Branch…"), false).clicked() {
        app.dialog = Some(crate::ui::dialogs::Dialog::NewBranch { name: String::new() });
        ui.close();
    }
    let p = rc_kit::pal(ui);
    for (title, kind) in [(tr("Local"), rc_vcs::BranchKind::Local), (tr("Remote"), rc_vcs::BranchKind::Remote)] {
        let list: Vec<rc_vcs::Branch> = app.vcs.branches.iter().filter(|b| b.kind == kind).cloned().collect();
        if list.is_empty() {
            continue;
        }
        ui.separator();
        ui.label(egui::RichText::new(title).size(11.5).color(p.muted));
        for b in list.into_iter().take(30) {
            let mut text = b.name.clone();
            if b.ahead > 0 {
                text.push_str(&format!("  ↑{}", b.ahead));
            }
            if b.behind > 0 {
                text.push_str(&format!("  ↓{}", b.behind));
            }
            let resp = ui.add_enabled(!busy, egui::Button::selectable(b.is_head, text));
            if resp.clicked() && !b.is_head {
                app.vcs.checkout(&b);
                ui.close();
            }
        }
    }
}

/// The side-by-side diff in the editor area (two read-only editors scrolled together).
pub fn diff_view(app: &mut DotCraftApp, ui: &mut Ui) {
    let p = rc_kit::pal(ui);
    let full = ui.max_rect();
    let head = Rect::from_min_size(full.min, vec2(full.width(), 38.0));
    let mut close = false;
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(head.shrink2(vec2(10.0, 0.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
    if let Some(d) = &app.vcs.diff {
        icons::image(&mut child, ico::DIFF, 16.0);
        child.label(egui::RichText::new(&d.title).font(theme::bold(13.0)));
        child.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if theme::icon_button(ui, ico::CLOSE_SMALL, true, tr("Close")).clicked() {
                close = true;
            }
            if theme::menu_item(ui, Some(ico::OPEN), tr("Open File"), false).clicked() {
                close = true;
                let path = app.vcs.diff.as_ref().map(|d| d.path.clone());
                if let Some(path) = path {
                    app.open_file(&path);
                }
            }
        });
    }
    ui.painter().hline(full.x_range(), head.bottom() - 0.5, egui::Stroke::new(1.0, p.island_border));
    if close || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.vcs.diff = None;
        app.focus_editor = true;
        return;
    }
    let Some(d) = &mut app.vcs.diff else { return };
    let body = Rect::from_min_max(pos2(full.left(), head.bottom()), full.max);
    let mid = body.center().x;
    let left = Rect::from_min_max(body.min, pos2(mid - 0.5, body.bottom()));
    let right = Rect::from_min_max(pos2(mid + 0.5, body.top()), body.max);
    ui.painter().vline(mid, body.y_range(), egui::Stroke::new(1.0, p.island_border));
    // The side under the pointer leads the scroll.
    let pointer = ui.input(|i| i.pointer.hover_pos());
    let lead_right = pointer.is_some_and(|pt| right.contains(pt));
    let (lead, follow) = if lead_right { (&mut d.right, &mut d.left) } else { (&mut d.left, &mut d.right) };
    let off = lead.scroll_offset();
    follow.set_scroll_offset(off);
    let config = app.editor_config.clone();
    for (rect, state) in [(left, &mut d.left), (right, &mut d.right)] {
        let mut eu = ui.new_child(egui::UiBuilder::new().max_rect(rect));
        eu.set_clip_rect(rect.intersect(ui.clip_rect()));
        rc_editor::EditorView::new(state, &config, &app.keymap).show(&mut eu);
    }
    let _ = trf;
}

/// Repository maintenance is explicit and shares the Git operation/cancellation lifecycle.
fn repository_tools(app: &mut DotCraftApp, ui: &mut Ui) {
    use crate::vcs::tools::ToolAction;
    let busy = app.vcs.busy.is_some();
    let has_repo = app.vcs.repo.is_some();
    let mut action = None;
    let mut open = None;
    if busy {
        ui.horizontal(|ui| {
            ui.spinner();
            if ui.button(tr("Cancel")).clicked() {
                app.vcs.cancel_operation();
            }
        });
    }
    egui::ScrollArea::vertical().id_salt("repository_tools").show(ui, |ui| {
        ui.add_enabled_ui(!busy, |ui| {
            let tools = &mut app.vcs.tools;
            ui.heading(tr("Clone Repository"));
            ui.label(tr("Repository URL or local path"));
            ui.text_edit_singleline(&mut tools.source);
            ui.label(tr("New absolute destination"));
            ui.text_edit_singleline(&mut tools.destination);
            if ui.button(tr("Clone Repository")).clicked() {
                action = Some(ToolAction::Clone { source: tools.source.clone(), destination: tools.destination.clone().into() });
            }
            if let Some(path) = &tools.cloned {
                if ui.button(tr("Open cloned repository")).clicked() {
                    open = Some(path.clone());
                }
            }
            ui.separator();
            ui.add_enabled_ui(has_repo, |ui| {
                ui.heading(tr("Cherry-pick"));
                ui.label(tr("Commit hash or revision"));
                ui.text_edit_singleline(&mut tools.revision);
                ui.horizontal_wrapped(|ui| {
                    if ui.button(tr("Cherry-pick commit")).clicked() {
                        action = Some(ToolAction::CherryPick(tools.revision.clone()));
                    }
                    if ui.button(tr("Continue cherry-pick")).clicked() {
                        action = Some(ToolAction::ContinueCherryPick);
                    }
                    if ui.button(tr("Abort cherry-pick")).clicked() {
                        tools.confirmation = Some(ToolAction::AbortCherryPick);
                    }
                });
                ui.separator();
                ui.heading(tr("Worktrees"));
                ui.label(tr("New branch name"));
                ui.text_edit_singleline(&mut tools.branch);
                ui.label(tr("Worktree absolute path"));
                ui.text_edit_singleline(&mut tools.worktree_path);
                ui.horizontal_wrapped(|ui| {
                    if ui.button(tr("List worktrees")).clicked() {
                        action = Some(ToolAction::ListWorktrees);
                    }
                    if ui.button(tr("Add worktree")).clicked() {
                        action = Some(ToolAction::AddWorktree { destination: tools.worktree_path.clone().into(), branch: tools.branch.clone() });
                    }
                    if ui.button(tr("Remove worktree")).clicked() {
                        tools.confirmation = Some(ToolAction::RemoveWorktree(tools.worktree_path.clone().into()));
                    }
                });
                ui.separator();
                ui.heading(tr("Submodules"));
                ui.horizontal_wrapped(|ui| {
                    if ui.button(tr("Submodule status")).clicked() {
                        action = Some(ToolAction::SubmoduleStatus);
                    }
                    if ui.button(tr("Initialize and update submodules")).clicked() {
                        action = Some(ToolAction::UpdateSubmodules);
                    }
                });
                ui.separator();
                ui.heading(tr("Git LFS"));
                ui.label(tr("Requires git-lfs installed on PATH"));
                ui.label(tr("LFS file pattern"));
                ui.text_edit_singleline(&mut tools.pattern);
                ui.horizontal_wrapped(|ui| {
                    if ui.button(tr("LFS status")).clicked() {
                        action = Some(ToolAction::LfsStatus);
                    }
                    if ui.button(tr("Track LFS pattern")).clicked() {
                        action = Some(ToolAction::LfsTrack(tools.pattern.clone()));
                    }
                    if ui.button(tr("Download LFS files")).clicked() {
                        action = Some(ToolAction::LfsPull);
                    }
                });
            });
            if let Some(confirmation) = tools.confirmation.clone() {
                ui.separator();
                ui.label(tr("Confirm repository operation"));
                match &confirmation {
                    ToolAction::AbortCherryPick => {
                        ui.label(tr("Abort the current cherry-pick and restore its starting state?"));
                    }
                    ToolAction::RemoveWorktree(path) => {
                        ui.label(trf("Remove linked worktree {path} and its ignored files? Git will refuse other unsaved changes or locked worktrees.", &[("path", &path.display())]));
                    }
                    _ => {}
                }
                ui.horizontal(|ui| {
                    if ui.button(tr("Confirm")).clicked() {
                        action = tools.confirmation.take();
                    }
                    if ui.button(tr("Cancel")).clicked() {
                        tools.confirmation = None;
                    }
                });
            }
        });
        if let Some(error) = &app.vcs.error {
            ui.colored_label(rc_kit::pal(ui).error, error);
        }
        ui.separator();
        ui.label(tr("Repository tool output"));
        ui.add(egui::TextEdit::multiline(&mut app.vcs.tools.report).interactive(false).desired_width(f32::INFINITY).desired_rows(8));
    });
    if let Some(action) = action {
        // Refuse to change a worktree while editor buffers have unsaved changes.
        if action.changes_worktree() && app.docs.list.iter().any(|doc| doc.is_modified()) {
            app.vcs.error = Some(tr("Save or close modified documents before changing the repository").into());
        } else {
            app.vcs.repository_tool(action);
        }
    }
    if let Some(path) = open {
        app.open_path(&path);
    }
}

#[cfg(test)]
mod repository_tools_tests {
    use super::*;
    use egui_kittest::{Harness, kittest::Queryable};
    use std::time::{Duration, Instant};

    fn git(path: &std::path::Path, args: &[&str]) {
        let output = std::process::Command::new("git").current_dir(path).args(args).output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    }

    #[test]
    fn repository_tools_preserve_unsaved_editor_documents() {
        let _lock = crate::screens::HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home = tempfile::tempdir().unwrap();
        let file = home.path().join("App.cs");
        std::fs::write(&file, "saved\n").unwrap();
        let mut app = crate::screens::test_app(home.path(), true);
        let index = app.docs.open(&file).unwrap();
        app.docs.list[index].editor.replace(0, 0, "unsaved ");
        app.vcs.tools.visible = true;
        app.vcs.tools.confirmation = Some(crate::vcs::tools::ToolAction::AbortCherryPick);
        let mut first = true;
        let mut h = Harness::builder().with_size(egui::vec2(900.0, 1100.0)).build_ui_state(
            move |ui, app: &mut DotCraftApp| {
                if std::mem::take(&mut first) {
                    rc_kit::theme::install_fonts(ui.ctx());
                    return;
                }
                git_window(app, ui);
            },
            app,
        );
        h.run_steps(3);
        h.get_by_role_and_label(egui::accesskit::Role::Button, tr("Confirm")).click();
        h.run_steps(2);
        assert!(h.state().vcs.busy.is_none());
        assert_eq!(h.state().vcs.error.as_deref(), Some(tr("Save or close modified documents before changing the repository")));
        assert!(h.state().docs.list[index].is_modified());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "saved\n");
    }

    #[test]
    fn repository_tools_clone_only_after_click_and_open_the_result() {
        let _lock = crate::screens::HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home = tempfile::tempdir().unwrap();
        let source = home.path().join("source");
        std::fs::create_dir(&source).unwrap();
        git(&source, &["init", "-q"]);
        git(&source, &["config", "user.name", "Test"]);
        git(&source, &["config", "user.email", "test@example.com"]);
        git(&source, &["config", "commit.gpgsign", "false"]);
        std::fs::write(source.join("README.txt"), "cloned content").unwrap();
        git(&source, &["add", "."]);
        git(&source, &["commit", "-qm", "initial"]);
        let target = home.path().canonicalize().unwrap().join("result");
        let mut app = crate::screens::test_app(home.path(), true);
        app.vcs.tools.visible = true;
        app.vcs.tools.source = source.display().to_string();
        app.vcs.tools.destination = target.display().to_string();
        let mut first = true;
        let mut h = Harness::builder().with_size(egui::vec2(900.0, 1000.0)).build_ui_state(
            move |ui, app: &mut DotCraftApp| {
                if std::mem::take(&mut first) {
                    rc_kit::theme::install_fonts(ui.ctx());
                    return;
                }
                app.vcs.update(&mut app.docs, true);
                git_window(app, ui);
            },
            app,
        );
        h.run_steps(3);
        assert!(!target.exists());
        h.get_by_role_and_label(egui::accesskit::Role::Button, tr("Clone Repository")).click();
        let deadline = Instant::now() + Duration::from_secs(5);
        while h.state().vcs.tools.cloned.is_none() && Instant::now() < deadline {
            h.run_steps(2);
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(h.state().vcs.tools.cloned.as_deref(), Some(target.as_path()));
        assert_eq!(std::fs::read_to_string(target.join("README.txt")).unwrap(), "cloned content");
        h.get_by_role_and_label(egui::accesskit::Role::Button, tr("Open cloned repository")).click();
        h.run_steps(3);
        assert_eq!(h.state().workspace.as_ref().unwrap().ws.root, target);
        h.state_mut().close_workspace();
    }

    #[test]
    fn repository_tools_destructive_confirmation_can_be_cancelled() {
        let _lock = crate::screens::HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home = tempfile::tempdir().unwrap();
        let mut app = crate::screens::test_app(home.path(), true);
        app.vcs.tools.visible = true;
        app.vcs.tools.confirmation = Some(crate::vcs::tools::ToolAction::RemoveWorktree(home.path().to_path_buf()));
        let mut first = true;
        let mut h = Harness::builder().with_size(egui::vec2(900.0, 1100.0)).build_ui_state(
            move |ui, app: &mut DotCraftApp| {
                if std::mem::take(&mut first) {
                    rc_kit::theme::install_fonts(ui.ctx());
                    return;
                }
                git_window(app, ui);
            },
            app,
        );
        h.run_steps(3);
        assert!(h.state().vcs.busy.is_none());
        h.get_by_role_and_label(egui::accesskit::Role::Button, tr("Cancel")).click();
        h.run_steps(2);
        assert!(h.state().vcs.tools.confirmation.is_none());
        assert!(h.state().vcs.busy.is_none());
        assert!(home.path().exists());
    }
}
