//! Git UI: the Commit tool window (changes, message, commit), the Git tool window (log and
//! the files of a commit), the branches popup of the toolbar and the side-by-side diff.

use crate::app::DotCraftApp;
use crate::toolwindow::ToolWindow;
use crate::ui::kit;
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
    // Header: tabs Log / Stashes / Repository Tools; `tools.visible` and `show_stashes` are the selection.
    let busy = app.vcs.busy;
    let tools_tab = app.vcs.tools.visible;
    let tabs = [
        (ico::TW_VCS, tr("Log"), !tools_tab && !app.vcs.show_stashes),
        (ico::SHELVE, tr("Stashes"), !tools_tab && app.vcs.show_stashes),
        (ico::SETTINGS, tr("Repository Tools"), tools_tab),
    ];
    let mut actions = vec![(ico::HIDE, tr("Hide"), true), (ico::REFRESH, tr("Refresh"), busy.is_none() && app.vcs.repo.is_some())];
    if busy.is_some() {
        actions.push((ico::STOP, tr("Cancel"), true));
    }
    let click = kit::header(ui, tr("Git"), &tabs, busy.map(tr).unwrap_or(""), &actions);
    match click.tab {
        Some(0) => (app.vcs.tools.visible, app.vcs.show_stashes) = (false, false),
        Some(1) => (app.vcs.tools.visible, app.vcs.show_stashes) = (false, true),
        Some(2) => app.vcs.tools.visible = true,
        _ => {}
    }
    match click.action {
        Some(0) => {
            app.toggle_tool(ToolWindow::Git);
            return;
        }
        Some(1) => app.vcs.refresh(),
        Some(2) => app.vcs.cancel_operation(),
        _ => {}
    }
    if app.vcs.tools.visible {
        repository_tools(app, ui);
        return;
    }
    if app.vcs.repo.is_none() {
        empty_state(ui, ico::TW_VCS, tr("No Git repository"), tr("The workspace is not inside a Git repository."));
        return;
    }
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
    // Toolbar: message, options, Stash Changes.
    let mut push = false;
    kit::toolbar(ui, |ui| {
        ui.add_enabled_ui(!busy, |ui| {
            kit::search(ui, &mut app.vcs.stash_message, tr("Stash Message"), 220.0);
            ui.checkbox(&mut app.vcs.stash_include_untracked, tr("Include untracked files"));
            ui.checkbox(&mut app.vcs.stash_keep_index, tr("Keep staged changes"));
            ui.checkbox(&mut app.vcs.stash_restore_index, tr("Restore staged changes"));
        });
        push = kit::button(ui, Some(ico::SHELVE), tr("Stash Changes"), !busy).clicked();
    });
    if push && save_before_stash(app) {
        app.vcs.stash_push();
    }
    if let Some(error) = app.vcs.error.clone() {
        kit::content(ui, |ui| kit::message(ui, &error, true));
    }
    if let Some((stash, drop)) = app.vcs.stash_confirmation.clone() {
        let question = if drop { tr("Delete this stash permanently?") } else { tr("Apply and remove this stash?") };
        match confirm_box(ui, question, &stash.message, if drop { tr("Confirm Drop") } else { tr("Confirm Pop") }, !busy) {
            Some(true) => {
                app.vcs.stash_confirmation = None;
                if drop {
                    app.vcs.stash_drop(&stash);
                } else if save_before_stash(app) {
                    app.vcs.stash_apply(&stash, true);
                }
            }
            Some(false) => app.vcs.stash_confirmation = None,
            None => {}
        }
    }
    let p = rc_kit::pal(ui);
    let stashes = app.vcs.stashes.clone();
    let enabled = !busy && app.vcs.stash_confirmation.is_none();
    let pending = app.vcs.stash_confirmation.as_ref().map(|(s, _)| s.hash.clone());
    let mut action = None;
    egui::ScrollArea::vertical().id_salt("git_stashes").auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        if stashes.is_empty() {
            kit::content(ui, |ui| kit::note(ui, tr("No stashes")));
        }
        for stash in &stashes {
            let label = format!("stash@{{{}}}", stash.index);
            let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width().max(320.0), 26.0), Sense::click());
            if pending.as_deref() == Some(stash.hash.as_str()) {
                ui.painter().rect_filled(r.shrink2(vec2(6.0, 0.0)), 4.0, p.selection);
            } else if resp.hovered() || ui.rect_contains_pointer(r) {
                ui.painter().rect_filled(r.shrink2(vec2(6.0, 0.0)), 4.0, p.row_hover);
            }
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, format!("{label}: {}", stash.message)));
            // Apply / Pop / Drop on the right.
            let mut bu = ui.new_child(egui::UiBuilder::new().max_rect(r.shrink2(vec2(10.0, 1.0))).layout(egui::Layout::right_to_left(egui::Align::Center)));
            bu.spacing_mut().item_spacing.x = 2.0;
            if kit::button(&mut bu, Some(ico::DELETE), tr("Drop"), enabled).clicked() {
                action = Some((stash.clone(), 2));
            }
            if kit::button(&mut bu, None, tr("Pop"), enabled).clicked() {
                action = Some((stash.clone(), 1));
            }
            if kit::button(&mut bu, None, tr("Apply"), enabled).clicked() {
                action = Some((stash.clone(), 0));
            }
            let text_right = bu.min_rect().left() - 8.0;
            icons::paint(ui.painter(), Rect::from_center_size(pos2(r.left() + 20.0, r.center().y), vec2(16.0, 16.0)), ico::SHELVE, &p, true);
            let g = ui.painter().layout_no_wrap(label, FontId::proportional(13.0), p.text);
            let x = r.left() + 36.0;
            ui.painter().galley(pos2(x, (r.center().y - g.size().y / 2.0).round()), g.clone(), p.text);
            let mx = x + g.size().x + 8.0;
            if text_right - mx > 20.0 {
                let mut job = egui::text::LayoutJob::simple_singleline(stash.message.clone(), FontId::proportional(12.0), p.muted);
                job.wrap.max_width = text_right - mx;
                job.wrap.max_rows = 1;
                job.wrap.break_anywhere = true;
                job.wrap.overflow_character = Some('…');
                let mg = ui.painter().layout_job(job);
                ui.painter().galley(pos2(mx, (r.center().y - mg.size().y / 2.0).round()), mg, p.muted);
            }
            if enabled {
                resp.context_menu(|ui| {
                    for (i, (icon, text)) in [(None, tr("Apply")), (None, tr("Pop")), (Some(ico::DELETE), tr("Drop"))].into_iter().enumerate() {
                        if theme::menu_item(ui, icon, text, false).clicked() {
                            action = Some((stash.clone(), i));
                            ui.close();
                        }
                    }
                });
            }
        }
    });
    match action {
        Some((stash, 0)) => {
            if save_before_stash(app) {
                app.vcs.stash_apply(&stash, false);
            }
        }
        Some((stash, 1)) => app.vcs.stash_confirmation = Some((stash, false)),
        Some((stash, _)) => app.vcs.stash_confirmation = Some((stash, true)),
        None => {}
    }
}

/// An inline confirmation: question, detail and the primary / secondary buttons, padded and
/// framed like DotCraft's dialogs. `Some(true)` confirms, `Some(false)` cancels.
fn confirm_box(ui: &mut Ui, question: &str, detail: &str, confirm: &str, enabled: bool) -> Option<bool> {
    let p = rc_kit::pal(ui);
    let mut result = None;
    egui::Frame::new()
        .outer_margin(egui::Margin { left: 10, right: 10, top: 4, bottom: 6 })
        .inner_margin(egui::Margin::same(12))
        .fill(p.header_bg)
        .stroke(egui::Stroke::new(1.0, p.island_border))
        .corner_radius(6)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.horizontal(|ui| {
                icons::image(ui, ico::DIALOG_WARNING, 16.0);
                ui.label(egui::RichText::new(question).font(theme::bold(13.0)));
            });
            if !detail.is_empty() {
                ui.label(egui::RichText::new(detail).size(12.5).color(p.muted));
            }
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                if ui.add_enabled_ui(enabled, |ui| theme::primary_button(ui, confirm)).inner.clicked() {
                    result = Some(true);
                }
                if theme::secondary_button(ui, tr("Cancel")).clicked() {
                    result = Some(false);
                }
            });
        });
    result
}

/// A [`kit::section`] row whose title is exposed as a label: the Clone section shares its
/// title with the Clone button, which tests find by role and label.
fn tools_section(ui: &mut Ui, icon: rc_kit::Ico, title: &str, open: &mut bool) -> bool {
    let p = rc_kit::pal(ui);
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width().max(200.0), 26.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(r.shrink2(vec2(6.0, 0.0)), 4.0, p.row_hover);
    }
    let chevron = if *open { ico::CHEVRON_DOWN } else { ico::CHEVRON_RIGHT };
    icons::paint_sized(ui.painter(), Rect::from_center_size(pos2(r.left() + 16.0, r.center().y), vec2(14.0, 14.0)), chevron, &p, true, 14.0);
    icons::paint(ui.painter(), Rect::from_center_size(pos2(r.left() + 34.0, r.center().y), vec2(16.0, 16.0)), icon, &p, true);
    ui.painter().text(pos2(r.left() + 48.0, r.center().y), Align2::LEFT_CENTER, title, theme::bold(12.5), p.text);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, title));
    if resp.clicked() {
        *open = !*open;
    }
    *open
}

fn save_before_stash(app: &mut DotCraftApp) -> bool {
    app.save_all_docs();
    !app.docs.list.iter().any(|doc| doc.is_modified())
}

/// What the branches popup shows: the list, or the actions of one branch.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
enum BranchPage {
    #[default]
    List,
    Branch(rc_vcs::BranchKind, String),
}

#[derive(Clone, Debug, Default)]
struct BranchMenuState {
    page: BranchPage,
    filter: String,
    /// The filter got the keyboard focus once (on opening).
    focused: bool,
}

fn branches_menu_id() -> egui::Id {
    egui::Id::new("branches_menu")
}

/// Opens the branches popup on its list with an empty filter.
pub fn reset_branches_menu(ctx: &egui::Context) {
    ctx.data_mut(|d| d.insert_temp(branches_menu_id(), BranchMenuState::default()));
}

/// Saves the editors before a checkout, merge or rebase rewrites files; refuses while some
/// stay modified.
pub fn save_before_branch_change(app: &mut DotCraftApp) -> bool {
    app.save_all_docs();
    if app.docs.list.iter().any(|doc| doc.is_modified()) {
        app.notify(tr("Save or close modified documents before changing the repository"), true);
        return false;
    }
    true
}

/// A merge, rebase… is in progress or files are conflicted.
fn has_conflicts(app: &DotCraftApp) -> bool {
    app.vcs.state != rc_vcs::RepoState::Clean || app.vcs.status.as_ref().is_some_and(|s| s.entries.iter().any(|e| e.conflict.is_some()))
}

/// A muted group title (Local, Remote) inside the popup.
fn group_title(ui: &mut Ui, text: &str) {
    let p = rc_kit::pal(ui);
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::hover());
    ui.painter().text(pos2(r.left() + 10.0, r.center().y), Align2::LEFT_CENTER, text, theme::bold(11.5), p.muted);
}

/// The branches popup of the toolbar's VCS widget: actions, a filter and the local and
/// remote branches; clicking a branch shows its actions.
pub fn branches_menu(app: &mut DotCraftApp, ui: &mut Ui) {
    let id = branches_menu_id();
    let mut state = ui.ctx().data_mut(|d| d.get_temp::<BranchMenuState>(id)).unwrap_or_default();
    let viewport = ui.ctx().content_rect();
    ui.set_width(320.0_f32.min((viewport.width() - 32.0).max(120.0)));
    let max_height = (viewport.height() - 140.0).max(120.0);
    match state.page.clone() {
        BranchPage::List => branch_list(app, ui, &mut state, max_height),
        BranchPage::Branch(kind, name) => {
            if theme::menu_entry(ui, Some(ico::CHEVRON_LEFT), tr("Branches"), None, false, true).clicked() {
                state.page = BranchPage::List;
            }
            ui.separator();
            branch_actions(app, ui, kind, &name);
        }
    }
    ui.ctx().data_mut(|d| d.insert_temp(id, state));
}

fn branch_list(app: &mut DotCraftApp, ui: &mut Ui, state: &mut BranchMenuState, max_height: f32) {
    let busy = app.vcs.busy.is_some();
    let w = ui.available_width();
    ui.horizontal(|ui| {
        ui.add_space(4.0);
        let search = kit::search(ui, &mut state.filter, tr("Search branches"), w - 8.0);
        if !std::mem::replace(&mut state.focused, true) {
            search.request_focus();
        }
    });
    ui.add_space(2.0);
    if state.filter.is_empty() {
        if app.vcs.state != rc_vcs::RepoState::Clean {
            let rebase = app.vcs.state == rc_vcs::RepoState::Rebasing;
            if theme::menu_entry(ui, Some(ico::ABORT), if rebase { tr("Abort Rebase") } else { tr("Abort Merge") }, None, false, !busy).clicked() {
                app.vcs.abort_operation();
                ui.close();
            }
        }
        if theme::menu_entry(ui, Some(ico::UPDATE), tr("Update Project…"), None, false, !busy).clicked() {
            app.vcs.pull();
            ui.close();
        }
        if theme::menu_item(ui, Some(ico::COMMIT), tr("Commit…"), false).clicked() {
            app.dock.show(ToolWindow::Commit);
            ui.close();
        }
        if theme::menu_entry(ui, Some(ico::PUSH), tr("Push…"), None, false, !busy).clicked() {
            app.vcs.push();
            ui.close();
        }
        if theme::menu_entry(ui, Some(ico::FETCH), tr("Fetch"), None, false, !busy).clicked() {
            app.vcs.fetch();
            ui.close();
        }
        ui.separator();
        if theme::menu_entry(ui, Some(ico::ADD), tr("New Branch…"), None, false, !busy).clicked() {
            app.dialog = Some(crate::ui::dialogs::Dialog::NewBranch { name: String::new(), start: None, checkout: true });
            ui.close();
        }
    }
    let needle = state.filter.trim().to_lowercase();
    let mut open = None;
    // Show every row up to the screen's height (a popup's scroll area would start short).
    let rows = app.vcs.branches.iter().filter(|b| needle.is_empty() || b.name.to_lowercase().contains(&needle)).count();
    let spacing = ui.spacing().item_spacing.y;
    let want = rows as f32 * (24.0 + spacing) + 2.0 * (30.0 + 3.0 * spacing);
    egui::ScrollArea::vertical().id_salt("branches_menu_list").max_height(max_height).min_scrolled_height(want.min(max_height)).show(ui, |ui| {
        let mut any = false;
        for (title, kind) in [(tr("Local"), rc_vcs::BranchKind::Local), (tr("Remote"), rc_vcs::BranchKind::Remote)] {
            let list: Vec<&rc_vcs::Branch> = app.vcs.branches.iter().filter(|b| b.kind == kind && (needle.is_empty() || b.name.to_lowercase().contains(&needle))).collect();
            if list.is_empty() {
                continue;
            }
            any = true;
            ui.separator();
            group_title(ui, title);
            for b in list {
                let mut trail = String::new();
                if b.ahead > 0 {
                    trail.push_str(&format!("↑{} ", b.ahead));
                }
                if b.behind > 0 {
                    trail.push_str(&format!("↓{} ", b.behind));
                }
                trail.push('›');
                if theme::menu_entry(ui, Some(ico::VCS), &b.name, Some(&trail), b.is_head, true).clicked() {
                    open = Some(BranchPage::Branch(b.kind, b.name.clone()));
                }
            }
        }
        if !any && !needle.is_empty() {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.add_space(10.0);
                kit::note(ui, tr("No matching branches"));
            });
        }
    });
    if let Some(page) = open {
        state.page = page;
    }
}

/// The actions of one branch (Rider's branch submenu).
fn branch_actions(app: &mut DotCraftApp, ui: &mut Ui, kind: rc_vcs::BranchKind, name: &str) {
    use crate::ui::dialogs::Dialog;
    let Some(branch) = app.vcs.branches.iter().find(|b| b.kind == kind && b.name == name).cloned() else {
        ui.horizontal(|ui| {
            ui.add_space(10.0);
            kit::note(ui, tr("The branch no longer exists"));
        });
        return;
    };
    let p = rc_kit::pal(ui);
    let current = app.vcs.status.as_ref().and_then(|s| s.branch.head.clone());
    let current_text = current.clone().unwrap_or_else(|| "HEAD".into());
    let enabled = app.vcs.busy.is_none();
    let safe = enabled && !has_conflicts(app);
    // Title: the branch and its upstream.
    ui.horizontal(|ui| {
        ui.add_space(10.0);
        ui.label(egui::RichText::new(&branch.name).font(theme::bold(13.0)));
        if let Some(up) = &branch.upstream {
            ui.label(egui::RichText::new(format!("→ {up}")).size(12.0).color(p.muted));
        }
    });
    if !branch.subject.is_empty() {
        ui.horizontal(|ui| {
            ui.add_space(10.0);
            kit::note(ui, &branch.subject);
        });
    }
    ui.add_space(2.0);
    let entry = |ui: &mut Ui, icon: Option<rc_kit::Ico>, text: &str, on: bool| {
        let clicked = theme::menu_entry(ui, icon, text, None, false, on).clicked();
        if clicked {
            ui.close();
        }
        clicked
    };
    let remote = kind == rc_vcs::BranchKind::Remote;
    let head = branch.is_head;
    if !head && entry(ui, Some(ico::VCS), tr("Checkout"), safe) && save_before_branch_change(app) {
        app.vcs.checkout(&branch);
    }
    if entry(ui, Some(ico::ADD), &trf("New Branch from '{branch}'…", &[("branch", &branch.name)]), enabled) {
        app.dialog = Some(Dialog::NewBranch { name: String::new(), start: Some(branch.name.clone()), checkout: true });
    }
    if !head {
        ui.separator();
        if entry(ui, Some(ico::DIFF), &trf("Compare with '{current}'", &[("current", &current_text)]), true) {
            app.vcs.compare_with_current(&branch.name);
            app.dialog = Some(Dialog::CompareBranch { branch: branch.name.clone() });
        }
        let has_current = current.is_some();
        if entry(ui, Some(ico::MERGE), &trf("Merge '{branch}' into '{current}'", &[("branch", &branch.name), ("current", &current_text)]), safe && has_current) && save_before_branch_change(app) {
            app.vcs.merge_branch(&branch.name);
        }
        if !remote && entry(ui, Some(ico::ARROW_RIGHT), &trf("Rebase '{current}' onto '{branch}'", &[("branch", &branch.name), ("current", &current_text)]), safe && has_current) && save_before_branch_change(app) {
            app.vcs.rebase_onto(&branch.name);
        }
    }
    ui.separator();
    if remote {
        if entry(ui, Some(ico::DELETE), tr("Delete on Remote…"), safe) {
            app.dialog = Some(Dialog::DeleteRemoteBranch { name: branch.name.clone() });
        }
        return;
    }
    if entry(ui, Some(ico::PUSH), tr("Push…"), enabled) {
        app.vcs.push_branch(&branch.name);
    }
    if entry(ui, Some(ico::EDIT), tr("Rename…"), safe) {
        app.dialog = Some(Dialog::RenameBranch { old: branch.name.clone(), name: branch.name.clone() });
    }
    if !head && entry(ui, Some(ico::DELETE), tr("Delete…"), safe) {
        app.dialog = Some(Dialog::DeleteBranch { name: branch.name.clone(), force: false });
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
    let (on, repo_on) = (!busy, !busy && has_repo);
    let mut action = None;
    let mut open = None;
    // The pending confirmation and the last error stay above the scrolling sections.
    if let Some(confirmation) = app.vcs.tools.confirmation.clone() {
        let detail = match &confirmation {
            ToolAction::AbortCherryPick => tr("Abort the current cherry-pick and restore its starting state?").to_string(),
            ToolAction::RemoveWorktree(path) => trf("Remove linked worktree {path} and its ignored files? Git will refuse other unsaved changes or locked worktrees.", &[("path", &path.display())]),
            _ => String::new(),
        };
        match confirm_box(ui, tr("Confirm repository operation"), &detail, tr("Confirm"), on) {
            Some(true) => action = app.vcs.tools.confirmation.take(),
            Some(false) => app.vcs.tools.confirmation = None,
            None => {}
        }
    }
    if let Some(error) = app.vcs.error.clone() {
        kit::content(ui, |ui| kit::message(ui, &error, true));
    }
    egui::ScrollArea::vertical().id_salt("repository_tools").auto_shrink([false, false]).show(ui, |ui| {
        let tools = &mut app.vcs.tools;
        let w = (ui.available_width() - 72.0).clamp(160.0, 520.0);
        // Section open states live in egui memory, open by default.
        let section = |ui: &mut Ui, key: &str, icon, title: &str, body: &mut dyn FnMut(&mut Ui)| {
            let id = ui.id().with(("git_tools_section", key));
            let mut open = ui.data(|d| d.get_temp::<bool>(id)).unwrap_or(true);
            if tools_section(ui, icon, title, &mut open) {
                egui::Frame::new().inner_margin(egui::Margin { left: 48, right: 12, top: 2, bottom: 10 }).show(ui, |ui| {
                    ui.spacing_mut().item_spacing = vec2(6.0, 8.0);
                    body(ui)
                });
            }
            ui.data_mut(|d| d.insert_temp(id, open));
        };
        ui.add_space(4.0);
        section(ui, "clone", ico::DOWNLOAD, tr("Clone Repository"), &mut |ui| {
            kit::field(ui, tr("Repository URL or local path"), &mut tools.source, "", w, on);
            kit::field(ui, tr("New absolute destination"), &mut tools.destination, "", w, on);
            ui.horizontal_wrapped(|ui| {
                if kit::button(ui, Some(ico::DOWNLOAD), tr("Clone Repository"), on).clicked() {
                    action = Some(ToolAction::Clone { source: tools.source.clone(), destination: tools.destination.clone().into() });
                }
                if let Some(path) = &tools.cloned
                    && kit::button(ui, Some(ico::OPEN), tr("Open cloned repository"), on).clicked()
                {
                    open = Some(path.clone());
                }
            });
        });
        section(ui, "cherry", ico::COMMIT, tr("Cherry-pick"), &mut |ui| {
            kit::field(ui, tr("Commit hash or revision"), &mut tools.revision, "", w, repo_on);
            ui.horizontal_wrapped(|ui| {
                if kit::button(ui, Some(ico::COMMIT), tr("Cherry-pick commit"), repo_on).clicked() {
                    action = Some(ToolAction::CherryPick(tools.revision.clone()));
                }
                if kit::button(ui, Some(ico::RESUME), tr("Continue cherry-pick"), repo_on).clicked() {
                    action = Some(ToolAction::ContinueCherryPick);
                }
                if kit::button(ui, Some(ico::REVERT), tr("Abort cherry-pick"), repo_on).clicked() {
                    tools.confirmation = Some(ToolAction::AbortCherryPick);
                }
            });
        });
        section(ui, "worktrees", ico::FOLDER, tr("Worktrees"), &mut |ui| {
            kit::field(ui, tr("New branch name"), &mut tools.branch, "", w, repo_on);
            kit::field(ui, tr("Worktree absolute path"), &mut tools.worktree_path, "", w, repo_on);
            ui.horizontal_wrapped(|ui| {
                if kit::button(ui, None, tr("List worktrees"), repo_on).clicked() {
                    action = Some(ToolAction::ListWorktrees);
                }
                if kit::button(ui, Some(ico::ADD), tr("Add worktree"), repo_on).clicked() {
                    action = Some(ToolAction::AddWorktree { destination: tools.worktree_path.clone().into(), branch: tools.branch.clone() });
                }
                if kit::button(ui, Some(ico::REMOVE), tr("Remove worktree"), repo_on).clicked() {
                    tools.confirmation = Some(ToolAction::RemoveWorktree(tools.worktree_path.clone().into()));
                }
            });
        });
        section(ui, "submodules", ico::PROJECT, tr("Submodules"), &mut |ui| {
            ui.horizontal_wrapped(|ui| {
                if kit::button(ui, None, tr("Submodule status"), repo_on).clicked() {
                    action = Some(ToolAction::SubmoduleStatus);
                }
                if kit::button(ui, Some(ico::UPDATE), tr("Initialize and update submodules"), repo_on).clicked() {
                    action = Some(ToolAction::UpdateSubmodules);
                }
            });
        });
        section(ui, "lfs", ico::FILE_BINARY, tr("Git LFS"), &mut |ui| {
            kit::note(ui, tr("Requires git-lfs installed on PATH"));
            kit::field(ui, tr("LFS file pattern"), &mut tools.pattern, "", w, repo_on);
            ui.horizontal_wrapped(|ui| {
                if kit::button(ui, None, tr("LFS status"), repo_on).clicked() {
                    action = Some(ToolAction::LfsStatus);
                }
                if kit::button(ui, Some(ico::ADD), tr("Track LFS pattern"), repo_on).clicked() {
                    action = Some(ToolAction::LfsTrack(tools.pattern.clone()));
                }
                if kit::button(ui, Some(ico::FETCH), tr("Download LFS files"), repo_on).clicked() {
                    action = Some(ToolAction::LfsPull);
                }
            });
        });
        section(ui, "output", ico::TW_MESSAGES, tr("Repository tool output"), &mut |ui| {
            ui.add(egui::TextEdit::multiline(&mut tools.report).interactive(false).font(egui::TextStyle::Monospace).margin(vec2(8.0, 6.0)).desired_width(f32::INFINITY).desired_rows(8));
        });
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

#[cfg(test)]
mod branches_tests {
    use crate::app::DotCraftApp;
    use crate::ui::dialogs::Dialog;
    use egui::accesskit::Role;
    use egui_kittest::{Harness, kittest::Queryable};
    use rc_kit::i18n::tr;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};

    fn git(dir: &Path, args: &[&str]) -> String {
        let out = std::process::Command::new("git").current_dir(dir).args(args).env("LC_ALL", "C").output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    /// A clone of a bare remote: `main` and `feature` pushed, `topic` local and unmerged.
    fn repos(base: &Path) -> (PathBuf, PathBuf) {
        let remote = base.join("remote.git");
        std::fs::create_dir(&remote).unwrap();
        git(&remote, &["init", "-q", "--bare", "-b", "main"]);
        let root = base.join("repo");
        std::fs::create_dir(&root).unwrap();
        git(&root, &["init", "-q", "-b", "main"]);
        for (k, v) in [("user.name", "Test"), ("user.email", "test@example.com"), ("commit.gpgsign", "false"), ("core.hooksPath", "no-hooks")] {
            git(&root, &["config", k, v]);
        }
        git(&root, &["remote", "add", "origin", &format!("file://{}", remote.display())]);
        std::fs::write(root.join("a.txt"), "a\n").unwrap();
        git(&root, &["add", "."]);
        git(&root, &["commit", "-qm", "init"]);
        git(&root, &["push", "-q", "-u", "origin", "main"]);
        git(&root, &["branch", "feature"]);
        git(&root, &["push", "-q", "origin", "feature"]);
        git(&root, &["branch", "-q", "-D", "feature"]);
        git(&root, &["checkout", "-qb", "topic"]);
        std::fs::write(root.join("t.txt"), "t\n").unwrap();
        git(&root, &["add", "."]);
        git(&root, &["commit", "-qm", "topic work"]);
        git(&root, &["checkout", "-q", "main"]);
        (root, remote)
    }

    fn open(home: &Path, root: &Path) -> Harness<'static, DotCraftApp> {
        let mut app = crate::screens::test_app(home, true);
        app.settings.roslyn_path = Some("/nonexistent".into());
        app.open_workspace(root);
        let mut h = crate::screens::harness(app, egui::vec2(1200.0, 800.0));
        frames(&mut h, "branches", |a| a.vcs.branches.iter().any(|b| b.name == "origin/feature"));
        h
    }

    fn frames(h: &mut Harness<'static, DotCraftApp>, what: &str, mut done: impl FnMut(&DotCraftApp) -> bool) {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(30) {
            h.run_steps(1);
            if done(h.state()) {
                h.run_steps(2);
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("timed out waiting for {what}; error {:?}", h.state().vcs.error);
    }

    fn open_popup(h: &mut Harness<'static, DotCraftApp>) {
        h.get_by_role_and_label(Role::Button, tr("Git branches and actions")).click();
        h.run_steps(3);
    }

    fn open_branch(h: &mut Harness<'static, DotCraftApp>, name: &str) {
        open_popup(h);
        h.get_by_role_and_label(Role::Button, name).click();
        h.run_steps(3);
    }

    #[test]
    fn branches_popup_filters_and_shows_branch_actions() {
        let _lock = crate::screens::HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home = tempfile::tempdir().unwrap();
        let base = home.path().canonicalize().unwrap();
        let (root, _remote) = repos(&base);
        let mut h = open(&base.join("home"), &root);
        open_popup(&mut h);
        for name in ["main", "topic", "origin/main", "origin/feature"] {
            assert!(h.query_by_role_and_label(Role::Button, name).is_some(), "{name}");
        }
        assert!(h.query_by_role_and_label(Role::Button, tr("Fetch")).is_some());
        let search = h.get_by_role(Role::TextInput);
        search.focus();
        search.type_text("feat");
        h.run_steps(3);
        assert!(h.query_by_role_and_label(Role::Button, "topic").is_none());
        assert!(h.query_by_role_and_label(Role::Button, tr("Fetch")).is_none());
        h.get_by_role_and_label(Role::Button, "origin/feature").click();
        h.run_steps(3);
        // A remote branch: checkout, new branch, compare, merge, delete on the remote.
        assert!(h.query_by_role_and_label(Role::Button, tr("Checkout")).is_some());
        assert!(h.query_by_role_and_label(Role::Button, "Merge 'origin/feature' into 'main'").is_some());
        assert!(h.query_by_role_and_label(Role::Button, tr("Rename…")).is_none());
        h.get_by_role_and_label(Role::Button, tr("Branches")).click();
        h.run_steps(3);
        assert!(h.query_by_role(Role::TextInput).is_some());

        // The current branch has no checkout, merge or delete (clicking the widget closes the
        // popup; opening it again starts on the list).
        h.get_by_role_and_label(Role::Button, tr("Git branches and actions")).click();
        h.run_steps(3);
        open_branch(&mut h, "main");
        assert!(h.query_by_role_and_label(Role::Button, tr("Rename…")).is_some());
        assert!(h.query_by_role_and_label(Role::Button, tr("Delete…")).is_none());
        assert!(h.query_by_role_and_label(Role::Button, tr("Checkout")).is_none());
    }

    #[test]
    fn deleting_an_unmerged_branch_asks_twice_and_checkout_works() {
        let _lock = crate::screens::HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home = tempfile::tempdir().unwrap();
        let base = home.path().canonicalize().unwrap();
        let (root, _remote) = repos(&base);
        let mut h = open(&base.join("home"), &root);

        // Checkout from the popup, then back.
        open_branch(&mut h, "topic");
        h.get_by_role_and_label(Role::Button, tr("Checkout")).click();
        frames(&mut h, "checkout", |a| a.vcs.branch_name().as_deref() == Some("topic"));
        assert!(h.state().notices.iter().any(|n| n.text == "Checked out topic"));
        open_branch(&mut h, "main");
        h.get_by_role_and_label(Role::Button, tr("Checkout")).click();
        frames(&mut h, "checkout main", |a| a.vcs.branch_name().as_deref() == Some("main"));

        // Delete: confirm, then the not-fully-merged confirmation, then force.
        open_branch(&mut h, "topic");
        h.get_by_role_and_label(Role::Button, tr("Delete…")).click();
        h.run_steps(3);
        assert!(matches!(&h.state().dialog, Some(Dialog::DeleteBranch { name, force: false }) if name == "topic"));
        h.get_by_role_and_label(Role::Button, tr("Delete")).click();
        frames(&mut h, "force confirmation", |a| matches!(&a.dialog, Some(Dialog::DeleteBranch { force: true, .. })));
        assert!(git(&root, &["branch", "--list", "topic"]).contains("topic"));
        assert!(h.state().vcs.error.is_none());
        h.get_by_role_and_label(Role::Button, tr("Force Delete")).click();
        frames(&mut h, "forced delete", |a| a.vcs.busy.is_none() && !a.vcs.branches.iter().any(|b| b.name == "topic"));
        assert!(h.state().notices.iter().any(|n| n.text == "Deleted branch topic"));
        assert!(git(&root, &["branch", "--list", "topic"]).trim().is_empty());
    }

    #[test]
    fn deleting_a_remote_branch_is_confirmed_and_can_be_cancelled() {
        let _lock = crate::screens::HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home = tempfile::tempdir().unwrap();
        let base = home.path().canonicalize().unwrap();
        let (root, remote) = repos(&base);
        let mut h = open(&base.join("home"), &root);
        for confirm in [false, true] {
            open_branch(&mut h, "origin/feature");
            h.get_by_role_and_label(Role::Button, tr("Delete on Remote…")).click();
            h.run_steps(3);
            assert!(matches!(&h.state().dialog, Some(Dialog::DeleteRemoteBranch { name }) if name == "origin/feature"));
            assert!(h.query_by_label_contains("for everyone who uses it").is_some());
            h.get_by_role_and_label(Role::Button, if confirm { tr("Delete on Remote") } else { tr("Cancel") }).click();
            h.run_steps(3);
            assert!(h.state().dialog.is_none());
            if !confirm {
                assert!(h.state().vcs.busy.is_none());
                assert!(git(&remote, &["branch", "--list", "feature"]).contains("feature"));
            }
        }
        frames(&mut h, "remote delete", |a| a.vcs.busy.is_none() && !a.vcs.branches.iter().any(|b| b.name == "origin/feature"));
        assert!(git(&remote, &["branch", "--list", "feature"]).trim().is_empty());
        assert!(h.state().notices.iter().any(|n| n.text == "Deleted remote branch origin/feature"));
    }

    #[test]
    fn compare_rename_and_new_branch_dialogs() {
        let _lock = crate::screens::HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home = tempfile::tempdir().unwrap();
        let base = home.path().canonicalize().unwrap();
        let (root, _remote) = repos(&base);
        let mut h = open(&base.join("home"), &root);

        open_branch(&mut h, "topic");
        h.get_by_role_and_label(Role::Button, "Compare with 'main'").click();
        frames(&mut h, "compare", |a| a.vcs.compare.as_ref().is_some_and(|c| !c.loading));
        assert!(h.query_by_role_and_label(Role::Button, "topic work").is_some());
        h.get_by_role_and_label(Role::Button, tr("Close")).click();
        h.run_steps(3);
        assert!(h.state().dialog.is_none());

        open_branch(&mut h, "topic");
        h.get_by_role_and_label(Role::Button, tr("Rename…")).click();
        h.run_steps(3);
        h.get_by_role(Role::TextInput).type_text("-renamed");
        h.run_steps(2);
        h.get_by_role_and_label(Role::Button, tr("Rename")).click();
        frames(&mut h, "rename", |a| a.vcs.branches.iter().any(|b| b.name == "topic-renamed"));

        open_branch(&mut h, "topic-renamed");
        h.get_by_role_and_label(Role::Button, "New Branch from 'topic-renamed'…").click();
        h.run_steps(3);
        h.get_by_role(Role::TextInput).type_text("from-topic");
        h.run_steps(2);
        h.get_by_role_and_label(Role::Button, tr("Create")).click();
        frames(&mut h, "new branch", |a| a.vcs.branch_name().as_deref() == Some("from-topic"));
        assert!(root.join("t.txt").exists());
    }
}
