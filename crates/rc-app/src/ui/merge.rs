//! The merge tool in the editor area (Rider's Conflicts dialog and three-way merge): the list
//! of conflicted files with Accept Yours / Accept Theirs / Merge and Continue / Abort of the
//! operation in progress, and the merge of one file in three panes — yours, the editable
//! result and theirs — with per-change arrows in the gutters between them.

use crate::app::DotCraftApp;
use crate::ui::kit;
use crate::vcs::merge;
use egui::{FontId, Rect, Sense, Ui, pos2, vec2};
use rc_kit::i18n::{tr, trf};
use rc_kit::{ico, icons, theme};
use rc_vcs::{ConflictKind, RepoState, Resolution, Side};

const HEAD_H: f32 = 38.0;
const TITLES_H: f32 = 26.0;
const BOTTOM_H: f32 = 46.0;
const GUTTER_W: f32 = 40.0;

/// The merge tool, drawn instead of the editors while it is open. It is opened with
/// `Vcs::open_merge_tool` (all conflicts) or `Vcs::open_merge_file` (one path), e.g. after a
/// failed merge, rebase, cherry-pick or interactive rebase step.
pub fn show(app: &mut DotCraftApp, ui: &mut Ui) {
    let Some(m) = &app.vcs.merge else { return };
    if m.open.is_some() { file_view(app, ui) } else { files_view(app, ui) }
}

/// What the operation in progress does, for the header.
fn operation_text(state: RepoState, ours: &str, theirs: &str) -> String {
    match state {
        RepoState::Merging => trf("Merging {theirs} into {ours}", &[("theirs", &theirs), ("ours", &ours)]),
        RepoState::Rebasing => trf("Rebasing {theirs} onto {ours}", &[("theirs", &theirs), ("ours", &ours)]),
        RepoState::CherryPicking => trf("Cherry-picking {theirs} onto {ours}", &[("theirs", &theirs), ("ours", &ours)]),
        RepoState::Reverting => trf("Reverting {theirs} on {ours}", &[("theirs", &theirs), ("ours", &ours)]),
        _ => String::new(),
    }
}

fn continue_text(state: RepoState) -> &'static str {
    match state {
        RepoState::Rebasing => tr("Continue Rebase"),
        RepoState::CherryPicking => tr("Continue Cherry-Pick"),
        RepoState::Reverting => tr("Continue Revert"),
        _ => tr("Continue Merge"),
    }
}

fn abort_text(state: RepoState) -> &'static str {
    match state {
        RepoState::Rebasing => tr("Abort Rebase"),
        RepoState::CherryPicking => tr("Abort Cherry-Pick"),
        RepoState::Reverting => tr("Abort Revert"),
        _ => tr("Abort Merge"),
    }
}

fn kind_text(kind: Option<ConflictKind>) -> &'static str {
    match kind {
        Some(ConflictKind::BothModified) | None => tr("Both modified"),
        Some(ConflictKind::BothAdded) => tr("Both added"),
        Some(ConflictKind::BothDeleted) => tr("Both deleted"),
        Some(ConflictKind::AddedByUs) => tr("Added by you"),
        Some(ConflictKind::AddedByThem) => tr("Added by them"),
        Some(ConflictKind::DeletedByUs) => tr("Deleted by you"),
        Some(ConflictKind::DeletedByThem) => tr("Deleted by them"),
    }
}

/// The painted header: optional back button, merge icon, bold title, grey status, close.
/// Returns (back, close).
fn head(ui: &mut Ui, full: Rect, title: &str, status: &str, back: bool) -> (bool, bool) {
    let p = rc_kit::pal(ui);
    let r = Rect::from_min_size(full.min, vec2(full.width(), HEAD_H));
    let (mut go_back, mut close) = (false, false);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(r.shrink2(vec2(10.0, 0.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
    if back && theme::icon_button(&mut child, ico::BACK, true, tr("Back to Conflicts")).clicked() {
        go_back = true;
    }
    icons::image(&mut child, ico::MERGE, 16.0);
    child.label(egui::RichText::new(title).font(theme::bold(13.0)));
    if !status.is_empty() {
        child.add_space(6.0);
        kit::note(&mut child, status);
    }
    child.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if theme::icon_button(ui, ico::CLOSE_SMALL, true, tr("Close Merge Tool")).clicked() {
            close = true;
        }
    });
    ui.painter().hline(full.x_range(), r.bottom() - 0.5, egui::Stroke::new(1.0, p.island_border));
    (go_back, close)
}

/// The conflicted files of the operation in progress.
fn files_view(app: &mut DotCraftApp, ui: &mut Ui) {
    let full = ui.max_rect();
    let Some(m) = &app.vcs.merge else { return };
    let (state, files, busy) = (m.state, m.files.clone(), m.busy.is_some());
    let status = operation_text(state, &m.sides.0, &m.sides.1);
    let (_, close) = head(ui, full, tr("Conflicts"), &status, false);
    if close {
        app.vcs.merge = None;
        app.focus_editor = true;
        return;
    }
    let kinds: std::collections::HashMap<String, Option<ConflictKind>> =
        app.vcs.status.as_ref().map(|s| s.conflicts().map(|e| (e.path.clone(), e.conflict)).collect()).unwrap_or_default();
    let body = Rect::from_min_max(pos2(full.left(), full.top() + HEAD_H), full.max);
    let mut ui = ui.new_child(egui::UiBuilder::new().max_rect(body));
    ui.set_clip_rect(body.intersect(ui.clip_rect()));
    let Some(m) = &mut app.vcs.merge else { return };
    let selected = m.selected.clone();
    let mut open_file = None;
    kit::toolbar(&mut ui, |ui| {
        let sel = selected.is_some() && !busy;
        if kit::button(ui, None, tr("Accept Yours"), sel).clicked()
            && let Some(s) = &selected
        {
            m.accept_side(vec![s.clone()], Side::Ours);
        }
        if kit::button(ui, None, tr("Accept Theirs"), sel).clicked()
            && let Some(s) = &selected
        {
            m.accept_side(vec![s.clone()], Side::Theirs);
        }
        if kit::button(ui, Some(ico::MERGE), tr("Merge…"), sel).clicked() {
            open_file = selected.clone();
        }
        theme::toolbar_separator(ui);
        let in_progress = state != RepoState::Clean;
        if kit::button(ui, Some(ico::RESUME), continue_text(state), in_progress && files.is_empty() && !busy && !m.loading).clicked() {
            m.continue_operation();
        }
        if kit::button(ui, Some(ico::ABORT), abort_text(state), in_progress && !busy).clicked() {
            m.abort_operation();
        }
        if theme::icon_button(ui, ico::REFRESH, !busy, tr("Refresh")).clicked() {
            m.reload();
        }
        if busy {
            ui.add(egui::Spinner::new().size(14.0));
        }
    });
    if let Some(e) = m.error.clone() {
        kit::content(&mut ui, |ui| kit::message(ui, &e, true));
    }
    if let Some((path, why)) = m.unmergeable.clone()
        && selected.as_deref() == Some(path.as_str())
    {
        kit::content(&mut ui, |ui| kit::message(ui, &why, false));
    }
    egui::ScrollArea::vertical().id_salt("merge_files").auto_shrink([false, false]).show(&mut ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        if files.is_empty() && !m.loading {
            let text = if state == RepoState::Clean { tr("No conflicts") } else { tr("All conflicts are resolved. Continue to finish the operation.") };
            kit::content(ui, |ui| kit::note(ui, text));
        }
        for path in &files {
            let name = path.rsplit('/').next().unwrap_or(path);
            let dir = path.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
            let secondary = if dir.is_empty() { kind_text(kinds.get(path).copied().flatten()).to_string() } else { format!("{dir}  ·  {}", kind_text(kinds.get(path).copied().flatten())) };
            let resp = kit::row(ui, 0, Some(icons::for_file(name)), name, &secondary, selected.as_deref() == Some(path.as_str()));
            if resp.clicked() {
                m.selected = Some(path.clone());
            }
            if resp.double_clicked() {
                open_file = Some(path.clone());
            }
            resp.context_menu(|ui| {
                if theme::menu_item(ui, Some(ico::MERGE), tr("Merge…"), false).clicked() {
                    open_file = Some(path.clone());
                    ui.close();
                }
                for (text, side) in [(tr("Accept Yours"), Side::Ours), (tr("Accept Theirs"), Side::Theirs)] {
                    if theme::menu_entry(ui, None, text, None, false, !busy).clicked() {
                        m.accept_side(vec![path.clone()], side);
                        ui.close();
                    }
                }
            });
        }
    });
    if let Some(path) = open_file {
        m.open_file(&path);
    }
}

/// A 16 pt icon button painted in a merge gutter; reports `label` to AccessKit.
fn gutter_button(ui: &mut Ui, rect: Rect, id: egui::Id, icon: rc_kit::Ico, label: &str) -> bool {
    let p = rc_kit::pal(ui);
    let resp = ui.interact(rect, id, Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect.expand(2.0), 4.0, p.hover);
    }
    icons::paint_sized(ui.painter(), Rect::from_center_size(rect.center(), vec2(14.0, 14.0)), icon, &p, true, 14.0);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    resp.on_hover_text(label).clicked()
}

enum ChunkAction {
    Accept(usize, Side),
    Ignore(usize),
}

/// The three-way merge of one file.
fn file_view(app: &mut DotCraftApp, ui: &mut Ui) {
    let p = rc_kit::pal(ui);
    let dark = p.dark;
    let full = ui.max_rect();
    let Some(m) = &mut app.vcs.merge else { return };
    let Some(f) = &mut m.open else { return };
    let status = if f.model.unresolved() == 0 {
        tr("All changes processed").to_string()
    } else {
        trf("Unresolved changes: {changes} · conflicts: {conflicts}", &[("changes", &f.model.unresolved()), ("conflicts", &f.model.unresolved_conflicts())])
    };
    let title = f.path.clone();
    let (back, close) = head(ui, full, &title, &status, true);
    if close {
        app.vcs.merge = None;
        app.focus_editor = true;
        return;
    }
    if back {
        m.close_file();
        return;
    }
    let busy = m.busy.is_some();
    let (ours_name, theirs_name) = m.sides.clone();
    // Toolbar: apply non-conflicting changes, previous / next change.
    let bar = Rect::from_min_size(pos2(full.left(), full.top() + HEAD_H), vec2(full.width(), 34.0));
    let mut bar_ui = ui.new_child(egui::UiBuilder::new().max_rect(bar));
    let mut jump: Option<bool> = None;
    kit::toolbar(&mut bar_ui, |ui| {
        if theme::icon_button(ui, ico::APPLY_NON_CONFLICTS_LEFT, true, tr("Apply Non-Conflicting Changes from the Left Side")).clicked() {
            f.apply_non_conflicting(Some(Side::Ours));
        }
        if theme::icon_button(ui, ico::APPLY_NON_CONFLICTS, true, tr("Apply All Non-Conflicting Changes")).clicked() {
            f.apply_non_conflicting(None);
        }
        if theme::icon_button(ui, ico::APPLY_NON_CONFLICTS_RIGHT, true, tr("Apply Non-Conflicting Changes from the Right Side")).clicked() {
            f.apply_non_conflicting(Some(Side::Theirs));
        }
        theme::toolbar_separator(ui);
        if theme::icon_button(ui, ico::UP, true, tr("Previous Change")).clicked() {
            jump = Some(false);
        }
        if theme::icon_button(ui, ico::DOWN, true, tr("Next Change")).clicked() {
            jump = Some(true);
        }
    });
    let row_h = f.result.row_height();
    if let Some(forward) = jump
        && row_h > 0.0
    {
        let top = (f.result.scroll_offset().y / row_h).round() as usize;
        if let Some(i) = f.next_unresolved(top + 3, forward) {
            let line = f.model.result_range(i).start;
            let x = f.result.scroll_offset().x;
            f.result.set_scroll_offset(vec2(x, (line.saturating_sub(3)) as f32 * row_h));
        }
    }
    // Column titles.
    let titles = Rect::from_min_size(pos2(full.left(), bar.bottom()), vec2(full.width(), TITLES_H));
    let bottom = Rect::from_min_max(pos2(full.left(), full.bottom() - BOTTOM_H), full.max);
    let body = Rect::from_min_max(pos2(full.left(), titles.bottom()), pos2(full.right(), bottom.top()));
    let w = ((body.width() - 2.0 * GUTTER_W) / 3.0).floor();
    let left = Rect::from_min_size(body.min, vec2(w, body.height()));
    let g1 = Rect::from_min_size(pos2(left.right(), body.top()), vec2(GUTTER_W, body.height()));
    let center = Rect::from_min_size(pos2(g1.right(), body.top()), vec2(w, body.height()));
    let g2 = Rect::from_min_size(pos2(center.right(), body.top()), vec2(GUTTER_W, body.height()));
    let right = Rect::from_min_max(pos2(g2.right(), body.top()), body.max);
    ui.painter().rect_filled(titles, 0.0, p.header_bg);
    for (rect, text) in [
        (left, trf("Yours: {name}", &[("name", &ours_name)])),
        (center, tr("Result").to_string()),
        (right, trf("Theirs: {name}", &[("name", &theirs_name)])),
    ] {
        let mut job = egui::text::LayoutJob::simple_singleline(text, FontId::proportional(12.5), p.text);
        job.wrap.max_width = rect.width() - 20.0;
        job.wrap.max_rows = 1;
        job.wrap.break_anywhere = true;
        job.wrap.overflow_character = Some('…');
        let g = ui.painter().layout_job(job);
        ui.painter().galley(pos2(rect.left() + 10.0, titles.center().y - g.size().y / 2.0), g, p.text);
    }
    ui.painter().hline(full.x_range(), titles.bottom() - 0.5, egui::Stroke::new(1.0, p.island_border));
    for g in [g1, g2] {
        ui.painter().rect_filled(g, 0.0, p.header_bg);
        ui.painter().vline(g.left() + 0.5, body.y_range(), egui::Stroke::new(1.0, p.island_border));
        ui.painter().vline(g.right() - 0.5, body.y_range(), egui::Stroke::new(1.0, p.island_border));
    }
    // Scroll the three panes together: the pane under the pointer leads.
    if row_h > 0.0 {
        let pointer = ui.input(|i| i.pointer.hover_pos());
        let to_ours = merge::anchors(&f.model, Side::Ours);
        let to_theirs = merge::anchors(&f.model, Side::Theirs);
        let flip = |a: &[(usize, usize)]| a.iter().map(|(x, y)| (*y, *x)).collect::<Vec<_>>();
        let line = |e: &rc_editor::EditorState| e.scroll_offset().y / row_h;
        let center_line = if pointer.is_some_and(|pt| left.contains(pt)) {
            merge::map_line(&flip(&to_ours), line(&f.left))
        } else if pointer.is_some_and(|pt| right.contains(pt)) {
            merge::map_line(&flip(&to_theirs), line(&f.right))
        } else {
            line(&f.result)
        };
        let set = |e: &mut rc_editor::EditorState, l: f32| {
            let x = e.scroll_offset().x;
            e.set_scroll_offset(vec2(x, (l * row_h).max(0.0)));
        };
        set(&mut f.result, center_line);
        set(&mut f.left, merge::map_line(&to_ours, center_line));
        set(&mut f.right, merge::map_line(&to_theirs, center_line));
    }
    let config = app.editor_config.clone();
    for (rect, state) in [(left, &mut f.left), (center, &mut f.result), (right, &mut f.right)] {
        let mut eu = ui.new_child(egui::UiBuilder::new().max_rect(rect));
        eu.set_clip_rect(rect.intersect(ui.clip_rect()));
        rc_editor::EditorView::new(state, &config, &app.keymap).show(&mut eu);
    }
    f.sync(dark);
    // Bands and arrows between the panes.
    let row_h = f.result.row_height();
    let mut actions = Vec::new();
    if row_h > 0.0 {
        let y = |rect: Rect, e: &rc_editor::EditorState, l: usize| rect.top() + l as f32 * row_h - e.scroll_offset().y;
        let painter = ui.painter_at(body);
        for (i, c) in f.model.chunks.iter().enumerate() {
            let resolved = f.model.is_resolved(i);
            let rr = f.model.result_range(i);
            let (c0, c1) = (y(center, &f.result, rr.start), y(center, &f.result, rr.end));
            if !resolved && rr.is_empty() {
                painter.hline(center.x_range(), c0, egui::Stroke::new(2.0, merge::chunk_color(c, None, false, dark).gamma_multiply(1.8)));
            }
            for side in [Side::Ours, Side::Theirs] {
                if !c.changed_by(side) {
                    continue;
                }
                let (pane, gutter, state) = if side == Side::Ours { (left, g1, &f.left) } else { (right, g2, &f.right) };
                let sr = c.side(side);
                let (s0, s1) = (y(pane, state, sr.start), y(pane, state, sr.end));
                let color = merge::chunk_color(c, Some(side), resolved, dark);
                if sr.is_empty() {
                    painter.hline(pane.x_range(), s0, egui::Stroke::new(2.0, color.gamma_multiply(1.8)));
                }
                if !resolved {
                    // The band from the side's lines to the result's lines.
                    let (a0, a1, b0, b1) = if side == Side::Ours { (s0, s1, c0, c1) } else { (c0, c1, s0, s1) };
                    let pts = vec![pos2(gutter.left(), a0), pos2(gutter.right(), b0), pos2(gutter.right(), b1.max(b0 + 1.0)), pos2(gutter.left(), a1.max(a0 + 1.0))];
                    painter.add(egui::Shape::convex_polygon(pts, color, egui::Stroke::NONE));
                }
                let top = s0.max(body.top() + 2.0);
                if top > body.bottom() - 18.0 || s1.max(s0 + row_h) < body.top() {
                    continue;
                }
                let can = f.model.can_accept(i, side);
                let append = matches!(f.model.resolution(i), Resolution::Taken(_));
                let (arrow_x, ignore_x) = if side == Side::Ours { (gutter.left() + 3.0, gutter.left() + 21.0) } else { (gutter.right() - 19.0, gutter.right() - 37.0) };
                if can {
                    let (icon, label) = match (side, append) {
                        (Side::Ours, false) => (ico::APPLY_NON_CONFLICTS_LEFT, tr("Accept Left Change")),
                        (Side::Ours, true) => (ico::APPEND_LEFT, tr("Append Left Change")),
                        (Side::Theirs, false) => (ico::APPLY_NON_CONFLICTS_RIGHT, tr("Accept Right Change")),
                        (Side::Theirs, true) => (ico::APPEND_RIGHT, tr("Append Right Change")),
                    };
                    let id = egui::Id::new(("merge_accept", i, side == Side::Ours));
                    if gutter_button(ui, Rect::from_min_size(pos2(arrow_x, top), vec2(16.0, 16.0)), id, icon, label) {
                        actions.push(ChunkAction::Accept(i, side));
                    }
                }
                if !resolved {
                    let id = egui::Id::new(("merge_ignore", i, side == Side::Ours));
                    if gutter_button(ui, Rect::from_min_size(pos2(ignore_x, top), vec2(16.0, 16.0)), id, ico::IGNORE_CHANGE, tr("Ignore Change")) {
                        actions.push(ChunkAction::Ignore(i));
                    }
                }
            }
        }
    }
    for a in actions {
        match a {
            ChunkAction::Accept(i, side) => f.accept(i, side),
            ChunkAction::Ignore(i) => f.ignore(i),
        }
    }
    // Bottom: whole-file sides, Apply and Cancel.
    ui.painter().hline(full.x_range(), bottom.top() + 0.5, egui::Stroke::new(1.0, p.island_border));
    let unresolved = f.model.unresolved();
    let path = f.path.clone();
    let mut bu = ui.new_child(egui::UiBuilder::new().max_rect(bottom.shrink2(vec2(12.0, 8.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
    bu.spacing_mut().item_spacing.x = 6.0;
    let mut action = None;
    if kit::button(&mut bu, None, tr("Accept Left"), !busy).clicked() {
        action = Some(0);
    }
    if kit::button(&mut bu, None, tr("Accept Right"), !busy).clicked() {
        action = Some(1);
    }
    if m.confirm_apply && unresolved > 0 {
        bu.add_space(8.0);
        kit::message(&mut bu, &trf("{n} changes are not resolved. Click Apply again to save the result as it is.", &[("n", &unresolved)]), false);
    } else if let Some(e) = &m.error {
        bu.add_space(8.0);
        kit::message(&mut bu, e, true);
    }
    bu.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if theme::secondary_button(ui, tr("Cancel")).clicked() {
            action = Some(3);
        }
        if ui.add_enabled_ui(!busy, |ui| theme::primary_button(ui, tr("Apply"))).inner.clicked() {
            action = Some(2);
        }
    });
    match action {
        Some(0) => m.accept_side(vec![path], Side::Ours),
        Some(1) => m.accept_side(vec![path], Side::Theirs),
        Some(2) => {
            if unresolved > 0 && !m.confirm_apply {
                m.confirm_apply = true;
            } else {
                m.apply_result();
            }
        }
        Some(3) => m.close_file(),
        _ => {}
    }
}

#[cfg(test)]
#[path = "merge_tests.rs"]
pub(crate) mod tests;
