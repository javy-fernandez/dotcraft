//! Pull Requests tool window (Rider's GitHub / Azure DevOps pull requests): sign in with a
//! personal access token, the open pull requests on the left, the selected one on the right
//! (description, reviewers, files that open in the diff viewer, conversations with replies,
//! approve / request changes / merge), and the New Pull Request dialog.

use crate::app::DotCraftApp;
use crate::toolwindow::ToolWindow;
use crate::ui::dialogs;
use crate::ui::kit;
use crate::ui::toolwindows::empty_state;
use egui::{Context, FontId, Rect, Sense, Ui, pos2, vec2};
use rc_kit::i18n::{tr, trf};
use rc_kit::{ico, icons, theme};
use rc_vcs::hosting::{FileChange, HostKind, MergeMethod, PrDetails, PrState, PullRequest, Review, Thread, Vote};

pub fn window(app: &mut DotCraftApp, ui: &mut Ui) {
    let pr = &app.pull_requests;
    let signed_in = pr.signed_in();
    let status = match (&pr.repo, &pr.viewer) {
        (Some(r), Some(v)) => format!("{} · {}", r.display_name(), v.name),
        (Some(r), None) => r.display_name(),
        _ => String::new(),
    };
    let busy = pr.loading || pr.details_loading || pr.busy.is_some() || pr.connecting;
    let actions = [
        (ico::HIDE, tr("Hide"), true),
        (ico::REFRESH, tr("Refresh"), signed_in && !busy),
        (ico::ADD, tr("New Pull Request"), signed_in && app.vcs.repo.is_some()),
        (ico::EXTERNAL_LINK, tr("Open in Browser"), pr.repo.is_some()),
        (ico::EXIT, tr("Sign Out"), signed_in),
    ];
    let click = kit::header(ui, tr("Pull Requests"), &[], &status, &actions);
    match click.action {
        Some(0) => {
            app.toggle_tool(ToolWindow::PullRequests);
            return;
        }
        Some(1) => app.pull_requests.refresh(),
        Some(2) => open_create(app),
        Some(3) => {
            if let Some(url) = app.pull_requests.details.as_ref().map(|d| d.pr.web_url.clone()).or_else(|| app.pull_requests.repo.as_ref().map(|r| r.pulls_page())) {
                crate::ui::run_windows::open_url(&url);
            }
        }
        Some(4) => {
            if let Err(e) = app.pull_requests.sign_out() {
                app.notify(trf("Could not remove the token from the keychain: {reason}", &[("reason", &e)]), true);
            }
        }
        _ => {}
    }
    if busy {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
    }
    let pr = &app.pull_requests;
    if app.vcs.repo.is_none() && pr.repo.is_none() {
        empty_state(ui, ico::TW_PULL_REQUESTS, tr("No Git repository"), tr("The workspace is not inside a Git repository."));
        return;
    }
    if pr.detecting {
        empty_state(ui, ico::TW_PULL_REQUESTS, tr("Pull Requests"), tr("Looking for GitHub or Azure DevOps remotes…"));
        return;
    }
    if pr.repo.is_none() {
        empty_state(ui, ico::TW_PULL_REQUESTS, tr("No GitHub or Azure DevOps remote"), tr("Pull requests show up here when a remote of the repository is on GitHub or Azure DevOps."));
        return;
    }
    if !signed_in {
        sign_in(app, ui);
        return;
    }
    signed_in_view(app, ui);
}

fn open_create(app: &mut DotCraftApp) {
    let current = app.vcs.status.as_ref().and_then(|s| s.branch.head.clone());
    let branches = app.vcs.branches.clone();
    app.pull_requests.open_create(current, &branches);
}

// ------------------------------------------------------------------ sign in

fn sign_in(app: &mut DotCraftApp, ui: &mut Ui) {
    let Some(repo) = app.pull_requests.repo.clone() else { return };
    let p = rc_kit::pal(ui);
    let connecting = app.pull_requests.connecting;
    let mut action = None;
    kit::content(ui, |ui| {
        ui.add_space(6.0);
        ui.spacing_mut().item_spacing.y = 8.0;
        ui.horizontal(|ui| {
            icons::image(ui, ico::TW_PULL_REQUESTS, 16.0);
            ui.label(egui::RichText::new(trf("Sign in to {service}", &[("service", &repo.kind.name())])).font(theme::bold(14.0)));
            ui.label(egui::RichText::new(repo.display_name()).size(12.5).color(p.muted));
        });
        kit::note(ui, tr("DotCraft uses a personal access token to list, review and create pull requests. It is kept in the system keychain."));
        kit::note(
            ui,
            match repo.kind {
                HostKind::GitHub => tr("The token needs the repo scope (classic), or Pull requests and Contents read and write (fine-grained)."),
                HostKind::AzureDevOps => tr("The token needs the Code (Read & Write) scope of the organization."),
            },
        );
        let resp = ui
            .vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 3.0;
                let caption = ui.label(egui::RichText::new(tr("Personal access token")).size(12.0).color(p.muted));
                ui.add_enabled(!connecting, egui::TextEdit::singleline(&mut app.pull_requests.token_input).password(true).hint_text(tr("Paste the token")).desired_width(360.0).margin(vec2(6.0, 4.0))).labelled_by(caption.id)
            })
            .inner;
        let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let has = !app.pull_requests.token_input.trim().is_empty();
        ui.horizontal(|ui| {
            if ui.add_enabled_ui(has && !connecting, |ui| theme::primary_button(ui, tr("Sign In"))).inner.clicked() || enter && has && !connecting {
                action = Some(0);
            }
            if kit::button(ui, Some(ico::EXTERNAL_LINK), tr("Create a Token…"), true).clicked() {
                action = Some(1);
            }
            if repo.kind == HostKind::GitHub && kit::button(ui, Some(ico::IMPORT), tr("Use GitHub CLI Login"), !connecting).clicked() {
                action = Some(2);
            }
        });
        if connecting {
            kit::note(ui, tr("Checking the token…"));
        }
        if let Some(e) = &app.pull_requests.error {
            kit::message(ui, e, true);
        }
    });
    match action {
        Some(0) => app.pull_requests.sign_in(),
        Some(1) => crate::ui::run_windows::open_url(&repo.token_page()),
        Some(2) => app.pull_requests.sign_in_with_cli(),
        _ => {}
    }
}

// ------------------------------------------------------------------ list and details

fn state_label(pr: &PullRequest) -> Option<&'static str> {
    match pr.state {
        PrState::Draft => Some(tr("Draft")),
        PrState::Merged => Some(tr("Merged")),
        PrState::Closed => Some(tr("Closed")),
        PrState::Open if pr.mergeable == Some(false) => Some(tr("Conflicts")),
        PrState::Open => None,
    }
}

fn vote_label(v: Vote) -> &'static str {
    match v {
        Vote::None => tr("Review requested"),
        Vote::Commented => tr("Commented"),
        Vote::Approved => tr("Approved"),
        Vote::ApprovedWithSuggestions => tr("Approved with suggestions"),
        Vote::ChangesRequested => tr("Changes requested"),
        Vote::Rejected => tr("Rejected"),
    }
}

fn date(iso: &str) -> &str {
    iso.get(..10).unwrap_or(iso)
}

fn signed_in_view(app: &mut DotCraftApp, ui: &mut Ui) {
    let count = app.pull_requests.list.len();
    kit::toolbar(ui, |ui| {
        kit::search(ui, &mut app.pull_requests.filter, tr("Filter by number, title, author or branch"), 280.0);
        kit::note(ui, &trf("{count} open", &[("count", &count)]));
        if app.pull_requests.loading {
            kit::note(ui, tr("Loading…"));
        }
    });
    if let Some(e) = app.pull_requests.error.clone() {
        kit::content(ui, |ui| kit::message(ui, &e, true));
    }
    let p = rc_kit::pal(ui);
    let avail = ui.available_rect_before_wrap();
    let left = Rect::from_min_size(avail.min, vec2((avail.width() * 0.4).clamp(280.0, 520.0), avail.height()));
    let right = Rect::from_min_max(pos2(left.right() + 1.0, avail.top()), avail.max);
    ui.painter().vline(left.right() + 0.5, avail.y_range(), egui::Stroke::new(1.0, p.island_border));
    let mut lu = ui.new_child(egui::UiBuilder::new().max_rect(left));
    lu.set_clip_rect(left.intersect(ui.clip_rect()));
    let mut select = None;
    let visible: Vec<PullRequest> = app.pull_requests.visible().into_iter().cloned().collect();
    let selected = app.pull_requests.selected;
    egui::ScrollArea::vertical().id_salt("pr_list").auto_shrink([false, false]).show_rows(&mut lu, 44.0, visible.len(), |ui, range| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for pr in &visible[range] {
            if list_row(ui, pr, selected == Some(pr.id)).clicked() {
                select = Some(pr.id);
            }
        }
    });
    if visible.is_empty() && !app.pull_requests.loading {
        lu.add_space(16.0);
        lu.vertical_centered(|ui| kit::note(ui, if count == 0 { tr("No open pull requests") } else { tr("No pull request matches the filter") }));
    }
    if let Some(id) = select {
        app.pull_requests.select(id);
    }
    let mut ru = ui.new_child(egui::UiBuilder::new().max_rect(right));
    ru.set_clip_rect(right.intersect(ui.clip_rect()));
    match app.pull_requests.details.clone() {
        Some(d) => details(app, &mut ru, &d),
        None if app.pull_requests.details_loading => {
            ru.add_space(20.0);
            ru.vertical_centered(|ui| kit::note(ui, tr("Loading…")));
        }
        None => {
            ru.add_space(20.0);
            ru.vertical_centered(|ui| kit::note(ui, tr("Select a pull request to see its changes and conversations")));
        }
    }
    ui.advance_cursor_after_rect(avail);
}

/// Two-line row: number and title, then author and branches.
fn list_row(ui: &mut Ui, pr: &PullRequest, selected: bool) -> egui::Response {
    let p = rc_kit::pal(ui);
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 44.0), Sense::click());
    if selected {
        ui.painter().rect_filled(r.shrink2(vec2(6.0, 1.0)), 4.0, p.selection);
    } else if resp.hovered() {
        ui.painter().rect_filled(r.shrink2(vec2(6.0, 1.0)), 4.0, p.row_hover);
    }
    let top = r.top() + 13.0;
    icons::paint(ui.painter(), Rect::from_center_size(pos2(r.left() + 22.0, top), vec2(16.0, 16.0)), ico::TW_PULL_REQUESTS, &p, pr.state != PrState::Draft);
    let x = r.left() + 38.0;
    let mut right_edge = r.right() - 12.0;
    if let Some(label) = state_label(pr) {
        let w = ui.painter().layout_no_wrap(label.to_string(), FontId::proportional(11.0), p.badge_fg).size().x + 12.0;
        right_edge -= w;
        theme::paint_badge(ui, pos2(right_edge, top), label);
        right_edge -= 6.0;
    }
    let mut job = egui::text::LayoutJob::default();
    job.append(&format!("#{}  ", pr.id), 0.0, egui::TextFormat::simple(FontId::proportional(13.0), p.muted));
    job.append(&pr.title, 0.0, egui::TextFormat::simple(FontId::proportional(13.0), p.text));
    job.wrap.max_width = (right_edge - x).max(40.0);
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('…');
    let g = ui.painter().layout_job(job);
    ui.painter().galley(pos2(x, (top - g.size().y / 2.0).round()), g, p.text);
    let mut job = egui::text::LayoutJob::simple_singleline(format!("{}  ·  {} → {}  ·  {}", pr.author, pr.source_branch, pr.target_branch, date(&pr.created)), FontId::proportional(12.0), p.muted);
    job.wrap.max_width = (r.right() - 12.0 - x).max(40.0);
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('…');
    let g = ui.painter().layout_job(job);
    ui.painter().galley(pos2(x, (r.top() + 31.0 - g.size().y / 2.0).round()), g, p.muted);
    let label = format!("#{} {}", pr.id, pr.title);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
    resp
}

fn details(app: &mut DotCraftApp, ui: &mut Ui, d: &PrDetails) {
    let p = rc_kit::pal(ui);
    let pr = &d.pr;
    let busy = app.pull_requests.busy.is_some();
    let kind = app.pull_requests.repo.as_ref().map(|r| r.kind).unwrap_or(HostKind::GitHub);
    egui::ScrollArea::vertical().id_salt(("pr_details", pr.id)).auto_shrink([false, false]).show(ui, |ui| {
        egui::Frame::new().inner_margin(egui::Margin { left: 12, right: 12, top: 6, bottom: 4 }).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 5.0;
            ui.horizontal_wrapped(|ui| {
                ui.label(egui::RichText::new(&pr.title).font(theme::bold(14.0)));
                ui.label(egui::RichText::new(format!("#{}", pr.id)).size(14.0).color(p.muted));
            });
            kit::note(ui, &trf("{author} wants to merge {source} into {target} · opened {date}", &[("author", &pr.author), ("source", &pr.source_branch), ("target", &pr.target_branch), ("date", &date(&pr.created))]));
            // State and reviewers as badges.
            let mut badges: Vec<String> = state_label(pr).map(|s| vec![s.to_string()]).unwrap_or_default();
            badges.extend(pr.reviewers.iter().map(|r| format!("{}: {}", r.name, vote_label(r.vote))));
            if !badges.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    for b in &badges {
                        let w = ui.painter().layout_no_wrap(b.clone(), FontId::proportional(11.0), p.badge_fg).size().x + 12.0;
                        let (r, resp) = ui.allocate_exact_size(vec2(w, 20.0), Sense::hover());
                        theme::paint_badge(ui, pos2(r.left(), r.center().y), b);
                        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, b));
                    }
                });
            }
            review_bar(app, ui, d, kind, busy);
        });
        // Description.
        let mut open = app.pull_requests.open;
        if kit::section(ui, ico::FILE_MARKDOWN, tr("Description"), "", &mut open[0]) {
            egui::Frame::new().inner_margin(egui::Margin { left: 24, right: 12, top: 2, bottom: 6 }).show(ui, |ui| {
                ui.set_width(ui.available_width());
                if pr.description.trim().is_empty() {
                    kit::note(ui, tr("No description provided."));
                } else {
                    egui_commonmark::CommonMarkViewer::new().show(ui, &mut app.md_cache, &pr.description);
                }
            });
        }
        // Files.
        let mut diff = None;
        if kit::section(ui, ico::DIFF, tr("Files"), &d.files.len().to_string(), &mut open[1]) {
            for f in &d.files {
                let name = f.path.rsplit('/').next().unwrap_or(&f.path);
                let dir = f.path.strip_suffix(name).map(|d| d.trim_end_matches('/')).unwrap_or_default();
                let change = match (f.change, &f.old_path, f.additions, f.deletions) {
                    (FileChange::Renamed, Some(old), _, _) => trf("renamed from {path}", &[("path", old)]),
                    (FileChange::Added, ..) => tr("added").to_string(),
                    (FileChange::Deleted, ..) => tr("deleted").to_string(),
                    (_, _, Some(a), Some(r)) => format!("+{a} −{r}"),
                    _ => String::new(),
                };
                let secondary = [dir, change.as_str()].into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join("  ·  ");
                let resp = kit::row(ui, 1, Some(icons::for_file(name)), name, &secondary, false).on_hover_text(tr("Show Diff"));
                if resp.clicked() {
                    diff = Some(f.clone());
                }
            }
        }
        if let Some(f) = diff {
            app.pull_requests.open_diff(&f);
        }
        // Conversations.
        if kit::section(ui, ico::BALLOON, tr("Conversations"), &d.threads.len().to_string(), &mut open[2]) {
            egui::Frame::new().inner_margin(egui::Margin { left: 24, right: 12, top: 2, bottom: 6 }).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 6.0;
                if d.threads.is_empty() {
                    kit::note(ui, tr("No comments yet."));
                }
                for t in &d.threads {
                    thread(app, ui, t, busy);
                }
                // A new comment.
                ui.add_space(4.0);
                ui.add_enabled(!busy, egui::TextEdit::multiline(&mut app.pull_requests.new_comment).hint_text(tr("Leave a comment")).desired_rows(3).desired_width(f32::INFINITY).margin(vec2(6.0, 4.0)));
                let can = !busy && !app.pull_requests.new_comment.trim().is_empty();
                if ui.add_enabled_ui(can, |ui| theme::primary_button(ui, tr("Comment"))).inner.clicked() {
                    app.pull_requests.add_comment();
                }
            });
        }
        app.pull_requests.open = open;
    });
}

fn review_bar(app: &mut DotCraftApp, ui: &mut Ui, d: &PrDetails, kind: HostKind, busy: bool) {
    let pr = &d.pr;
    let perms = d.permissions;
    let methods: Vec<String> = MergeMethod::ALL.iter().map(|m| method_label(*m, kind).to_string()).collect();
    let mut act = None;
    ui.add_space(2.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        if perms.can_review {
            kit::search(ui, &mut app.pull_requests.review_body, tr("Review comment (optional)"), 200.0);
            if kit::button(ui, Some(ico::CHECKMARK), tr("Approve"), !busy).clicked() {
                act = Some(0);
            }
            let label = if kind == HostKind::AzureDevOps { tr("Wait for Author") } else { tr("Request Changes") };
            if kit::button(ui, Some(ico::REVERT), label, !busy).clicked() {
                act = Some(1);
            }
        }
    });
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        if pr.is_open() {
            let current = method_label(app.pull_requests.merge_method, kind);
            if let Some(i) = kit::dropdown(ui, "pr_merge_method", current, &methods, perms.can_merge && !busy) {
                app.pull_requests.merge_method = MergeMethod::ALL[i];
            }
            let label = if kind == HostKind::AzureDevOps { tr("Complete") } else { tr("Merge") };
            if kit::button(ui, Some(ico::MERGE), label, perms.can_merge && !busy).clicked() {
                act = Some(2);
            }
        }
        if kit::button(ui, Some(ico::EXTERNAL_LINK), tr("Open in Browser"), true).clicked() {
            crate::ui::run_windows::open_url(&pr.web_url);
        }
        if let Some(what) = app.pull_requests.busy {
            kit::note(ui, match what {
                "merge" => tr("Merging…"),
                "review" => tr("Sending review…"),
                _ => tr("Sending…"),
            });
        }
    });
    if pr.is_open() && !perms.can_merge {
        let why = if pr.state == PrState::Draft {
            tr("Drafts can't be merged.")
        } else if pr.mergeable == Some(false) {
            tr("This pull request has conflicts with its target branch.")
        } else {
            tr("You don't have permission to merge this pull request.")
        };
        kit::note(ui, why);
    }
    match act {
        Some(0) => app.pull_requests.review(Review::Approve),
        Some(1) => app.pull_requests.review(Review::RequestChanges),
        Some(2) => app.pull_requests.confirm_merge = true,
        _ => {}
    }
    if app.pull_requests.confirm_merge {
        let question = trf("Merge #{id} into {target}?", &[("id", &pr.id), ("target", &pr.target_branch)]);
        let detail = trf("{method}: {title}", &[("method", &method_label(app.pull_requests.merge_method, kind)), ("title", &pr.title)]);
        match confirm_box(ui, &question, &detail, if kind == HostKind::AzureDevOps { tr("Complete") } else { tr("Merge") }, !busy) {
            Some(true) => app.pull_requests.merge(),
            Some(false) => app.pull_requests.confirm_merge = false,
            None => {}
        }
    }
}

fn method_label(m: MergeMethod, kind: HostKind) -> &'static str {
    match (m, kind) {
        (MergeMethod::Merge, HostKind::GitHub) => tr("Create a merge commit"),
        (MergeMethod::Merge, HostKind::AzureDevOps) => tr("Merge (no fast-forward)"),
        (MergeMethod::Squash, _) => tr("Squash and merge"),
        (MergeMethod::Rebase, _) => tr("Rebase and merge"),
    }
}

fn confirm_box(ui: &mut Ui, question: &str, detail: &str, confirm: &str, enabled: bool) -> Option<bool> {
    let p = rc_kit::pal(ui);
    let mut result = None;
    egui::Frame::new().outer_margin(egui::Margin { left: 10, right: 10, top: 4, bottom: 6 }).inner_margin(egui::Margin::same(12)).fill(p.header_bg).stroke(egui::Stroke::new(1.0, p.island_border)).corner_radius(6).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 6.0;
        ui.horizontal(|ui| {
            icons::image(ui, ico::DIALOG_QUESTION, 16.0);
            ui.label(egui::RichText::new(question).font(theme::bold(13.0)));
        });
        ui.label(egui::RichText::new(detail).size(12.5).color(p.muted));
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

fn thread(app: &mut DotCraftApp, ui: &mut Ui, t: &Thread, busy: bool) {
    let p = rc_kit::pal(ui);
    egui::Frame::new().inner_margin(egui::Margin::same(10)).fill(p.header_bg).stroke(egui::Stroke::new(1.0, p.island_border)).corner_radius(6).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 4.0;
        if t.path.is_some() || t.resolved {
            ui.horizontal(|ui| {
                if let Some(path) = &t.path {
                    let name = path.rsplit('/').next().unwrap_or(path);
                    icons::image(ui, icons::for_file(name), 16.0);
                    let at = match t.line {
                        Some(l) => format!("{path}:{l}"),
                        None => path.clone(),
                    };
                    ui.label(egui::RichText::new(at).size(12.0).color(p.muted));
                }
                if t.resolved {
                    let label = tr("Resolved");
                    let w = ui.painter().layout_no_wrap(label.to_string(), FontId::proportional(11.0), p.badge_fg).size().x + 12.0;
                    let (r, _) = ui.allocate_exact_size(vec2(w, 18.0), Sense::hover());
                    theme::paint_badge(ui, pos2(r.left(), r.center().y), label);
                }
            });
        }
        for (i, c) in t.comments.iter().enumerate() {
            if i > 0 {
                ui.add_space(2.0);
            }
            ui.horizontal(|ui| {
                icons::image(ui, ico::USER, 16.0);
                ui.label(egui::RichText::new(&c.author).font(theme::bold(12.5)));
                ui.label(egui::RichText::new(date(&c.created)).size(12.0).color(p.muted));
            });
            egui::Frame::new().inner_margin(egui::Margin { left: 22, right: 0, top: 0, bottom: 0 }).show(ui, |ui| {
                ui.label(egui::RichText::new(c.body.trim()).size(12.5));
            });
        }
        let replying = app.pull_requests.reply.as_ref().is_some_and(|(id, _)| *id == t.id);
        if replying {
            let mut send = false;
            let mut cancel = false;
            if let Some((_, text)) = &mut app.pull_requests.reply {
                ui.add_enabled(!busy, egui::TextEdit::multiline(text).hint_text(tr("Reply")).desired_rows(2).desired_width(f32::INFINITY).margin(vec2(6.0, 4.0)));
                let can = !busy && !text.trim().is_empty();
                ui.horizontal(|ui| {
                    send = ui.add_enabled_ui(can, |ui| theme::primary_button(ui, tr("Reply"))).inner.clicked();
                    cancel = theme::secondary_button(ui, tr("Cancel")).clicked();
                });
            }
            if send {
                app.pull_requests.send_reply();
            } else if cancel {
                app.pull_requests.reply = None;
            }
        } else if kit::button(ui, Some(ico::BALLOON), tr("Reply…"), !busy).clicked() {
            app.pull_requests.reply = Some((t.id.clone(), String::new()));
        }
    });
}

// ------------------------------------------------------------------ New Pull Request

/// The New Pull Request dialog (shown over the window while open).
pub fn create_dialog(app: &mut DotCraftApp, ctx: &Context) {
    let Some(mut f) = app.pull_requests.create.take() else { return };
    let repo = app.pull_requests.repo.as_ref().map(|r| r.display_name()).unwrap_or_default();
    let vcs_busy = app.vcs.busy.is_some();
    let mut push = false;
    let ((ok, cancel), esc) = dialogs::modal(ctx, "new_pull_request", 520.0, |ui| {
        let p = rc_kit::pal(ui);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(tr("New Pull Request")).font(theme::bold(15.0)));
            ui.label(egui::RichText::new(&repo).size(12.5).color(p.muted));
        });
        ui.add_space(10.0);
        ui.spacing_mut().item_spacing.y = 8.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            kit::note(ui, tr("From"));
            if let Some(i) = kit::dropdown(ui, "pr_source", &f.source, &f.sources, !f.busy) {
                f.source = f.sources[i].clone();
                f.unpushed = !crate::pull_requests::is_pushed(&f.source, &app.vcs.branches);
            }
            icons::image(ui, ico::ARROW_RIGHT, 16.0);
            kit::note(ui, tr("into"));
            if let Some(i) = kit::dropdown(ui, "pr_target", &f.target, &f.targets, !f.busy) {
                f.target = f.targets[i].clone();
            }
        });
        if f.unpushed {
            ui.horizontal(|ui| {
                kit::message(ui, &trf("{branch} is not on the remote yet.", &[("branch", &f.source)]), false);
                push = kit::button(ui, Some(ico::PUSH), tr("Push Branch"), !vcs_busy && !f.source.is_empty()).clicked();
            });
        }
        if f.source == f.target {
            kit::message(ui, tr("Pick a target branch other than the source."), false);
        }
        let w = ui.available_width();
        kit::field(ui, tr("Title"), &mut f.title, tr("What does this change?"), w, !f.busy);
        let caption = ui.label(egui::RichText::new(tr("Description")).size(12.0).color(p.muted));
        ui.add_enabled(!f.busy, egui::TextEdit::multiline(&mut f.description).hint_text(tr("Markdown is supported")).desired_rows(6).desired_width(f32::INFINITY).margin(vec2(6.0, 4.0))).labelled_by(caption.id);
        ui.add_enabled(!f.busy, egui::Checkbox::new(&mut f.draft, tr("Create as draft")));
        if let Some(e) = &f.error {
            kit::message(ui, e, true);
        }
        let valid = !f.busy && !f.title.trim().is_empty() && !f.source.is_empty() && !f.unpushed && f.source != f.target;
        dialogs::buttons(ui, if f.busy { tr("Creating…") } else { tr("Create") }, valid)
    });
    if push {
        app.vcs.push_branch(&f.source);
    }
    let branches = app.vcs.branches.clone();
    let unpushed = f.unpushed;
    if ok {
        app.pull_requests.create = Some(f);
        app.pull_requests.create_pr();
    } else if !(cancel || esc) {
        app.pull_requests.create = Some(f);
        // After Push Branch, the remote branch shows up with the next branch list.
        if unpushed {
            app.pull_requests.check_pushed(&branches);
        }
    }
}
