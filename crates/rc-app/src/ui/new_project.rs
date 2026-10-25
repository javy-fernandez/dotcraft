//! New Solution / Add New Project (Rider): templates on the left, names, location, framework
//! and Git on the right; `rc_dotnet::new_project` runs `dotnet new` in the background.

use crate::app::DotCraftApp;
use egui::{Align2, Context, FontId, Id, Rect, Sense, pos2, vec2};
use rc_core::{Inbox, Runtime, Waker};
use rc_dotnet::new_project::{NewProject, TEMPLATES, Target, Template};
use rc_kit::i18n::{tr, trf};
use rc_kit::{Ico, ico, icons, theme};
use std::path::PathBuf;

enum Event {
    Progress(String),
    Done(Result<PathBuf, String>),
}

pub struct NewProjectDialog {
    /// Add to this solution instead of creating one.
    pub add_to: Option<PathBuf>,
    template: usize,
    solution_name: String,
    project_name: String,
    /// The project name follows the solution name until it is edited.
    project_edited: bool,
    location: String,
    frameworks: Vec<String>,
    framework: usize,
    git: bool,
    use_controllers: bool,
    busy: Option<String>,
    error: Option<String>,
    inbox: Inbox<Event>,
    focus: bool,
}

fn template_icon(t: &Template) -> Ico {
    if t.test {
        ico::RUN_ALL
    } else if t.web {
        ico::CONFIG_WEB
    } else if t.short_name == "classlib" {
        ico::LIBRARY
    } else {
        ico::CONFIG_APP
    }
}

/// The folder new solutions go to: the parent of the last opened workspace, or ~/Projects.
fn default_location(app: &DotCraftApp) -> PathBuf {
    if let Some(p) = app.settings.new_project_location.clone().filter(|p| p.is_dir()) {
        return p;
    }
    if let Some(parent) = app.settings.recent_workspaces.first().and_then(|w| if w.is_dir() { w.parent() } else { w.parent().and_then(|p| p.parent()) }).filter(|p| p.is_dir()) {
        return parent.to_path_buf();
    }
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    ["Projects", "RiderProjects", "source/repos"].iter().map(|d| home.join(d)).find(|p| p.is_dir()).unwrap_or(home)
}

impl NewProjectDialog {
    pub fn new(app: &DotCraftApp, add_to: Option<PathBuf>, waker: Waker) -> Self {
        let frameworks = rc_dotnet::Dotnet::locate(Some(&crate::app::dotnet_path(&app.settings))).map(|d| rc_dotnet::new_project::frameworks(&d)).unwrap_or_default();
        let location = match &add_to {
            Some(s) => s.parent().unwrap_or(s).to_path_buf(),
            None => default_location(app),
        };
        // A free default name: Solution1, Solution2…
        let base = if add_to.is_some() { "Project" } else { "Solution" };
        let name = (1..).map(|i| format!("{base}{i}")).find(|n| !location.join(n).exists()).unwrap_or_default();
        NewProjectDialog {
            add_to,
            template: 0,
            project_name: name.clone(),
            solution_name: name,
            project_edited: false,
            location: location.display().to_string(),
            frameworks,
            framework: 0,
            git: true,
            use_controllers: true,
            busy: None,
            error: None,
            inbox: Inbox::new(waker),
            focus: true,
        }
    }

    fn request(&self) -> NewProject {
        let target = match &self.add_to {
            Some(s) => Target::AddTo { solution: s.clone() },
            None => Target::NewSolution { location: PathBuf::from(self.location.trim()), solution_name: self.solution_name.trim().to_string(), git: self.git },
        };
        NewProject { template: TEMPLATES[self.template].short_name, project_name: self.project_name.trim().to_string(), framework: self.frameworks.get(self.framework).cloned(), use_controllers: self.use_controllers, target }
    }
}

/// Shows the dialog; returns false when it closed.
pub fn show(app: &mut DotCraftApp, ctx: &Context, d: &mut NewProjectDialog) -> bool {
    let mut open = true;
    for ev in d.inbox.drain() {
        match ev {
            Event::Progress(line) => d.busy = Some(line),
            Event::Done(Ok(solution)) => {
                d.busy = None;
                let project = d.request().project_file();
                if d.add_to.is_none() {
                    app.settings.new_project_location = Some(PathBuf::from(d.location.trim()));
                    app.open_workspace(&solution);
                } else {
                    app.reload_workspace();
                }
                // Open the main file of the new project.
                let main = ["Program.cs", "Class1.cs", "UnitTest1.cs", "Test1.cs", "Worker.cs"].iter().map(|f| project.parent().unwrap().join(f)).find(|p| p.exists());
                if let Some(f) = main {
                    app.open_file(&f);
                }
                app.notify(trf("{name} created", &[("name", &project.file_stem().unwrap_or_default().to_string_lossy())]), false);
                return false;
            }
            Event::Done(Err(e)) => {
                d.busy = None;
                d.error = Some(e);
            }
        }
    }
    if d.busy.is_some() {
        ctx.request_repaint_after(std::time::Duration::from_millis(150));
    }
    let p = rc_kit::theme::palette(ctx.global_style().visuals.dark_mode);
    let req = d.request();
    let problem = req.problem().map(tr);
    let mut create = false;
    let title = if d.add_to.is_some() { tr("Add New Project") } else { tr("New Solution") };
    let resp = egui::Modal::new(Id::new("new_project")).show(ctx, |ui| {
        ui.set_width(760.0);
        ui.set_height(440.0);
        ui.label(egui::RichText::new(title).font(theme::bold(15.0)));
        ui.add_space(10.0);
        let avail = ui.available_rect_before_wrap();
        let left = Rect::from_min_size(avail.min, vec2(250.0, avail.height() - 44.0));
        let right = Rect::from_min_max(pos2(left.right() + 18.0, avail.top()), pos2(avail.right(), avail.bottom() - 44.0));
        // Templates.
        ui.painter().rect_filled(left, 8.0, p.main_bg);
        let mut lu = ui.new_child(egui::UiBuilder::new().max_rect(left.shrink(4.0)));
        lu.spacing_mut().item_spacing.y = 2.0;
        for (i, t) in TEMPLATES.iter().enumerate() {
            let (r, resp) = lu.allocate_exact_size(vec2(lu.available_width(), 30.0), Sense::click());
            if i == d.template {
                lu.painter().rect_filled(r, 6.0, p.selection);
            } else if resp.hovered() {
                lu.painter().rect_filled(r, 6.0, p.row_hover);
            }
            icons::paint(lu.painter(), Rect::from_center_size(pos2(r.left() + 16.0, r.center().y), vec2(16.0, 16.0)), template_icon(t), &p, true);
            lu.painter().text(pos2(r.left() + 32.0, r.center().y), Align2::LEFT_CENTER, tr(t.name), FontId::proportional(13.0), p.text);
            if resp.clicked() {
                d.template = i;
            }
            resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, i == d.template, tr(t.name)));
        }
        // Fields.
        let mut ru = ui.new_child(egui::UiBuilder::new().max_rect(right));
        let t = &TEMPLATES[d.template];
        ru.label(egui::RichText::new(tr(t.name)).font(theme::bold(14.0)));
        ru.label(egui::RichText::new(tr(t.description)).size(12.5).color(p.muted));
        ru.add_space(12.0);
        egui::Grid::new("new_project_grid").num_columns(2).spacing([12.0, 10.0]).show(&mut ru, |ui| {
            let w = 330.0;
            if d.add_to.is_none() {
                ui.label(tr("Solution name"));
                let te = ui.add(egui::TextEdit::singleline(&mut d.solution_name).desired_width(w));
                if std::mem::take(&mut d.focus) {
                    te.request_focus();
                }
                if te.changed() && !d.project_edited {
                    d.project_name = d.solution_name.clone();
                }
                ui.end_row();
            }
            ui.label(tr("Project name"));
            let te = ui.add(egui::TextEdit::singleline(&mut d.project_name).desired_width(w));
            if d.add_to.is_some() && std::mem::take(&mut d.focus) {
                te.request_focus();
            }
            if te.changed() {
                d.project_edited = true;
            }
            ui.end_row();
            if d.add_to.is_none() {
                ui.label(tr("Location"));
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut d.location).desired_width(w - 34.0));
                    if theme::icon_button(ui, ico::OPEN, true, tr("Choose a folder")).clicked()
                        && let Some(dir) = rfd::FileDialog::new().set_directory(d.location.trim()).pick_folder()
                    {
                        d.location = dir.display().to_string();
                    }
                });
                ui.end_row();
            }
            ui.label(tr("Framework"));
            let current = d.frameworks.get(d.framework).cloned().unwrap_or_else(|| tr("Template default").to_string());
            egui::ComboBox::from_id_salt("new_project_fw").selected_text(current).width(160.0).show_ui(ui, |ui| {
                for (i, f) in d.frameworks.iter().enumerate() {
                    ui.selectable_value(&mut d.framework, i, f);
                }
            });
            ui.end_row();
            if TEMPLATES[d.template].short_name == "webapi" {
                ui.label("");
                ui.checkbox(&mut d.use_controllers, tr("Use controllers (instead of minimal APIs)"));
                ui.end_row();
            }
            if d.add_to.is_none() {
                ui.label("");
                ui.checkbox(&mut d.git, tr("Create Git repository"));
                ui.end_row();
            }
        });
        ru.add_space(14.0);
        let dir = req.project_dir();
        ru.label(egui::RichText::new(trf("The project will be created in {path}", &[("path", &dir.display())])).size(12.0).color(p.muted));
        if let Some(e) = d.error.as_deref().or(problem) {
            ru.add_space(6.0);
            ru.label(egui::RichText::new(e).size(12.5).color(p.error));
        }
        if let Some(line) = &d.busy {
            ru.add_space(6.0);
            ru.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(14.0));
                ui.label(egui::RichText::new(line).size(12.0).color(p.muted).monospace());
            });
        }
        // Buttons.
        let bar = Rect::from_min_max(pos2(avail.left(), avail.bottom() - 32.0), avail.max);
        let mut bu = ui.new_child(egui::UiBuilder::new().max_rect(bar).layout(egui::Layout::right_to_left(egui::Align::Center)));
        if bu.add_enabled_ui(problem.is_none() && d.busy.is_none(), |ui| theme::primary_button(ui, tr("Create"))).inner.clicked() {
            create = true;
        }
        if theme::secondary_button(&mut bu, tr("Cancel")).clicked() {
            open = false;
        }
        ui.advance_cursor_after_rect(avail);
        if ui.input(|i| i.key_pressed(egui::Key::Enter)) && problem.is_none() && d.busy.is_none() {
            create = true;
        }
    });
    if resp.should_close() && d.busy.is_none() {
        open = false;
    }
    if create {
        let Some(dotnet) = rc_dotnet::Dotnet::locate(Some(&crate::app::dotnet_path(&app.settings))) else {
            d.error = Some(tr("The .NET SDK was not found. Install it or set its path in Settings › Tools.").into());
            return open;
        };
        d.error = None;
        d.busy = Some(tr("Creating…").into());
        let out = d.inbox.sender();
        Runtime::spawn(async move {
            let progress = out.clone();
            let r = req.create(&dotnet, &mut move |line| {
                progress.send(Event::Progress(line));
            }).await;
            out.send(Event::Done(r.map_err(|e| format!("{e:#}"))));
        });
    }
    open
}
