//! Background loading and saving for the multi-culture `.resx` grid.

use std::path::{Path, PathBuf};

use rc_core::Runtime;
use rc_core::inbox::{Inbox, Waker};
use rc_dotnet::resx::{ResourceGroup, ResourceRow};
use rc_kit::i18n::tr;

enum Event {
    Loaded(u64, Result<ResourceGroup, String>),
    Saved(u64, Result<ResourceGroup, String>),
}

pub struct ResourceEditor {
    pub path: Option<PathBuf>,
    pub group: Option<ResourceGroup>,
    pub loading: bool,
    pub saving: bool,
    pub error: Option<String>,
    pub comment_column: usize,
    saved_rows: Vec<ResourceRow>,
    generation: u64,
    inbox: Inbox<Event>,
}

impl ResourceEditor {
    pub fn new(waker: Waker) -> Self {
        Self {
            path: None,
            group: None,
            loading: false,
            saving: false,
            error: None,
            comment_column: 0,
            saved_rows: Vec::new(),
            generation: 0,
            inbox: Inbox::new(waker),
        }
    }

    pub fn dirty(&self) -> bool {
        self.group
            .as_ref()
            .is_some_and(|group| group.rows != self.saved_rows)
    }

    pub fn open(&mut self, path: &Path) -> bool {
        if self.path.as_deref() == Some(path) {
            if !self.dirty() && !self.loading && !self.saving {
                self.load(path.to_path_buf());
            }
            return true;
        }
        if self.dirty() || self.saving {
            self.error =
                Some(tr("Save or discard resource changes before opening another file").into());
            return false;
        }
        self.load(path.to_path_buf());
        true
    }

    pub fn refresh(&mut self, discard: bool) {
        if self.dirty() && !discard {
            self.error = Some(tr("Save or discard resource changes before refreshing").into());
            return;
        }
        if let Some(path) = self.path.clone() {
            self.load(path);
        }
    }

    fn load(&mut self, path: PathBuf) {
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.path = Some(path.clone());
        self.group = None;
        self.saved_rows.clear();
        self.loading = true;
        self.saving = false;
        self.error = None;
        let out = self.inbox.sender();
        Runtime::handle().spawn_blocking(move || {
            out.send(Event::Loaded(
                generation,
                ResourceGroup::load(&path).map_err(|error| format!("{error:#}")),
            ));
        });
    }

    pub fn save(&mut self) {
        if self.saving || !self.dirty() {
            return;
        }
        let Some(group) = self.group.clone() else {
            return;
        };
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.saving = true;
        self.error = None;
        let out = self.inbox.sender();
        Runtime::handle().spawn_blocking(move || {
            out.send(Event::Saved(
                generation,
                group.save().map_err(|error| format!("{error:#}")),
            ));
        });
    }

    pub fn update(&mut self) {
        for event in self.inbox.drain() {
            match event {
                Event::Loaded(generation, result) if generation == self.generation => {
                    self.loading = false;
                    self.finish(result);
                }
                Event::Saved(generation, result) if generation == self.generation => {
                    self.saving = false;
                    self.finish(result);
                }
                _ => {}
            }
        }
    }

    fn finish(&mut self, result: Result<ResourceGroup, String>) {
        match result {
            Ok(group) => {
                self.saved_rows = group.rows.clone();
                self.comment_column = self.comment_column.min(group.files.len().saturating_sub(1));
                self.group = Some(group);
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
    }
}
