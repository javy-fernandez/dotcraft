//! Create and edit folder publish profiles, then stream `dotnet publish` in the Build window.

use std::path::PathBuf;

use egui::{Context, TextEdit};
use rc_core::{Inbox, Runtime, Waker};
use rc_dotnet::{BuildRequest, publish::PublishProfile};
use rc_kit::i18n::tr;

pub enum PublishOutcome {
    Keep,
    Close,
    Start(BuildRequest),
}

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
        if self.selected_name.as_deref() != Some(self.draft.name.as_str())
            && self.names.contains(&self.draft.name)
        {
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

    pub fn show(&mut self, ctx: &Context) -> PublishOutcome {
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
        for result in self.saved.drain() {
            self.saving = false;
            match result {
                Ok((profile, request)) => {
                    if let Some(request) = request {
                        return PublishOutcome::Start(request);
                    }
                    if !self.names.contains(&profile.name) {
                        self.names.push(profile.name.clone());
                        self.names.sort_unstable();
                    }
                    self.selected_name = Some(profile.name.clone());
                    self.draft = profile;
                }
                Err(error) => self.error = Some(error),
            }
        }
        let mut next_project = None;
        let mut next_profile = None;
        let mut create = false;
        let mut save = false;
        let mut publish = false;
        let mut close = false;
        let response = egui::Modal::new(egui::Id::new("publish_profile")).show(ctx, |ui| {
            ui.set_width((ctx.content_rect().width() - 48.0).clamp(340.0, 650.0));
            super::dialogs::title(ui, tr("Publish Project"));
            ui.add_enabled_ui(!self.saving && !self.loading, |ui| {
                ui.horizontal(|ui| {
                    ui.label(tr("Project"));
                    egui::ComboBox::from_id_salt("publish_project")
                        .selected_text(
                            self.projects
                                .get(self.project_index)
                                .and_then(|project| project.file_name())
                                .map(|name| name.to_string_lossy())
                                .unwrap_or_default(),
                        )
                        .show_ui(ui, |ui| {
                            for (index, project) in self.projects.iter().enumerate() {
                                let label = project
                                    .file_name()
                                    .unwrap_or(project.as_os_str())
                                    .to_string_lossy();
                                if ui
                                    .selectable_label(index == self.project_index, label)
                                    .clicked()
                                {
                                    next_project = Some(index);
                                }
                            }
                        });
                });
                ui.horizontal(|ui| {
                    ui.label(tr("Publish profile"));
                    egui::ComboBox::from_id_salt("publish_profile_select")
                        .selected_text(self.selected_name.as_deref().unwrap_or(tr("New profile")))
                        .show_ui(ui, |ui| {
                            for name in &self.names {
                                if ui
                                    .selectable_label(
                                        self.selected_name.as_deref() == Some(name.as_str()),
                                        name,
                                    )
                                    .clicked()
                                {
                                    next_profile = Some(name.clone());
                                }
                            }
                        });
                    if ui.button(tr("New profile")).clicked() {
                        create = true;
                    }
                });
                egui::Grid::new("publish_properties")
                    .num_columns(2)
                    .spacing([12.0, 8.0])
                    .show(ui, |ui| {
                        ui.label(tr("Profile name"));
                        ui.add_enabled(
                            self.selected_name.is_none(),
                            TextEdit::singleline(&mut self.draft.name).desired_width(380.0),
                        );
                        ui.end_row();
                        ui.label(tr("Configuration"));
                        ui.add(
                            TextEdit::singleline(&mut self.draft.configuration)
                                .desired_width(380.0),
                        );
                        ui.end_row();
                        ui.label(tr("Output folder"));
                        ui.add(
                            TextEdit::singleline(&mut self.draft.publish_dir).desired_width(380.0),
                        );
                        ui.end_row();
                        ui.label(tr("Runtime identifier"));
                        ui.add(
                            TextEdit::singleline(&mut self.draft.runtime_identifier)
                                .hint_text("linux-x64")
                                .desired_width(380.0),
                        );
                        ui.end_row();
                    });
                ui.checkbox(&mut self.draft.self_contained, tr("Self-contained"));
                ui.checkbox(&mut self.draft.single_file, tr("Single file"));
                ui.checkbox(&mut self.draft.ready_to_run, tr("ReadyToRun"));
                ui.checkbox(&mut self.draft.trimmed, tr("Trim unused code"));
            });
            if self.loading {
                ui.spinner();
                ui.label(tr("Loading publish profiles…"));
            }
            if let Some(error) = &self.error {
                ui.colored_label(ui.visuals().error_fg_color, error);
            }
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        !self.loading && !self.saving,
                        egui::Button::new(tr("Save Profile")),
                    )
                    .clicked()
                {
                    save = true;
                }
                if ui
                    .add_enabled(
                        !self.loading && !self.saving,
                        egui::Button::new(tr("Publish")),
                    )
                    .clicked()
                {
                    publish = true;
                }
                if ui
                    .add_enabled(!self.saving, egui::Button::new(tr("Cancel")))
                    .clicked()
                {
                    close = true;
                }
                if self.saving {
                    ui.spinner();
                }
            });
        });
        if let Some(index) = next_project {
            self.project_index = index;
            self.load(None);
        }
        if let Some(name) = next_profile {
            self.load(Some(name));
        }
        if create {
            self.create();
        }
        if save {
            self.save(false);
        }
        if publish {
            self.save(true);
        }
        if close || (response.should_close() && !self.saving) {
            PublishOutcome::Close
        } else {
            PublishOutcome::Keep
        }
    }
}
