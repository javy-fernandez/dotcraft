//! Create and edit folder publish profiles, then stream `dotnet publish` in the Build window.

use std::path::PathBuf;

use egui::{Context, TextEdit};
use rc_core::{Inbox, Runtime, Waker};
use rc_dotnet::{BuildRequest, publish::PublishProfile};
use rc_kit::{i18n::tr, ico, theme};

use super::{dialogs, kit};

pub enum PublishOutcome {
    Keep,
    Close,
    Start(BuildRequest),
}

/// The configurations offered in the drop-down; a profile's own one is added when it differs.
const CONFIGURATIONS: [&str; 2] = ["Debug", "Release"];

pub struct PublishDialog {
    projects: Vec<PathBuf>,
    project_index: usize,
    names: Vec<String>,
    selected_name: Option<String>,
    draft: PublishProfile,
    loaded: Inbox<(u64, Result<(Vec<String>, Option<PublishProfile>), String>)>,
    saved: Inbox<Result<(PublishProfile, Option<BuildRequest>), String>>,
    generation: u64,
    loading: bool,
    saving: bool,
    error: Option<String>,
}

impl PublishDialog {
    pub fn new(projects: Vec<PathBuf>, selected: Option<&std::path::Path>, waker: Waker) -> Self {
        let project_index = selected
            .and_then(|path| projects.iter().position(|project| project == path))
            .unwrap_or(0);
        let mut state = Self {
            projects,
            project_index,
            names: Vec::new(),
            selected_name: None,
            draft: PublishProfile::default(),
            loaded: Inbox::new(waker.clone()),
            saved: Inbox::new(waker),
            generation: 0,
            loading: false,
            saving: false,
            error: None,
        };
        state.load(None);
        state
    }

    /// Shows `message` under the fields (a publish that could not start).
    pub fn fail(&mut self, message: impl Into<String>) {
        self.error = Some(message.into());
    }

    fn load(&mut self, name: Option<String>) {
        let Some(project) = self.projects.get(self.project_index).cloned() else {
            return;
        };
        self.generation += 1;
        let generation = self.generation;
        let out = self.loaded.sender();
        self.loading = true;
        self.error = None;
        Runtime::spawn(async move {
            let result = (|| {
                let names = PublishProfile::list(&project)?;
                let chosen = name.or_else(|| names.first().cloned());
                let profile = chosen
                    .as_deref()
                    .map(|name| PublishProfile::load(&project, name))
                    .transpose()?;
                Ok::<_, anyhow::Error>((names, profile))
            })()
            .map_err(|error| error.to_string());
            out.send((generation, result));
        });
    }

    fn create(&mut self) {
        self.selected_name = None;
        self.draft = PublishProfile::default();
        self.error = None;
    }

    fn save(&mut self, publish: bool) {
        let Some(project) = self.projects.get(self.project_index).cloned() else {
            return;
        };
        self.draft.name = self.draft.name.trim().to_string();
        let renamed = !self.selected_name.as_deref().is_some_and(|name| name.eq_ignore_ascii_case(&self.draft.name));
        if renamed && PublishProfile::find(&self.names, &self.draft.name).is_some() {
            self.error = Some(tr("A publish profile with this name already exists").into());
            return;
        }
        let draft = self.draft.clone();
        let out = self.saved.sender();
        self.saving = true;
        self.error = None;
        Runtime::spawn(async move {
            let result = draft
                .save(&project)
                .and_then(|_| {
                    let request = publish.then(|| draft.request(&project)).transpose()?;
                    Ok((draft, request))
                })
                .map_err(|error| error.to_string());
            out.send(result);
        });
    }

    /// Applies finished loads and saves; returns the publish to start, if one was asked for.
    fn receive(&mut self) -> Option<BuildRequest> {
        for (generation, result) in self.loaded.drain() {
            if generation != self.generation {
                continue;
            }
            self.loading = false;
            match result {
                Ok((names, profile)) => {
                    self.names = names;
                    if let Some(profile) = profile {
                        self.selected_name = Some(profile.name.clone());
                        self.draft = profile;
                    } else {
                        self.create();
                    }
                }
                Err(error) => self.error = Some(error),
            }
        }
        let mut start = None;
        for result in self.saved.drain() {
            self.saving = false;
            match result {
                Ok((profile, request)) => {
                    if PublishProfile::find(&self.names, &profile.name).is_none() {
                        self.names.push(profile.name.clone());
                        self.names.sort_unstable_by_key(|name| name.to_ascii_lowercase());
                    }
                    self.selected_name = Some(profile.name.clone());
                    self.draft = profile;
                    start = request.or(start);
                }
                Err(error) => self.error = Some(error),
            }
        }
        start
    }

    pub fn show(&mut self, ctx: &Context) -> PublishOutcome {
        if let Some(request) = self.receive() {
            return PublishOutcome::Start(request);
        }
        let mut next_project = None;
        let mut next_profile = None;
        let mut create = false;
        let mut save = false;
        let mut publish = false;
        let mut cancel = false;
        let width = (ctx.content_rect().width() - 48.0).clamp(340.0, 600.0);
        let field_width = (width - 150.0).max(160.0);
        let ready = !self.loading && !self.saving;
        let ((), escape) = dialogs::modal(ctx, "publish_profile", width, |ui| {
            dialogs::title(ui, tr("Publish Project"));
            let projects: Vec<String> = self
                .projects
                .iter()
                .map(|project| project.file_stem().unwrap_or(project.as_os_str()).to_string_lossy().into_owned())
                .collect();
            let project = projects.get(self.project_index).cloned().unwrap_or_default();
            let selected = self.selected_name.clone().unwrap_or_else(|| tr("New profile").to_string());
            let mut configurations: Vec<String> = CONFIGURATIONS.iter().map(|name| name.to_string()).collect();
            if !self.draft.configuration.is_empty() && !configurations.contains(&self.draft.configuration) {
                configurations.push(self.draft.configuration.clone());
            }
            egui::Grid::new("publish_fields").num_columns(2).spacing([12.0, 10.0]).show(ui, |ui| {
                ui.label(tr("Project"));
                next_project = kit::dropdown(ui, "publish_project", &project, &projects, !self.saving);
                ui.end_row();
                ui.label(tr("Publish profile"));
                ui.horizontal(|ui| {
                    next_profile = kit::dropdown(ui, "publish_profile_name", &selected, &self.names, ready);
                    create = kit::button(ui, Some(ico::ADD), tr("Add profile"), ready).clicked();
                });
                ui.end_row();
                if self.loading {
                    return;
                }
                ui.label(tr("Profile name"));
                ui.add_enabled(self.selected_name.is_none() && !self.saving, TextEdit::singleline(&mut self.draft.name).hint_text("FolderProfile").desired_width(field_width));
                ui.end_row();
                ui.label(tr("Configuration"));
                if let Some(index) = kit::dropdown(ui, "publish_configuration", &self.draft.configuration, &configurations, !self.saving) {
                    self.draft.configuration = configurations[index].clone();
                }
                ui.end_row();
                ui.label(tr("Output folder"));
                ui.add_enabled(!self.saving, TextEdit::singleline(&mut self.draft.publish_dir).desired_width(field_width));
                ui.end_row();
                ui.label(tr("Runtime identifier"));
                ui.add_enabled(!self.saving, TextEdit::singleline(&mut self.draft.runtime_identifier).hint_text("linux-x64").desired_width(field_width));
                ui.end_row();
                ui.label("");
                ui.add_enabled_ui(!self.saving, |ui| {
                    ui.vertical(|ui| {
                        ui.checkbox(&mut self.draft.self_contained, tr("Self-contained"));
                        ui.checkbox(&mut self.draft.single_file, tr("Single file"));
                        ui.checkbox(&mut self.draft.ready_to_run, tr("ReadyToRun"));
                        ui.checkbox(&mut self.draft.trimmed, tr("Trim unused code"));
                    });
                });
                ui.end_row();
            });
            if self.loading {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.spinner();
                    kit::note(ui, tr("Loading publish profiles…"));
                });
            }
            if let Some(error) = &self.error {
                ui.add_space(6.0);
                kit::message(ui, error, true);
            }
            ui.add_space(14.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                publish = ui.add_enabled_ui(ready, |ui| theme::primary_button(ui, tr("Publish"))).inner.clicked();
                cancel = theme::secondary_button(ui, tr("Cancel")).clicked();
                save = ui.add_enabled_ui(ready, |ui| theme::secondary_button(ui, tr("Save Profile"))).inner.clicked();
                if self.saving {
                    ui.spinner();
                }
            });
        });
        if let Some(index) = next_project
            && index != self.project_index
        {
            self.project_index = index;
            self.load(None);
        }
        if let Some(index) = next_profile
            && let Some(name) = self.names.get(index).cloned()
        {
            self.load(Some(name));
        }
        if create {
            self.create();
        }
        if save && ready {
            self.save(false);
        }
        if publish && ready {
            self.save(true);
        }
        if (cancel || escape) && !self.saving {
            PublishOutcome::Close
        } else {
            PublishOutcome::Keep
        }
    }
}
