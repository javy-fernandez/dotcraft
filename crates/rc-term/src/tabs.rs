//! The Terminal tool window's tabs: a list of sessions and the active one. No chrome: the
//! application draws the tab strip and puts a [`crate::TerminalView`] of the active session
//! below it.

use crate::session::{SessionEvent, SpawnSpec, TermSession};
use rc_core::Waker;
use rc_kit::i18n::tr;

pub struct TermTab {
    pub session: TermSession,
    /// Name the user gave the tab ("Rename Session").
    pub custom_title: Option<String>,
    /// Number of the default title ("Local (2)"), fixed when the tab is opened.
    number: usize,
}

impl TermTab {
    pub fn id(&self) -> u64 {
        self.session.id()
    }

    /// The custom name, else the application's title, else "Local" like IntelliJ.
    pub fn title(&self) -> String {
        if let Some(t) = &self.custom_title {
            return t.clone();
        }
        if let Some(t) = self.session.title().filter(|t| !t.trim().is_empty()) {
            return t;
        }
        if self.number <= 1 { tr("Local").to_string() } else { format!("{} ({})", tr("Local"), self.number) }
    }
}

#[derive(Default)]
pub struct Terminals {
    tabs: Vec<TermTab>,
    active: usize,
}

impl Terminals {
    pub fn new() -> Self {
        Self::default()
    }

    /// Starts a session in a new tab and makes it active; returns its id.
    pub fn open(&mut self, spec: &SpawnSpec, waker: Waker) -> anyhow::Result<u64> {
        let session = TermSession::spawn(spec, waker)?;
        Ok(self.push(session))
    }

    /// Adds an existing session as a new active tab; returns its id.
    pub fn push(&mut self, session: TermSession) -> u64 {
        let used: Vec<usize> = self.tabs.iter().map(|t| t.number).collect();
        let number = (1..).find(|n| !used.contains(n)).unwrap_or(1);
        let id = session.id();
        self.tabs.push(TermTab { session, custom_title: None, number });
        self.active = self.tabs.len() - 1;
        id
    }

    /// Closes a tab (killing its process). The tab to its left becomes active.
    pub fn close(&mut self, id: u64) -> bool {
        let Some(i) = self.index_of(id) else { return false };
        self.tabs.remove(i);
        if self.active >= self.tabs.len() || (i < self.active) || (i == self.active && i > 0) {
            self.active = self.active.saturating_sub(1);
        }
        self.active = self.active.min(self.tabs.len().saturating_sub(1));
        true
    }

    pub fn tabs(&self) -> &[TermTab] {
        &self.tabs
    }

    pub fn tabs_mut(&mut self) -> &mut [TermTab] {
        &mut self.tabs
    }

    pub fn len(&self) -> usize {
        self.tabs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }

    pub fn active_index(&self) -> usize {
        self.active
    }

    pub fn set_active(&mut self, index: usize) {
        if index < self.tabs.len() {
            self.active = index;
        }
    }

    pub fn activate(&mut self, id: u64) {
        if let Some(i) = self.index_of(id) {
            self.active = i;
        }
    }

    pub fn active(&self) -> Option<&TermTab> {
        self.tabs.get(self.active)
    }

    pub fn active_mut(&mut self) -> Option<&mut TermTab> {
        self.tabs.get_mut(self.active)
    }

    pub fn get(&self, id: u64) -> Option<&TermTab> {
        self.tabs.iter().find(|t| t.id() == id)
    }

    pub fn rename(&mut self, id: u64, title: Option<String>) {
        if let Some(i) = self.index_of(id) {
            self.tabs[i].custom_title = title.filter(|t| !t.trim().is_empty());
        }
    }

    /// Titles of every tab, in order.
    pub fn titles(&self) -> Vec<String> {
        self.tabs.iter().map(TermTab::title).collect()
    }

    /// Moves a tab (drag and drop in the tab strip), keeping the same tab active.
    pub fn move_tab(&mut self, from: usize, to: usize) {
        if from >= self.tabs.len() || to >= self.tabs.len() || from == to {
            return;
        }
        let active_id = self.active().map(TermTab::id);
        let tab = self.tabs.remove(from);
        self.tabs.insert(to, tab);
        if let Some(id) = active_id {
            self.activate(id);
        }
    }

    /// Drains every session's events (call once per frame).
    pub fn poll(&mut self) -> Vec<(u64, SessionEvent)> {
        self.tabs.iter().flat_map(|t| t.session.poll_events().into_iter().map(move |e| (t.id(), e))).collect()
    }

    fn index_of(&self, id: u64) -> Option<usize> {
        self.tabs.iter().position(|t| t.id() == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tab(t: &mut Terminals) -> u64 {
        t.push(TermSession::output_only(20, 5, Waker::noop()))
    }

    #[test]
    fn open_close_and_titles() {
        let mut t = Terminals::new();
        let a = tab(&mut t);
        let b = tab(&mut t);
        let c = tab(&mut t);
        assert_eq!(t.active_index(), 2);
        assert_eq!(t.titles(), vec!["Local", "Local (2)", "Local (3)"]);
        t.activate(b);
        assert!(t.close(b));
        assert_eq!(t.active().unwrap().id(), a);
        // The freed number is reused.
        let d = tab(&mut t);
        assert_eq!(t.get(d).unwrap().title(), "Local (2)");
        t.rename(c, Some("Build".into()));
        assert_eq!(t.get(c).unwrap().title(), "Build");
        t.get(a).unwrap().session.feed(b"\x1b]0;vim main.rs\x07");
        t.poll();
        assert_eq!(t.get(a).unwrap().title(), "vim main.rs");
        t.move_tab(0, 2);
        assert_eq!(t.tabs()[2].id(), a);
        assert_eq!(t.active().unwrap().id(), d);
        assert!(t.close(a) && t.close(c) && t.close(d));
        assert!(t.is_empty() && t.active().is_none());
    }
}
