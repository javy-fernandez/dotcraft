//! The XAML preview pane (Rider): the view Avalonia rendered, centred on a canvas, with zoom
//! controls and the build state in a bar at the bottom and the previewer's error above it.
//! Frameworks without a previewer (MAUI, WPF, WinUI) get a note instead.

use crate::app::DotCraftApp;
use crate::xaml_preview::{Env, Stage, XamlKind, Zoom, fit_scale, zoom_in, zoom_out};
use egui::{Align2, FontId, Rect, Sense, Ui, pos2, vec2};
use rc_kit::i18n::{tr, trf};
use rc_kit::{ico, icons, theme};

const BAR: f32 = 32.0;

fn env(app: &mut DotCraftApp, ctx: &egui::Context) -> Env {
    let dotnet = app.runner.dotnet().ok().or_else(|| rc_dotnet::Dotnet::locate(app.settings.dotnet_path.as_deref()));
    let executables = app
        .runner
        .configs
        .iter()
        .filter_map(|c| match &c.kind {
            crate::runner::ConfigKind::App { project, .. } => Some(project.clone()),
            _ => None,
        })
        .collect();
    Env { dotnet, executables, building: app.runner.building(), scale: ctx.pixels_per_point() }
}

/// The preview of document `idx` in `rect`. Returns whether it was clicked.
pub fn preview(app: &mut DotCraftApp, ui: &mut Ui, idx: usize, rect: Rect) -> bool {
    let p = rc_kit::pal(ui);
    let ctx = ui.ctx().clone();
    let id = app.docs.list[idx].editor.id;
    let env = env(app, &ctx);
    app.xaml_previews.update(&ctx, &app.docs.list[idx], &env);
    let canvas = theme::blend(p.editor_bg, p.island_border.gamma_multiply(0.35));
    ui.painter().rect_filled(rect, 0.0, canvas);
    if app.docs.list[idx].md_view == Some(crate::docs::MdView::Split) {
        ui.painter().vline(rect.left() - 0.5, rect.y_range(), egui::Stroke::new(1.0, p.island_border));
    }
    let clicked = ui.input(|i| i.pointer.primary_pressed() && i.pointer.interact_pos().is_some_and(|p| rect.contains(p)));
    let Some(preview) = app.xaml_previews.map.get(&id) else { return clicked };
    // The bar (zoom, outdated build) once there is a view to zoom.
    let has_bar = preview.texture.is_some() && matches!(preview.stage, Stage::Running(_) | Stage::Idle(_));
    let mut content = rect;
    if has_bar {
        content.max.y -= BAR;
        bar(app, ui, idx, Rect::from_min_max(pos2(rect.left(), content.bottom()), rect.max), &env);
    }
    // The previewer's error, above the bar.
    let error = app.xaml_previews.map.get(&id).and_then(|pv| pv.error.clone().filter(|_| matches!(pv.stage, Stage::Running(_))));
    if let Some(error) = &error {
        content.max.y -= error_panel(app, ui, idx, content, error);
    }
    let Some(preview) = app.xaml_previews.map.get(&id) else { return clicked };
    // Below the Editor / Preview switch.
    let area = Rect::from_min_max(pos2(content.left() + 20.0, content.top() + 48.0), pos2(content.right() - 20.0, content.bottom() - 20.0));
    if let (Some(texture), Stage::Running(_) | Stage::Idle(_)) = (&preview.texture, &preview.stage) {
        let size = preview.frame_size;
        let scale = match preview.zoom {
            Zoom::Fit => fit_scale(size, area.size()),
            Zoom::Scale(s) => s,
        };
        let shown = size * scale;
        let (texture, dim) = (texture.id(), preview.updating && error.is_none());
        if let Some(pv) = app.xaml_previews.map.get_mut(&id) {
            pv.shown_scale = scale;
        }
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(area).id_salt(("xaml_area", id)));
        child.set_clip_rect(area.intersect(ui.clip_rect()));
        egui::ScrollArea::both().id_salt(("xaml_scroll", id)).auto_shrink(false).show(&mut child, |ui| {
            let avail = ui.available_size();
            let (r, _) = ui.allocate_exact_size(vec2(shown.x.max(avail.x), shown.y.max(avail.y)), Sense::hover());
            // Centred while it fits, pixel-aligned so 100% is sharp.
            let ppp = ui.ctx().pixels_per_point();
            let min = r.min + ((r.size() - shown) / 2.0).max(vec2(0.0, 0.0));
            let min = pos2((min.x * ppp).round() / ppp, (min.y * ppp).round() / ppp);
            let img = Rect::from_min_size(min, shown);
            ui.painter().rect_filled(img.expand(3.0).translate(vec2(0.0, 1.0)), 4.0, egui::Color32::from_black_alpha(if p.dark { 60 } else { 18 }));
            let tint = if dim { egui::Color32::from_white_alpha(235) } else { egui::Color32::WHITE };
            ui.painter().image(texture, img, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), tint);
            ui.painter().rect_stroke(img.expand(0.5), 0.0, egui::Stroke::new(1.0, p.island_border), egui::StrokeKind::Middle);
        });
        return clicked;
    }
    // No picture yet: what is happening, or why there is none.
    let building = env.building;
    let (icon, title, hint, button): (rc_kit::Ico, String, String, Option<String>) = match &preview.stage {
        Stage::Unsupported(kind) => {
            let (title, hint) = note(*kind);
            (ico::DIALOG_INFO, title.into(), hint.into(), None)
        }
        Stage::Resolving(_) => (ico::PREVIEW, tr("Loading the project…").into(), String::new(), None),
        Stage::Unavailable(reason) => (ico::PREVIEW, tr("No preview").into(), reason.clone(), None),
        Stage::NeedsBuild(project) if building => (ico::BUILD, tr("Building…").into(), trf("The preview starts when {app} is built.", &[("app", &file_name(&project.app_project))]), None),
        Stage::NeedsBuild(project) => (
            ico::BUILD,
            tr("Build the project to see the preview").into(),
            trf("The previewer runs the XAML inside the built {app}.", &[("app", &file_name(&project.app_project))]),
            Some(tr("Build").into()),
        ),
        Stage::Running(_) if error.is_some() => (ico::ERROR, tr("The XAML could not be shown").into(), tr("Fix the error to see the preview.").into(), None),
        Stage::Running(_) | Stage::Idle(_) => (ico::PREVIEW, tr("Starting the previewer…").into(), tr("Avalonia renders the XAML inside the built application.").into(), None),
        Stage::Stopped { message, log, .. } => {
            let tail: Vec<&str> = log.lines().filter(|l| !l.trim().is_empty()).collect();
            let tail = tail[tail.len().saturating_sub(6)..].join("\n");
            (ico::ERROR, message.clone(), tail, Some(tr("Restart").into()))
        }
    };
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(content).id_salt(("xaml_state", id)));
    child.set_clip_rect(content.intersect(ui.clip_rect()));
    if crate::ui::toolwindows::empty_state_with(&mut child, icon, &title, &hint, button.as_deref()) {
        match &preview.stage {
            Stage::NeedsBuild(project) => {
                let target = project.app_project.clone();
                let outs = app.runner.build(&target, rc_dotnet::BuildAction::Build);
                crate::ui::run_windows::handle_outputs(app, outs);
            }
            Stage::Stopped { .. } => app.xaml_previews.restart(id, &env, &ctx),
            _ => {}
        }
    }
    clicked
}

/// Title and explanation for XAML that has no previewer.
pub fn note(kind: XamlKind) -> (&'static str, &'static str) {
    match kind {
        XamlKind::Maui => (
            tr("No preview for .NET MAUI"),
            tr(".NET MAUI has no design-time XAML previewer (Visual Studio removed its own). Run the app to see this page: XAML Hot Reload for MAUI is available in Visual Studio and in the .NET MAUI extension for VS Code."),
        ),
        XamlKind::Wpf => (tr("No preview for WPF"), tr("The WPF designer is part of Visual Studio on Windows and cannot run in another IDE. Run the app to see this window.")),
        XamlKind::WinUi => (tr("No preview for WinUI"), tr("WinUI has no design-time XAML previewer. Run the app to see this page.")),
        XamlKind::Avalonia | XamlKind::Unknown => (tr("No preview"), tr("The preview shows Avalonia XAML: .axaml files, or .xaml files whose root element uses the Avalonia namespace.")),
    }
}

fn file_name(path: &std::path::Path) -> String {
    path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned())
}

/// The bottom bar: the state on the left (with Build when the code changed), zoom on the right.
fn bar(app: &mut DotCraftApp, ui: &mut Ui, idx: usize, rect: Rect, env: &Env) {
    let p = rc_kit::pal(ui);
    let id = app.docs.list[idx].editor.id;
    ui.painter().rect_filled(rect, 0.0, p.island_bg);
    ui.painter().hline(rect.x_range(), rect.top() + 0.5, egui::Stroke::new(1.0, p.island_border));
    let Some(preview) = app.xaml_previews.map.get(&id) else { return };
    let (size, zoom, shown, stale, updating) = (preview.frame_size, preview.zoom, preview.shown_scale, preview.stale, preview.updating);
    let running = matches!(&preview.stage, Stage::Running(r) if r.connected);
    let has_frame = preview.texture.is_some();
    let app_project = preview.project().map(|p| p.app_project.clone());
    let mut row = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(vec2(8.0, 3.0))).id_salt(("xaml_bar", id)).layout(egui::Layout::left_to_right(egui::Align::Center)));
    row.set_clip_rect(rect.intersect(ui.clip_rect()));
    row.spacing_mut().item_spacing.x = 2.0;
    let mut build = false;
    if stale && running {
        icons::image(&mut row, ico::WARNING, 16.0);
        row.add_space(4.0);
        crate::ui::kit::note(&mut row, tr("Code changed since the last build"));
        build = crate::ui::kit::button(&mut row, Some(ico::BUILD), tr("Build"), !env.building).clicked();
    } else if updating {
        crate::ui::kit::note(&mut row, tr("Updating…"));
    } else if has_frame {
        crate::ui::kit::note(&mut row, &format!("{} × {}", size.x.round(), size.y.round()));
    }
    let mut new_zoom = None;
    row.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        if theme::icon_toggle(ui, ico::ZOOM_FIT, zoom == Zoom::Fit, tr("Zoom to Fit")).clicked() {
            new_zoom = Some(Zoom::Fit);
        }
        if theme::icon_button(ui, ico::ZOOM_ACTUAL, has_frame, tr("Actual Size")).clicked() {
            new_zoom = Some(Zoom::Scale(1.0));
        }
        if theme::icon_button(ui, ico::ZOOM_IN, has_frame, tr("Zoom In")).clicked() {
            new_zoom = Some(Zoom::Scale(zoom_in(shown)));
        }
        let label = if has_frame { format!("{:.0}%", shown * 100.0) } else { String::new() };
        ui.add_sized(vec2(44.0, 24.0), egui::Label::new(egui::RichText::new(label).size(12.0).color(p.muted)));
        if theme::icon_button(ui, ico::ZOOM_OUT, has_frame, tr("Zoom Out")).clicked() {
            new_zoom = Some(Zoom::Scale(zoom_out(shown)));
        }
    });
    if let Some(z) = new_zoom
        && let Some(pv) = app.xaml_previews.map.get_mut(&id)
    {
        pv.zoom = z;
    }
    if build && let Some(target) = app_project {
        let outs = app.runner.build(&target, rc_dotnet::BuildAction::Build);
        crate::ui::run_windows::handle_outputs(app, outs);
    }
}

/// The previewer's error at the bottom of `content`; returns its height.
fn error_panel(app: &mut DotCraftApp, ui: &mut Ui, idx: usize, content: Rect, error: &rc_dotnet::avalonia_preview::XamlError) -> f32 {
    let p = rc_kit::pal(ui);
    let width = content.width() - 48.0;
    let galley = ui.painter().layout(error.message.clone(), FontId::proportional(12.5), p.text, width.max(80.0));
    let height = (galley.size().y + 44.0).min(content.height() * 0.45).max(60.0);
    let rect = Rect::from_min_max(pos2(content.left(), content.bottom() - height), content.max);
    ui.painter().rect_filled(rect, 0.0, theme::blend(p.island_bg, p.error.gamma_multiply(0.12)));
    ui.painter().hline(rect.x_range(), rect.top() + 0.5, egui::Stroke::new(1.0, theme::blend(p.island_border, p.error.gamma_multiply(0.4))));
    icons::paint(ui.painter(), Rect::from_min_size(pos2(rect.left() + 12.0, rect.top() + 10.0), vec2(16.0, 16.0)), ico::ERROR, &p, true);
    ui.painter().text(pos2(rect.left() + 36.0, rect.top() + 18.0), Align2::LEFT_CENTER, tr("XAML error"), theme::bold(12.5), p.text);
    if let Some(line) = error.line {
        let at = match error.column {
            Some(column) => trf("Line {line}, column {column}", &[("line", &line), ("column", &column)]),
            None => trf("Line {line}", &[("line", &line)]),
        };
        let r = Rect::from_min_max(pos2(rect.right() - 220.0, rect.top() + 5.0), pos2(rect.right() - 8.0, rect.top() + 31.0));
        let mut c = ui.new_child(egui::UiBuilder::new().max_rect(r).layout(egui::Layout::right_to_left(egui::Align::Center)));
        if crate::ui::kit::button(&mut c, None, &at, true).on_hover_text(tr("Go to the error")).clicked() {
            let doc = &mut app.docs.list[idx];
            let pos = doc.editor.buffer.pos_at(line.saturating_sub(1) as usize, error.column.unwrap_or(1).saturating_sub(1) as usize);
            doc.editor.set_selection(rc_text::Selection::point(pos));
            app.focus_editor = true;
        }
    }
    let body = Rect::from_min_max(pos2(rect.left() + 36.0, rect.top() + 34.0), pos2(rect.right() - 12.0, rect.bottom() - 6.0));
    let mut c = ui.new_child(egui::UiBuilder::new().max_rect(body).id_salt(("xaml_error", app.docs.list[idx].editor.id)));
    c.set_clip_rect(body.intersect(ui.clip_rect()));
    egui::ScrollArea::vertical().auto_shrink(false).show(&mut c, |ui| {
        ui.add(egui::Label::new(egui::RichText::new(&error.message).size(12.5).color(p.text)).wrap().selectable(true));
    });
    height
}
