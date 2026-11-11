//! Work item links in the Git windows: underlined references (`#123`, `AB#123`, custom
//! patterns) that open in the browser, in the log, the commit details and the commit message
//! editor, plus the completion of ids after `#` / `AB#` in the message.

use crate::work_items::{WorkItems, open_link};
use egui::text::{CCursor, CCursorRange, LayoutJob};
use egui::{Align2, Color32, FontId, Galley, Id, Key, Modifiers, Order, Pos2, Rect, Response, Sense, Stroke, TextFormat, Ui, pos2, vec2};
use rc_kit::i18n::{tr, trf};
use rc_vcs::work_items::{CompletionQuery, Linker, WorkItem, WorkItemLink, complete, completion_query};
use std::ops::Range;
use std::sync::Arc;

/// The colour of links (the editor scheme's hyperlink colour).
pub fn link_color(ui: &Ui) -> Color32 {
    rc_kit::theme::scheme(ui.visuals().dark_mode).hyperlink
}

/// `text` laid out with its links underlined in the link colour.
pub fn job(text: &str, links: &[WorkItemLink], font: FontId, color: Color32, link: Color32) -> LayoutJob {
    let mut job = LayoutJob::default();
    let plain = TextFormat { font_id: font.clone(), color, ..Default::default() };
    let linked = TextFormat { font_id: font, color: link, underline: Stroke::new(1.0, link), ..Default::default() };
    let mut at = 0;
    for l in links {
        if l.range.start < at || l.range.end > text.len() {
            continue;
        }
        job.append(&text[at..l.range.start], 0.0, plain.clone());
        job.append(&text[l.range.clone()], 0.0, linked.clone());
        at = l.range.end;
    }
    job.append(&text[at..], 0.0, plain);
    job
}

fn char_index(text: &str, byte: usize) -> usize {
    text[..byte.min(text.len())].chars().count()
}

/// The on-screen rectangles of a byte range of a galley painted at `pos` (one per row).
fn range_rects(galley: &Galley, pos: Pos2, text: &str, range: &Range<usize>) -> Vec<Rect> {
    let (a, b) = (char_index(text, range.start), char_index(text, range.end));
    let start = galley.pos_from_cursor(CCursor::new(a));
    let end = galley.pos_from_cursor(CCursor::new(b));
    let rects = if (start.center().y - end.center().y).abs() < 1.0 {
        vec![Rect::from_min_max(start.min, end.max)]
    } else {
        let right = galley.rect.right();
        vec![Rect::from_min_max(start.min, pos2(right, start.max.y)), Rect::from_min_max(pos2(0.0, end.min.y), end.max)]
    };
    rects.into_iter().map(|r| r.translate(pos.to_vec2()).intersect(Rect::from_min_size(pos, galley.size()))).filter(|r| r.width() > 0.5 && r.height() > 0.5).collect()
}

/// Makes the links of a galley painted at `pos` clickable: hand cursor and URL on hover, a
/// click opens the browser. Each link is an AccessKit link labelled with its text. Returns
/// whether one was clicked.
pub fn interact_links(ui: &mut Ui, id: Id, galley: &Galley, pos: Pos2, text: &str, links: &[WorkItemLink]) -> bool {
    let mut clicked = false;
    let clip = ui.clip_rect();
    for (i, l) in links.iter().enumerate() {
        for (k, rect) in range_rects(galley, pos, text, &l.range).into_iter().enumerate() {
            if !clip.intersects(rect) {
                continue;
            }
            let resp = ui.interact(rect, id.with((i, k)), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
            let label = text[l.range.clone()].to_string();
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Link, true, &label));
            let resp = resp.on_hover_text(&l.url);
            if resp.clicked() {
                open_link(&l.url);
                clicked = true;
            }
        }
    }
    clicked
}

/// A wrapped text with clickable work item links (the commit details).
pub fn linked_text(ui: &mut Ui, linker: &Linker, text: &str, font: FontId, color: Color32) -> Response {
    let links = linker.find(text);
    let mut job = job(text, &links, font, color, link_color(ui));
    job.wrap.max_width = ui.available_width();
    let galley = ui.fonts_mut(|f| f.layout_job(job));
    let (rect, resp) = ui.allocate_exact_size(galley.size(), Sense::hover());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, text));
    ui.painter().galley(rect.min, galley.clone(), color);
    interact_links(ui, resp.id, &galley, rect.min, text, &links);
    resp
}

/// One line of 13 pt text, cut with `…` at `max_width`, with clickable links (the log's
/// subjects). Returns whether a link was clicked.
pub fn linked_line(ui: &mut Ui, linker: &Linker, id: Id, text: &str, left_center: Pos2, max_width: f32, color: Color32) -> bool {
    let links = linker.find(text);
    let mut job = job(text, &links, FontId::proportional(13.0), color, link_color(ui));
    job.wrap.max_width = max_width;
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('…');
    let galley = ui.fonts_mut(|f| f.layout_job(job));
    let pos = pos2(left_center.x, (left_center.y - galley.size().y / 2.0).round());
    ui.painter().galley(pos, galley.clone(), color);
    !links.is_empty() && interact_links(ui, id, &galley, pos, text, &links)
}

/// Completion popup state, kept in egui's memory between frames.
#[derive(Clone, Copy, Default)]
struct Popup {
    selected: usize,
    /// Escape closed the popup for the reference starting here.
    dismissed: Option<usize>,
    /// The popup's rectangle in the last frame.
    rect: Option<Rect>,
}

enum Key2 {
    Up,
    Down,
    Accept,
    Dismiss,
}

const MAX_ITEMS: usize = 12;
const ROW_H: f32 = 24.0;

/// The commit message editor: links underlined (Cmd/Ctrl+click opens them) and completion of
/// work item ids after `#` / `AB#`.
pub fn message_editor(ui: &mut Ui, text: &mut String, work_items: &WorkItems, hint: &str) -> Response {
    let id = ui.make_persistent_id("commit_message");
    let popup_id = id.with("work_items");
    let mut popup: Popup = ui.data(|d| d.get_temp(popup_id)).unwrap_or_default();
    let linker = work_items.linker();
    // Keys for an open popup are taken before the text field sees them.
    let mut key = None;
    if popup.rect.is_some() {
        ui.input_mut(|i| {
            key = if i.consume_key(Modifiers::NONE, Key::ArrowDown) {
                Some(Key2::Down)
            } else if i.consume_key(Modifiers::NONE, Key::ArrowUp) {
                Some(Key2::Up)
            } else if i.consume_key(Modifiers::NONE, Key::Enter) || i.consume_key(Modifiers::NONE, Key::Tab) {
                Some(Key2::Accept)
            } else if i.consume_key(Modifiers::NONE, Key::Escape) {
                Some(Key2::Dismiss)
            } else {
                None
            };
        });
    }
    let link = link_color(ui);
    let font = egui::TextStyle::Body.resolve(ui.style());
    let color = ui.visuals().text_color();
    let layout_linker: Arc<Linker> = linker.clone();
    let mut layouter = |ui: &Ui, buf: &dyn egui::TextBuffer, wrap: f32| {
        let s = buf.as_str();
        let mut job = job(s, &layout_linker.find(s), font.clone(), color, link);
        job.wrap.max_width = wrap;
        ui.fonts_mut(|f| f.layout_job(job))
    };
    let mut edit = egui::TextEdit::multiline(text).id(id).hint_text(hint).desired_rows(4).desired_width(f32::INFINITY).layouter(&mut layouter);
    // Escape closes the popup first (handled here, so egui must not drop the focus on its
    // own); Tab accepts while the popup is open.
    edit = edit.event_filter(egui::EventFilter { tab: popup.rect.is_some(), horizontal_arrows: true, vertical_arrows: true, escape: true });
    let mut out = edit.show(ui);
    let resp = out.response.response.clone();
    if resp.has_focus() && ui.input(|i| i.key_pressed(Key::Escape)) {
        ui.memory_mut(|m| m.surrender_focus(id));
    }

    // Links: hand cursor and URL with Cmd/Ctrl held; Cmd/Ctrl+click opens.
    if let Some(p) = resp.hover_pos() {
        let c = out.galley.cursor_from_pos(p - out.galley_pos);
        let byte = text.char_indices().nth(c.index.0).map_or(text.len(), |(b, _)| b);
        let links = linker.find(text);
        let on = links.iter().find(|l| l.range.contains(&byte));
        if let Some(l) = on {
            let command = ui.input(|i| i.modifiers.command);
            if command {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                if resp.clicked() {
                    open_link(&l.url);
                }
            }
            let key = if cfg!(target_os = "macos") { "⌘" } else { "Ctrl" };
            resp.clone().on_hover_text(format!("{}\n{}", l.url, trf("{key}+Click to open", &[("key", &key)])));
        }
    }

    // Completion.
    let cursor: Option<usize> = out.cursor_range.map(|r| r.primary.index.0);
    let query = cursor.and_then(|c| {
        let byte = text.char_indices().nth(c).map_or(text.len(), |(b, _)| b);
        completion_query(text, byte).map(|q| (q, byte))
    });
    let pointer_on_popup = popup.rect.is_some_and(|r| ui.input(|i| i.pointer.interact_pos().is_some_and(|p| r.contains(p))));
    let items = work_items.items();
    let shown: Vec<&WorkItem> = match &query {
        Some((q, _)) if popup.dismissed != Some(q.start) && (resp.has_focus() || pointer_on_popup) => complete(&linker, &items, q, MAX_ITEMS),
        _ => Vec::new(),
    };
    if query.as_ref().is_none_or(|(q, _)| popup.dismissed != Some(q.start)) {
        popup.dismissed = None;
    }
    let mut accept = None;
    if let (Some((q, _)), false) = (&query, shown.is_empty()) {
        popup.selected = popup.selected.min(shown.len() - 1);
        match key {
            Some(Key2::Down) => popup.selected = (popup.selected + 1) % shown.len(),
            Some(Key2::Up) => popup.selected = (popup.selected + shown.len() - 1) % shown.len(),
            Some(Key2::Accept) => accept = Some(popup.selected),
            Some(Key2::Dismiss) => popup.dismissed = Some(q.start),
            None => {}
        }
    }
    popup.rect = None;
    if let (Some((q, end)), false, None) = (&query, shown.is_empty(), popup.dismissed) {
        let caret = out.galley.pos_from_cursor(CCursor::new(cursor.unwrap_or(0usize))).translate(out.galley_pos.to_vec2());
        let (rect, clicked) = show_popup(ui, popup_id, caret.left_bottom() + vec2(-8.0, 4.0), q, &shown, popup.selected, work_items.online());
        popup.rect = Some(rect);
        if let Some(i) = clicked {
            accept = Some(i);
        }
        if let Some(i) = accept
            && let Some(w) = shown.get(i)
        {
            let insert = format!("{}{}", q.prefix, w.id);
            text.replace_range(q.start..*end, &insert);
            let at = char_index(text, q.start + insert.len());
            out.state.cursor.set_char_range(Some(CCursorRange::one(CCursor::new(at))));
            out.state.store(ui.ctx(), id);
            ui.memory_mut(|m| m.request_focus(id));
            popup.rect = None;
            popup.selected = 0;
            // A completed reference does not reopen the popup.
            popup.dismissed = Some(q.start);
        }
    } else {
        popup.selected = 0;
    }
    ui.data_mut(|d| d.insert_temp(popup_id, popup));
    resp
}

/// The list of ids; returns its rectangle and the clicked row.
fn show_popup(ui: &Ui, id: Id, at: Pos2, q: &CompletionQuery, items: &[&WorkItem], selected: usize, online: bool) -> (Rect, Option<usize>) {
    let p = rc_kit::pal(ui);
    let width = 360.0;
    let mut clicked = None;
    let area = egui::Area::new(id).order(Order::Foreground).fixed_pos(at).show(ui.ctx(), |ui| {
        egui::Frame::NONE
            .fill(p.popup_bg)
            .stroke(Stroke::new(1.0, p.popup_border))
            .corner_radius(8)
            .shadow(egui::Shadow { offset: [0, 4], blur: 16, spread: 0, color: Color32::from_black_alpha(if p.dark { 90 } else { 30 }) })
            .inner_margin(egui::Margin::same(4))
            .show(ui, |ui| {
                ui.set_width(width);
                ui.spacing_mut().item_spacing.y = 0.0;
                for (row, w) in items.iter().enumerate() {
                    let (r, resp) = ui.allocate_exact_size(vec2(width, ROW_H), Sense::click());
                    if row == selected {
                        ui.painter().rect_filled(r, 4.0, p.selection);
                    } else if resp.hovered() {
                        ui.painter().rect_filled(r, 4.0, p.row_hover);
                    }
                    rc_kit::icons::paint(ui.painter(), Rect::from_center_size(pos2(r.left() + 14.0, r.center().y), vec2(16.0, 16.0)), rc_kit::ico::HASHTAG, &p, true);
                    let label = format!("{}{}", q.prefix, w.id);
                    let typed = q.prefix.len() + q.digits.len();
                    let mut job = LayoutJob::default();
                    job.append(&label[..typed.min(label.len())], 0.0, TextFormat { font_id: rc_kit::theme::mono_bold(13.0), color: p.accent, ..Default::default() });
                    job.append(&label[typed.min(label.len())..], 0.0, TextFormat { font_id: FontId::monospace(13.0), color: p.text, ..Default::default() });
                    job.append(&w.title, 10.0, TextFormat { font_id: FontId::proportional(12.5), color: p.muted, ..Default::default() });
                    job.wrap.max_width = r.width() - 36.0;
                    job.wrap.max_rows = 1;
                    job.wrap.break_anywhere = true;
                    job.wrap.overflow_character = Some('…');
                    let g = ui.painter().layout_job(job);
                    ui.painter().galley(pos2(r.left() + 28.0, r.center().y - g.size().y / 2.0), g, p.text);
                    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, row == selected, format!("{label} {}", w.title)));
                    if resp.clicked() || resp.is_pointer_button_down_on() {
                        clicked = Some(row);
                    }
                }
                let source = if online { tr("Open items and recent commits") } else { tr("From recent commits") };
                let (r, _) = ui.allocate_exact_size(vec2(width, 20.0), Sense::hover());
                ui.painter().text(pos2(r.left() + 8.0, r.center().y), Align2::LEFT_CENTER, source, FontId::proportional(11.5), p.muted);
            });
    });
    (area.response.rect, clicked)
}

/// Settings › Version Control: the trackers found in the remotes and the custom patterns
/// (Rider's Issue Navigation).
pub fn settings_page(ui: &mut Ui, draft: &mut rc_core::Settings, app: &crate::app::DotCraftApp) {
    let p = rc_kit::pal(ui);
    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
    ui.label(egui::RichText::new(tr("Issue Navigation")).font(rc_kit::theme::bold(13.0)));
    ui.add_space(2.0);
    crate::ui::kit::note(ui, tr("In commit messages, #123 and AB#123 link to the issue tracker of the repository's remote. Add patterns for other trackers; $1, $2… in the link are replaced by the groups of the match."));
    ui.add_space(6.0);
    let trackers = app.vcs.work_items.trackers();
    if trackers.is_empty() {
        crate::ui::kit::note(ui, tr("No GitHub, GitLab or Azure DevOps remote found."));
    }
    for t in trackers {
        let name = match t.kind {
            rc_vcs::TrackerKind::GitHub => "GitHub",
            rc_vcs::TrackerKind::GitLab => "GitLab",
            rc_vcs::TrackerKind::Azure => "Azure DevOps",
        };
        ui.label(egui::RichText::new(format!("{name}  {}", t.web)).size(12.5).color(p.text));
    }
    ui.add_space(10.0);
    let gap = ui.spacing().item_spacing.x;
    let field_w = ((ui.available_width() - 24.0 - gap * 3.0) / 2.0).max(60.0);
    let mut remove = None;
    for (i, link) in draft.issue_links.iter_mut().enumerate() {
        ui.push_id(("issue_link", i), |ui| {
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut link.pattern).hint_text(tr("Issue ID (regex)")).desired_width(field_w).margin(vec2(6.0, 4.0)).font(egui::TextStyle::Monospace));
                ui.add(egui::TextEdit::singleline(&mut link.url).hint_text(tr("Issue link")).desired_width(field_w).margin(vec2(6.0, 4.0)));
                if rc_kit::theme::icon_button(ui, rc_kit::ico::DELETE, true, tr("Remove")).clicked() {
                    remove = Some(i);
                }
            });
        });
        ui.add_space(4.0);
    }
    if let Some(i) = remove {
        draft.issue_links.remove(i);
    }
    if rc_kit::theme::add_button(ui, rc_kit::ico::ADD, tr("Add Pattern")).clicked() {
        draft.issue_links.push(rc_core::settings::IssueLink { pattern: r"\b[A-Z]+-\d+\b".into(), url: "https://jira.example.com/browse/$0".into() });
    }
    // Live check of the draft.
    let custom: Vec<rc_vcs::LinkPattern> = draft.issue_links.iter().map(|l| rc_vcs::LinkPattern { pattern: l.pattern.clone(), url: l.url.clone() }).collect();
    let (_, errors) = Linker::new(&[], &custom);
    for e in errors {
        crate::ui::kit::message(ui, &trf("Invalid pattern: {error}", &[("error", &e)]), true);
    }
    ui.add_space(6.0);
    crate::ui::kit::note(ui, tr("Type # or AB# in the commit message to complete ids from recent commits, or from the tracker when GH_TOKEN, GITHUB_TOKEN, GITLAB_TOKEN or AZURE_DEVOPS_EXT_PAT is set or the GitHub CLI is signed in."));
}

/// A copy of the Shop fixture with GitHub and Azure DevOps remotes and commits that reference
/// work items (used by the tests and the captures).
#[cfg(test)]
pub fn shop_with_work_items(base: &std::path::Path) -> std::path::PathBuf {
    let root = base.join("Shop");
    let copy = std::process::Command::new("cp").args(["-R", crate::screens::fixtures().join("Shop").to_str().unwrap(), root.to_str().unwrap()]).status().unwrap();
    assert!(copy.success());
    let git = |args: &[&str]| assert!(std::process::Command::new("git").args(args).current_dir(&root).output().unwrap().status.success(), "{args:?}");
    git(&["init", "-q", "-b", "main"]);
    git(&["config", "user.name", "Ada Lovelace"]);
    git(&["config", "user.email", "ada@example.com"]);
    git(&["remote", "add", "origin", "git@github.com:contoso/shop.git"]);
    git(&["remote", "add", "boards", "https://contoso@dev.azure.com/contoso/Shop/_git/Shop"]);
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "Initial shop"]);
    for (i, (subject, body)) in [
        ("Add coupons (#12)", "Coupons take a percentage off the order.\n\nCloses #12, tracked in AB#1234."),
        ("Fix rounding of discounts #15", ""),
        ("SHOP-7 Translate the checkout", "Spanish and French."),
        ("Export orders to CSV AB#1240", ""),
        ("Merge pull request #18 from contoso/gift-cards", "Gift cards, see contoso/payments#4"),
    ]
    .into_iter()
    .enumerate()
    {
        std::fs::write(root.join(format!("src/Shop.Core/Change{i}.cs")), "namespace Shop.Core;\n").unwrap();
        git(&["add", "."]);
        let mut args = vec!["commit", "-q", "-m", subject];
        if !body.is_empty() {
            args.extend(["-m", body]);
        }
        git(&args);
    }
    std::fs::write(root.join("src/Shop.Core/Coupon.cs"), "namespace Shop.Core;\n\npublic record Coupon(string Code, decimal Percent);\n").unwrap();
    root
}

#[cfg(test)]
pub(crate) mod tests {
    use crate::screens::{HOME_LOCK, harness, test_app};
    use crate::work_items::OPENED;
    use egui::accesskit::Role;
    use egui_kittest::kittest::Queryable;

    type H = egui_kittest::Harness<'static, crate::app::DotCraftApp>;

    pub fn type_text(h: &mut H, s: &str) {
        for ch in s.chars() {
            h.event(egui::Event::Text(ch.to_string()));
            h.run_steps(1);
        }
    }

    /// The Shop repository open with the Git and Commit windows, its log and remotes loaded.
    pub fn open_shop(base: &std::path::Path, dark: bool) -> H {
        let root = super::shop_with_work_items(base);
        let mut app = test_app(&base.join("home"), dark);
        app.settings.roslyn_path = Some("/nonexistent".into());
        app.settings.issue_links = vec![rc_core::settings::IssueLink { pattern: r"\bSHOP-(\d+)\b".into(), url: "https://jira.example.com/browse/SHOP-$1".into() }];
        app.open_workspace(&root.join("Shop.sln"));
        app.dock.show(crate::toolwindow::ToolWindow::Git);
        app.dock.show(crate::toolwindow::ToolWindow::Commit);
        let mut h = harness(app, egui::vec2(1440.0, 900.0));
        let start = std::time::Instant::now();
        while h.state().vcs.log.len() < 6 || h.state().vcs.work_items.trackers().len() < 2 || h.state().vcs.changes().is_empty() {
            assert!(start.elapsed() < std::time::Duration::from_secs(60), "log not loaded");
            h.run_steps(1);
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        h.run_steps(3);
        h
    }

    #[test]
    fn log_links_open_the_tracker_and_the_message_completes_ids() {
        let _home = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let mut h = open_shop(&dir.path().canonicalize().unwrap(), true);
        OPENED.with(|o| o.borrow_mut().clear());
        // The log's subjects: # is the GitHub issue (origin), AB# the Azure Boards work item.
        h.get_by_role_and_label(Role::Link, "#15").click();
        h.run_steps(2);
        h.get_by_role_and_label(Role::Link, "AB#1240").click();
        h.run_steps(2);
        h.get_by_role_and_label(Role::Link, "SHOP-7").click();
        h.run_steps(2);
        assert_eq!(
            OPENED.with(|o| o.borrow().clone()),
            vec!["https://github.com/contoso/shop/issues/15", "https://dev.azure.com/contoso/Shop/_workitems/edit/1240", "https://jira.example.com/browse/SHOP-7"]
        );
        // Clicking a link does not select the commit; the details show the body's links.
        assert!(h.state().vcs.selected_commit.is_none());
        let coupons = h.state().vcs.log.iter().find(|c| c.subject.starts_with("Add coupons")).unwrap().hash.clone();
        h.state_mut().vcs.select_commit(&coupons);
        h.run_steps(3);
        assert_eq!(h.query_all_by_role_and_label(Role::Link, "#12").count(), 3, "subject in the log and the details, and the body");
        h.get_by_role_and_label(Role::Link, "AB#1234").click();
        h.run_steps(2);
        assert_eq!(OPENED.with(|o| o.borrow().last().cloned()).as_deref(), Some("https://dev.azure.com/contoso/Shop/_workitems/edit/1234"));

        // Completion in the commit message (offline: ids of recent commits).
        h.get_by_role(Role::MultilineTextInput).focus();
        h.run_steps(2);
        type_text(&mut h, "Fix totals #1");
        h.run_steps(2);
        for row in ["#18 Merge pull request #18 from contoso/gift-cards", "#15 Fix rounding of discounts #15", "#12 Add coupons (#12)"] {
            h.get_by_label(row);
        }
        h.key_press(egui::Key::ArrowDown);
        h.run_steps(1);
        h.key_press(egui::Key::Enter);
        h.run_steps(2);
        assert_eq!(h.state().vcs.message, "Fix totals #15");
        assert!(h.query_by_label("#12 Add coupons (#12)").is_none(), "the popup closes");
        // AB# lists Azure Boards ids; Escape closes the popup and keeps the text.
        type_text(&mut h, ", AB#");
        h.get_by_label("AB#1240 Export orders to CSV AB#1240");
        h.get_by_label("AB#1234 Add coupons (#12)");
        h.key_press(egui::Key::Escape);
        h.run_steps(2);
        assert!(h.get_by_role(Role::MultilineTextInput).is_focused(), "Escape only closes the popup");
        assert!(h.query_by_label("AB#1240 Export orders to CSV AB#1240").is_none());
        type_text(&mut h, "12");
        assert!(h.query_by_label("AB#1240 Export orders to CSV AB#1240").is_none(), "dismissed until the reference changes");
        assert_eq!(h.state().vcs.message, "Fix totals #15, AB#12");
        // A click on a row completes too.
        type_text(&mut h, " #");
        h.get_by_label("#18 Merge pull request #18 from contoso/gift-cards").click();
        h.run_steps(3);
        assert_eq!(h.state().vcs.message, "Fix totals #15, AB#12 #18");
        // Without a popup, Escape leaves the field.
        h.key_press(egui::Key::Escape);
        h.run_steps(2);
        assert!(!h.get_by_role(Role::MultilineTextInput).is_focused());
    }

    #[test]
    fn settings_page_adds_custom_patterns() {
        let _home = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let mut app = test_app(dir.path(), true);
        app.dialog = Some(crate::ui::dialogs::Dialog::Settings { draft: Box::new(app.settings.clone()), page: crate::ui::dialogs::SettingsPage::VersionControl });
        let mut h = harness(app, egui::vec2(1200.0, 800.0));
        h.get_by_label(rc_kit::i18n::tr("Add Pattern")).click();
        h.run_steps(2);
        h.get_by_label(rc_kit::i18n::tr("OK")).click();
        h.run_steps(2);
        assert_eq!(h.state().settings.issue_links.len(), 1);
        assert_eq!(h.state().settings.issue_links[0].url, "https://jira.example.com/browse/$0");
    }
}
