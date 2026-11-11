//! Interactive rebase (Rider's "Interactively Rebase from Here"): a dialog listing the commits
//! from the chosen one up to HEAD, oldest first, where commits are reordered by drag and drop
//! (or Alt+Up/Down) and get an action (pick, edit, reword, squash, fixup, drop). While the rebase
//! is stopped (conflicts, an edit step) the Git and Commit windows show a banner with the
//! conflicted files and Continue / Skip / Abort.

use crate::app::DotCraftApp;
use crate::ui::kit;
use egui::text::{LayoutJob, TextFormat};
use egui::{Align2, Color32, Context, FontId, Id, Key, Rect, Sense, Stroke, Ui, pos2, vec2};
use rc_kit::i18n::{tr, trf};
use rc_kit::{ico, icons, theme};
use rc_vcs::{PlanProblem, RebaseAction, RebasePlan, RebaseStop};

const ROW_H: f32 = 30.0;

/// State of the interactive rebase dialog.
pub struct RebaseDialog {
    /// The commit the rebase starts from (included).
    pub from: String,
    plan: Option<RebasePlan>,
    /// The hashes in their original order (to tell whether anything changed).
    original: Vec<String>,
    error: Option<String>,
    pub selected: usize,
    /// The row being dragged and the insertion index (0..=len) under the pointer.
    drag: Option<(usize, usize)>,
}

impl RebaseDialog {
    pub fn new(from: &str) -> RebaseDialog {
        RebaseDialog { from: from.to_string(), plan: None, original: Vec::new(), error: None, selected: 0, drag: None }
    }

    /// The plan as currently edited (`None` while it loads).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn plan(&self) -> Option<&RebasePlan> {
        self.plan.as_ref()
    }

    /// Moves step `from` so it ends up before the step at insertion index `to` (0..=len).
    pub fn move_step(&mut self, from: usize, to: usize) {
        let Some(plan) = &mut self.plan else { return };
        if from >= plan.steps.len() || to > plan.steps.len() {
            return;
        }
        let step = plan.steps.remove(from);
        let at = if to > from { to - 1 } else { to };
        plan.steps.insert(at, step);
        self.selected = at;
    }

    /// Sets the action of step `i`; a reword starts from the commit's own message.
    pub fn set_action(&mut self, i: usize, action: RebaseAction) {
        let Some(step) = self.plan.as_mut().and_then(|p| p.steps.get_mut(i)) else { return };
        step.action = action;
        if action == RebaseAction::Reword && step.message.is_none() {
            step.message = Some(step.original_message());
        }
    }
}

/// "Interactively Rebase from Here…" on a commit of the log.
pub fn open(app: &mut DotCraftApp, hash: &str) {
    app.vcs.load_rebase_plan(hash);
    app.dialog = Some(crate::ui::dialogs::Dialog::InteractiveRebase(Box::new(RebaseDialog::new(hash))));
}

pub fn action_label(a: RebaseAction) -> &'static str {
    match a {
        RebaseAction::Pick => tr("Pick"),
        RebaseAction::Reword => tr("Reword"),
        RebaseAction::Edit => tr("Edit"),
        RebaseAction::Squash => tr("Squash"),
        RebaseAction::Fixup => tr("Fixup"),
        RebaseAction::Drop => tr("Drop"),
    }
}

fn action_key(a: RebaseAction) -> Key {
    match a {
        RebaseAction::Pick => Key::P,
        RebaseAction::Reword => Key::R,
        RebaseAction::Edit => Key::E,
        RebaseAction::Squash => Key::S,
        RebaseAction::Fixup => Key::F,
        RebaseAction::Drop => Key::D,
    }
}

fn action_color(a: RebaseAction, p: &theme::Palette) -> Color32 {
    let rgb = |c: u32| Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8);
    match (a, p.dark) {
        (RebaseAction::Pick, _) => p.text,
        (RebaseAction::Reword | RebaseAction::Edit, true) => rgb(0x6F9EF7),
        (RebaseAction::Reword | RebaseAction::Edit, false) => rgb(0x0A54C9),
        (RebaseAction::Squash | RebaseAction::Fixup, true) => rgb(0xC77DBB),
        (RebaseAction::Squash | RebaseAction::Fixup, false) => rgb(0x871094),
        (RebaseAction::Drop, _) => p.muted,
    }
}

fn problem_text(p: PlanProblem) -> &'static str {
    match p {
        PlanProblem::NothingToRebase => tr("Every commit is dropped: keep at least one"),
        PlanProblem::MeldWithoutTarget => tr("The first kept commit cannot be squashed or fixed up"),
        PlanProblem::EmptyMessage => tr("A reworded commit needs a message"),
    }
}

/// The dialog; returns true when it closes.
pub fn dialog(app: &mut DotCraftApp, ctx: &Context, d: &mut RebaseDialog) -> bool {
    if d.plan.is_none() && d.error.is_none()
        && let Some((from, loaded)) = app.vcs.rebase_plan.take()
    {
        if from == d.from {
            match loaded {
                Ok(plan) => {
                    d.original = plan.steps.iter().map(|s| s.commit.hash.clone()).collect();
                    d.plan = Some(plan);
                }
                Err(e) => d.error = Some(e),
            }
        } else {
            app.vcs.rebase_plan = Some((from, loaded));
        }
    }
    let width = (ctx.content_rect().width() - 48.0).clamp(320.0, 860.0);
    let mut start = false;
    let mut cancel = false;
    let modal = egui::Modal::new(Id::new("interactive_rebase")).show(ctx, |ui| {
        ui.set_width(width);
        let p = rc_kit::pal(ui);
        let title = match &d.plan {
            Some(plan) => {
                let base = plan.base.as_deref().map_or_else(|| tr("the root").to_string(), |b| b[..b.len().min(8)].to_string());
                trf("Rebasing {n} commits onto {base}", &[("n", &plan.steps.len()), ("base", &base)])
            }
            None => tr("Interactive Rebase").to_string(),
        };
        ui.label(egui::RichText::new(title).font(theme::bold(15.0)));
        ui.add_space(8.0);
        if let Some(e) = &d.error {
            kit::message(ui, e, true);
        } else if d.plan.is_none() {
            kit::note(ui, tr("Loading…"));
        } else {
            toolbar(ui, d);
            ui.add_space(4.0);
            commit_list(ui, d, &p);
            message_editor(ui, d);
            keyboard(ui, d);
        }
        ui.add_space(12.0);
        let problem = d.plan.as_ref().and_then(RebasePlan::validate);
        let unchanged = d.plan.as_ref().is_none_or(|plan| plan.is_unchanged(&d.original));
        ui.horizontal(|ui| {
            match problem {
                Some(pr) if d.plan.is_some() => kit::message(ui, problem_text(pr), true),
                _ if d.plan.is_some() => kit::note(ui, tr("Drag commits to reorder them; P E R S F D set the action of the selected one.")),
                _ => {}
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let enabled = problem.is_none() && !unchanged && app.vcs.busy.is_none();
                start = ui.add_enabled_ui(enabled, |ui| theme::primary_button(ui, tr("Start Rebasing"))).inner.clicked();
                cancel = theme::secondary_button(ui, tr("Cancel")).clicked();
            });
        });
    });
    if start
        && let Some(plan) = d.plan.clone()
    {
        if !crate::ui::git::save_before_branch_change(app) {
            return true;
        }
        app.vcs.interactive_rebase(plan);
        return true;
    }
    cancel || modal.should_close() && d.drag.is_none()
}

fn toolbar(ui: &mut Ui, d: &mut RebaseDialog) {
    let n = d.plan.as_ref().map_or(0, |p| p.steps.len());
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        let i = d.selected;
        if theme::icon_button(ui, ico::UP, i > 0, tr("Move Up (Alt+Up)")).clicked() && i > 0 {
            d.move_step(i, i - 1);
        }
        if theme::icon_button(ui, ico::DOWN, i + 1 < n, tr("Move Down (Alt+Down)")).clicked() && i + 1 < n {
            d.move_step(i, i + 2);
        }
        theme::toolbar_separator(ui);
        let current = d.plan.as_ref().and_then(|p| p.steps.get(i)).map(|s| s.action);
        for a in RebaseAction::ALL {
            let label = action_label(a);
            let r = theme::tool_button(ui, action_icon(a), label, current == Some(a));
            if r.clicked() {
                d.set_action(i, a);
            }
        }
    });
}

fn action_icon(a: RebaseAction) -> rc_kit::Ico {
    match a {
        RebaseAction::Pick => ico::COMMIT,
        RebaseAction::Reword => ico::EDIT,
        RebaseAction::Edit => ico::PAUSE,
        RebaseAction::Squash => ico::MERGE,
        RebaseAction::Fixup => ico::COLLAPSE_ALL,
        RebaseAction::Drop => ico::DELETE,
    }
}

/// The commits, oldest first, with drag-and-drop reordering.
fn commit_list(ui: &mut Ui, d: &mut RebaseDialog, p: &theme::Palette) {
    let Some(plan) = d.plan.clone() else { return };
    let n = plan.steps.len();
    let height = (n as f32 * ROW_H).min(ROW_H * 12.0);
    let frame = egui::Frame::new().stroke(Stroke::new(1.0, p.island_border)).corner_radius(6).inner_margin(egui::Margin::same(4));
    frame.show(ui, |ui| {
        egui::ScrollArea::vertical().id_salt("rebase_commits").max_height(height).auto_shrink([false, true]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let top = ui.cursor().top();
            let mut picked: Option<(usize, RebaseAction)> = None;
            let mut drop_now: Option<(usize, usize)> = None;
            for (i, s) in plan.steps.iter().enumerate() {
                let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click_and_drag());
                let dragged = d.drag.is_some_and(|(f, _)| f == i);
                if i == d.selected {
                    ui.painter().rect_filled(r, 4.0, p.selection);
                } else if resp.hovered() {
                    ui.painter().rect_filled(r, 4.0, p.row_hover);
                }
                let fade = |c: Color32| if dragged { c.gamma_multiply(0.45) } else { c };
                // Drag handle: two columns of dots.
                for row in 0..3 {
                    for col in 0..2 {
                        ui.painter().circle_filled(pos2(r.left() + 9.0 + col as f32 * 4.0, r.center().y - 4.0 + row as f32 * 4.0), 1.1, fade(p.muted));
                    }
                }
                // Action drop-down.
                let ar = Rect::from_min_size(pos2(r.left() + 22.0, r.center().y - 13.0), vec2(92.0, 26.0));
                let mut au = ui.new_child(egui::UiBuilder::new().max_rect(ar));
                let labels: Vec<String> = RebaseAction::ALL.iter().map(|a| action_label(*a).to_string()).collect();
                if let Some(k) = kit::dropdown(&mut au, ("rebase_action", i), action_label(s.action), &labels, true) {
                    picked = Some((i, RebaseAction::ALL[k]));
                }
                let mut x = ar.right() + 8.0;
                let hash = ui.painter().text(pos2(x, r.center().y), Align2::LEFT_CENTER, s.commit.short_hash(), FontId::monospace(12.0), fade(p.muted));
                x = hash.right() + 12.0;
                if s.action.melds() {
                    // A corner joining the commit to the one above.
                    let c = fade(action_color(s.action, p));
                    ui.painter().line_segment([pos2(x + 2.0, r.top() + 4.0), pos2(x + 2.0, r.center().y)], Stroke::new(1.2, c));
                    ui.painter().line_segment([pos2(x + 2.0, r.center().y), pos2(x + 10.0, r.center().y)], Stroke::new(1.2, c));
                    x += 16.0;
                }
                let meta = format!("{}  {}", s.commit.author_name, crate::vcs::format_date(s.commit.author_time));
                let mg = ui.painter().layout_no_wrap(meta, FontId::proportional(12.0), fade(p.muted));
                let subject = match (&s.message, s.action) {
                    (Some(m), RebaseAction::Reword) => m.lines().next().unwrap_or_default().to_string(),
                    _ => s.commit.subject.clone(),
                };
                let color = fade(if s.action == RebaseAction::Pick { p.text } else { action_color(s.action, p) });
                let mut format = TextFormat::simple(FontId::proportional(13.0), color);
                if s.action == RebaseAction::Drop {
                    format.strikethrough = Stroke::new(1.0, color);
                }
                let mut job = LayoutJob::single_section(subject.clone(), format);
                job.wrap.max_width = (r.right() - mg.size().x - 20.0 - x).max(40.0);
                job.wrap.max_rows = 1;
                job.wrap.break_anywhere = true;
                job.wrap.overflow_character = Some('…');
                let g = ui.painter().layout_job(job);
                ui.painter().galley(pos2(x, (r.center().y - g.size().y / 2.0).round()), g, color);
                ui.painter().galley(pos2(r.right() - mg.size().x - 8.0, (r.center().y - mg.size().y / 2.0).round()), mg, fade(p.muted));
                resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, i == d.selected, &s.commit.subject));
                if resp.clicked() || resp.drag_started() {
                    d.selected = i;
                }
                if resp.drag_started() {
                    d.drag = Some((i, i));
                }
                if resp.dragged()
                    && let Some(pt) = ui.ctx().pointer_interact_pos()
                {
                    let to = (((pt.y - top) / ROW_H) + 0.5).floor().clamp(0.0, n as f32) as usize;
                    d.drag = Some((i, to));
                }
                if resp.drag_stopped()
                    && let Some((from, to)) = d.drag.take()
                {
                    drop_now = Some((from, to));
                }
            }
            // The insertion line where the dragged commit would land.
            if let Some((from, to)) = d.drag
                && to != from
                && to != from + 1
            {
                let y = top + to as f32 * ROW_H;
                let left = ui.min_rect().left();
                let right = ui.min_rect().right();
                ui.painter().line_segment([pos2(left + 4.0, y), pos2(right - 4.0, y)], Stroke::new(2.0, p.accent));
                ui.painter().circle_stroke(pos2(left + 4.0, y), 3.0, Stroke::new(1.5, p.accent));
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            }
            if let Some((i, a)) = picked {
                d.selected = i;
                d.set_action(i, a);
            }
            if let Some((from, to)) = drop_now {
                d.move_step(from, to);
            }
        });
    });
}

/// The message of the selected commit: editable when it is reworded, read-only otherwise.
/// Always shown, so selecting another row never resizes the dialog under the pointer.
fn message_editor(ui: &mut Ui, d: &mut RebaseDialog) {
    let i = d.selected;
    let Some(step) = d.plan.as_mut().and_then(|p| p.steps.get_mut(i)) else { return };
    ui.add_space(8.0);
    let p = rc_kit::pal(ui);
    let hash = step.commit.short_hash().to_string();
    let edit = |ui: &mut Ui, text: &mut dyn egui::TextBuffer, caption: Id| {
        ui.add(egui::TextEdit::multiline(text).desired_rows(4).desired_width(f32::INFINITY).margin(vec2(6.0, 4.0))).labelled_by(caption);
    };
    if step.action == RebaseAction::Reword {
        let caption = ui.label(egui::RichText::new(trf("New message of {commit}", &[("commit", &hash)])).size(12.0).color(p.muted));
        edit(ui, step.message.get_or_insert_with(String::new), caption.id);
    } else {
        let caption = ui.label(egui::RichText::new(trf("Message of {commit}", &[("commit", &hash)])).size(12.0).color(p.muted));
        let original = step.original_message();
        edit(ui, &mut original.as_str(), caption.id);
    }
}

/// Rider's shortcuts: P E R S F D set the action, Alt+Up/Down move, Up/Down select.
fn keyboard(ui: &mut Ui, d: &mut RebaseDialog) {
    if ui.ctx().egui_wants_keyboard_input() {
        return;
    }
    let n = d.plan.as_ref().map_or(0, |p| p.steps.len());
    let i = d.selected;
    ui.input_mut(|input| {
        if input.consume_key(egui::Modifiers::ALT, Key::ArrowUp) && i > 0 {
            d.move_step(i, i - 1);
        } else if input.consume_key(egui::Modifiers::ALT, Key::ArrowDown) && i + 1 < n {
            d.move_step(i, i + 2);
        } else if input.consume_key(egui::Modifiers::NONE, Key::ArrowUp) {
            d.selected = i.saturating_sub(1);
        } else if input.consume_key(egui::Modifiers::NONE, Key::ArrowDown) {
            d.selected = (i + 1).min(n.saturating_sub(1));
        }
        for a in RebaseAction::ALL {
            if input.consume_key(egui::Modifiers::NONE, action_key(a)) {
                d.set_action(i, a);
            }
        }
    });
}

// ------------------------------------------------------------------ paused rebase

/// Opens a conflicted file in the 3-way merge tool; Apply stages it, and the rebase then
/// continues from the banner.
pub fn resolve_in_merge_tool(app: &mut DotCraftApp, path: &str) {
    app.vcs.open_merge_file(path);
}

/// The banner of a stopped rebase (Git and Commit windows): where it stopped, the conflicted
/// files and Continue / Skip / Abort.
pub fn paused_banner(app: &mut DotCraftApp, ui: &mut Ui) {
    if app.vcs.state != rc_vcs::RepoState::Rebasing {
        return;
    }
    let Some(st) = app.vcs.rebase.clone() else { return };
    let p = rc_kit::pal(ui);
    let busy = app.vcs.busy.is_some();
    let fill = theme::blend(p.island_bg, p.warn.gamma_multiply(0.12));
    // Outer 8 + stroke 1 + inner 10 on each side.
    // Some windows lay out wider than they show: stay inside the visible (clip) area.
    let visible = ui.available_width().min(ui.clip_rect().right() - 8.0 - ui.cursor().left());
    let width = (visible - 38.0).max(120.0);
    let narrow = width < 560.0;
    egui::Frame::new().fill(fill).stroke(Stroke::new(1.0, p.warn.gamma_multiply(0.45))).corner_radius(6).inner_margin(egui::Margin::symmetric(10, 6)).outer_margin(egui::Margin::symmetric(8, 4)).show(ui, |ui| {
        ui.set_width(width);
        ui.spacing_mut().item_spacing.y = 4.0;
        ui.horizontal(|ui| {
            icons::image(ui, ico::PAUSE, 16.0);
            let title = match &st.branch {
                Some(b) => trf("Rebasing '{branch}'", &[("branch", &b)]),
                None => tr("Rebasing").to_string(),
            };
            ui.label(egui::RichText::new(title).font(theme::bold(13.0)));
            ui.label(egui::RichText::new(trf("step {step} of {total}", &[("step", &st.step), ("total", &st.total)])).size(12.0).color(p.muted));
        });
        let commit = st.commit.as_ref().map(|(h, s)| format!("{} {s}", &h[..h.len().min(8)])).unwrap_or_default();
        let reason = match st.stop {
            RebaseStop::Conflicts => trf("Stopped at {commit}: resolve the conflicts, then continue", &[("commit", &commit)]),
            RebaseStop::Edit => trf("Stopped to edit {commit}: amend it in the Commit window, then continue", &[("commit", &commit)]),
            RebaseStop::ExecFailed => tr("A step of the rebase failed; fix it, then continue").to_string(),
            RebaseStop::Other => trf("Stopped at {commit}", &[("commit", &commit)]),
        };
        ui.add(egui::Label::new(egui::RichText::new(reason).size(12.5)).wrap());
        let mut action: Option<(String, u8)> = None;
        for path in &st.conflicts {
            let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::click());
            if resp.hovered() {
                ui.painter().rect_filled(r, 4.0, p.row_hover);
            }
            let name = path.rsplit('/').next().unwrap_or(path);
            icons::paint(ui.painter(), Rect::from_center_size(pos2(r.left() + 12.0, r.center().y), vec2(16.0, 16.0)), icons::for_file(name), &p, true);
            let conflict = Color32::from_rgb(0xD5, 0x75, 0x6C);
            ui.painter().text(pos2(r.left() + 26.0, r.center().y), Align2::LEFT_CENTER, path, FontId::proportional(12.5), conflict);
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, path));
            if resp.double_clicked() {
                action = Some((path.clone(), 0));
            }
            // Per-file actions: right-aligned on the row, or wrapped under it when narrow.
            let buttons = [(0u8, Some(ico::MERGE), tr("Merge…")), (1, None, tr("Accept Rebased")), (2, None, tr("Accept Commit")), (3, None, tr("Mark Resolved"))];
            let mut add = |bu: &mut Ui, code: u8, icon: Option<rc_kit::Ico>, label: &str| {
                if kit::button(bu, icon, label, code == 0 || !busy).clicked() {
                    action = Some((path.clone(), code));
                }
            };
            if narrow {
                ui.horizontal_wrapped(|ui| {
                    ui.add_space(18.0);
                    for (code, icon, label) in buttons {
                        add(ui, code, icon, label);
                    }
                });
            } else {
                let br = Rect::from_min_max(pos2(r.center().x, r.top()), r.max);
                let mut bu = ui.new_child(egui::UiBuilder::new().max_rect(br).layout(egui::Layout::right_to_left(egui::Align::Center)));
                for (code, icon, label) in buttons.into_iter().rev() {
                    add(&mut bu, code, icon, label);
                }
            }
        }
        match action {
            Some((path, 0)) => resolve_in_merge_tool(app, &path),
            Some((path, 1)) => app.vcs.resolve_conflict(&path, false),
            Some((path, 2)) => app.vcs.resolve_conflict(&path, true),
            Some((path, _)) => {
                app.save_all_docs();
                app.vcs.mark_resolved(&path);
            }
            None => {}
        }
        let can_continue = !busy && st.conflicts.is_empty();
        let mut clicked = None;
        let mut buttons = |ui: &mut Ui, from: usize, to: usize| {
            for (i, label) in [tr("Continue Rebase"), tr("Skip Commit"), tr("Abort Rebase")].into_iter().enumerate().take(to).skip(from) {
                let enabled = if i == 0 { can_continue } else { !busy };
                let r = ui.add_enabled_ui(enabled, |ui| if i == 0 { theme::primary_button(ui, label) } else { theme::secondary_button(ui, label) }).inner;
                if r.clicked() {
                    clicked = Some(i);
                }
            }
            if busy && to == 3 {
                ui.add(egui::Spinner::new().size(14.0));
            }
        };
        if narrow {
            ui.horizontal(|ui| buttons(ui, 0, 1));
            ui.horizontal(|ui| buttons(ui, 1, 3));
        } else {
            ui.horizontal(|ui| buttons(ui, 0, 3));
        }
        match clicked {
            Some(0) => {
                app.save_all_docs();
                app.vcs.continue_rebase();
            }
            Some(1) => app.vcs.skip_rebase(),
            Some(_) => app.vcs.abort_operation(),
            None => {}
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dialog_with(n: usize) -> RebaseDialog {
        let steps = (0..n)
            .map(|i| rc_vcs::RebaseStep {
                commit: rc_vcs::Commit {
                    hash: format!("{i:040}"),
                    parents: vec![],
                    author_name: String::new(),
                    author_email: String::new(),
                    author_time: 0,
                    committer_name: String::new(),
                    committer_email: String::new(),
                    commit_time: 0,
                    refs: vec![],
                    head_branch: None,
                    subject: format!("c{i}"),
                    body: String::new(),
                },
                action: RebaseAction::Pick,
                message: None,
            })
            .collect();
        let mut d = RebaseDialog::new("x");
        d.plan = Some(RebasePlan { base: None, steps });
        d
    }

    fn order(d: &RebaseDialog) -> Vec<String> {
        d.plan().unwrap().steps.iter().map(|s| s.commit.subject.clone()).collect()
    }

    #[test]
    fn moves_steps_like_a_drop_between_rows() {
        let mut d = dialog_with(4);
        d.move_step(0, 3);
        assert_eq!(order(&d), ["c1", "c2", "c0", "c3"]);
        assert_eq!(d.selected, 2);
        d.move_step(3, 0);
        assert_eq!(order(&d), ["c3", "c1", "c2", "c0"]);
        d.move_step(1, 2); // dropped on itself
        assert_eq!(order(&d), ["c3", "c1", "c2", "c0"]);
        d.move_step(0, 4);
        assert_eq!(order(&d), ["c1", "c2", "c0", "c3"]);
    }

    #[test]
    fn reword_starts_from_the_original_message() {
        let mut d = dialog_with(2);
        d.set_action(1, RebaseAction::Reword);
        assert_eq!(d.plan().unwrap().steps[1].message.as_deref(), Some("c1"));
    }
}
