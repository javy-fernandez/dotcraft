//! Tool windows: their stripe placement, icons, titles and the dock layout state.

use rc_core::session::ToolWindowState;
use rc_kit::i18n::tr;
use rc_kit::{Ico, ico};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ToolWindow {
    Explorer,
    Structure,
    Commit,
    Run,
    Debug,
    Tests,
    Build,
    Terminal,
    Problems,
    Git,
    Find,
    Notifications,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Bottom,
    Right,
}

impl Side {
    pub const ALL: [Side; 3] = [Side::Left, Side::Right, Side::Bottom];

    pub fn id(self) -> &'static str {
        match self {
            Side::Left => "left",
            Side::Bottom => "bottom",
            Side::Right => "right",
        }
    }

    pub fn from_id(id: &str) -> Option<Side> {
        Side::ALL.into_iter().find(|s| s.id() == id)
    }

    pub fn title(self) -> &'static str {
        match self {
            Side::Left => tr("Left"),
            Side::Bottom => tr("Bottom"),
            Side::Right => tr("Right"),
        }
    }
}

impl ToolWindow {
    /// Order of the buttons on the stripes.
    pub const ALL: [ToolWindow; 12] = [
        ToolWindow::Explorer,
        ToolWindow::Commit,
        ToolWindow::Structure,
        ToolWindow::Run,
        ToolWindow::Debug,
        ToolWindow::Tests,
        ToolWindow::Build,
        ToolWindow::Find,
        ToolWindow::Problems,
        ToolWindow::Terminal,
        ToolWindow::Git,
        ToolWindow::Notifications,
    ];

    pub fn id(self) -> &'static str {
        match self {
            ToolWindow::Explorer => "explorer",
            ToolWindow::Structure => "structure",
            ToolWindow::Commit => "commit",
            ToolWindow::Run => "run",
            ToolWindow::Debug => "debug",
            ToolWindow::Tests => "tests",
            ToolWindow::Build => "build",
            ToolWindow::Terminal => "terminal",
            ToolWindow::Problems => "problems",
            ToolWindow::Git => "git",
            ToolWindow::Find => "find",
            ToolWindow::Notifications => "notifications",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            ToolWindow::Explorer => tr("Explorer"),
            ToolWindow::Structure => tr("Structure"),
            ToolWindow::Commit => tr("Commit"),
            ToolWindow::Run => tr("Run"),
            ToolWindow::Debug => tr("Debug"),
            ToolWindow::Tests => tr("Tests"),
            ToolWindow::Build => tr("Build"),
            ToolWindow::Terminal => tr("Terminal"),
            ToolWindow::Problems => tr("Problems"),
            ToolWindow::Git => tr("Git"),
            ToolWindow::Find => tr("Find"),
            ToolWindow::Notifications => tr("Notifications"),
        }
    }

    pub fn icon(self) -> Ico {
        match self {
            ToolWindow::Explorer => ico::TW_PROJECT,
            ToolWindow::Structure => ico::TW_STRUCTURE,
            ToolWindow::Commit => ico::TW_COMMIT,
            ToolWindow::Run => ico::TW_RUN,
            ToolWindow::Debug => ico::TW_DEBUG,
            ToolWindow::Tests => ico::RUN_ALL,
            ToolWindow::Build => ico::TW_BUILD,
            ToolWindow::Terminal => ico::FILE_SHELL,
            ToolWindow::Problems => ico::TW_PROBLEMS,
            ToolWindow::Git => ico::TW_VCS,
            ToolWindow::Find => ico::TW_FIND,
            ToolWindow::Notifications => ico::TW_NOTIFICATIONS,
        }
    }

    pub fn side(self) -> Side {
        match self {
            ToolWindow::Explorer | ToolWindow::Structure | ToolWindow::Commit => Side::Left,
            ToolWindow::Notifications => Side::Right,
            _ => Side::Bottom,
        }
    }

    /// Whether the button sits on the lower part of the left stripe (bottom tool windows).
    pub fn default_size(self, side: Side) -> f32 {
        match side {
            Side::Left => 300.0,
            Side::Right => 320.0,
            Side::Bottom => 260.0,
        }
    }

    pub fn from_id(id: &str) -> Option<ToolWindow> {
        ToolWindow::ALL.into_iter().find(|t| t.id() == id)
    }
}

/// Which tool window is open on each side and how big each one is.
#[derive(Clone, Debug, PartialEq)]
pub struct Dock {
    pub left: Option<ToolWindow>,
    pub bottom: Option<ToolWindow>,
    pub right: Option<ToolWindow>,
    pub sizes: BTreeMap<ToolWindow, f32>,
    /// Tool windows moved away from their default side.
    pub sides: BTreeMap<ToolWindow, Side>,
}

impl Default for Dock {
    fn default() -> Self {
        Dock { left: Some(ToolWindow::Explorer), bottom: None, right: None, sizes: BTreeMap::new(), sides: BTreeMap::new() }
    }
}

impl Dock {
    pub fn open_on(&self, side: Side) -> Option<ToolWindow> {
        match side {
            Side::Left => self.left,
            Side::Bottom => self.bottom,
            Side::Right => self.right,
        }
    }

    fn slot(&mut self, side: Side) -> &mut Option<ToolWindow> {
        match side {
            Side::Left => &mut self.left,
            Side::Bottom => &mut self.bottom,
            Side::Right => &mut self.right,
        }
    }

    /// The side `tw` is docked on (its default unless moved).
    pub fn side_of(&self, tw: ToolWindow) -> Side {
        self.sides.get(&tw).copied().unwrap_or(tw.side())
    }

    pub fn is_open(&self, tw: ToolWindow) -> bool {
        self.open_on(self.side_of(tw)) == Some(tw)
    }

    /// Stripe click: opens `tw` (replacing the other window of its side) or hides it.
    pub fn toggle(&mut self, tw: ToolWindow) {
        let slot = self.slot(self.side_of(tw));
        *slot = if *slot == Some(tw) { None } else { Some(tw) };
    }

    pub fn show(&mut self, tw: ToolWindow) {
        *self.slot(self.side_of(tw)) = Some(tw);
    }

    /// "Move to": docks `tw` on `side`, keeping it open if it was.
    pub fn move_to(&mut self, tw: ToolWindow, side: Side) {
        let was_open = self.is_open(tw);
        if was_open {
            *self.slot(self.side_of(tw)) = None;
        }
        if side == tw.side() {
            self.sides.remove(&tw);
        } else {
            self.sides.insert(tw, side);
        }
        self.sizes.remove(&tw);
        if was_open {
            self.show(tw);
        }
    }

    pub fn hide_all(&mut self) {
        self.left = None;
        self.bottom = None;
        self.right = None;
    }

    pub fn size(&self, tw: ToolWindow) -> f32 {
        self.sizes.get(&tw).copied().unwrap_or_else(|| tw.default_size(self.side_of(tw)))
    }

    pub fn to_session(&self) -> BTreeMap<String, ToolWindowState> {
        ToolWindow::ALL
            .into_iter()
            .map(|tw| (tw.id().to_string(), ToolWindowState { visible: self.is_open(tw), size: self.size(tw), side: self.sides.get(&tw).map(|s| s.id().to_string()) }))
            .collect()
    }

    pub fn from_session(map: &BTreeMap<String, ToolWindowState>) -> Dock {
        if map.is_empty() {
            return Dock::default();
        }
        let mut dock = Dock { left: None, bottom: None, right: None, sizes: BTreeMap::new(), sides: BTreeMap::new() };
        for (id, st) in map {
            if let Some(tw) = ToolWindow::from_id(id) {
                dock.sizes.insert(tw, st.size);
                if let Some(side) = st.side.as_deref().and_then(Side::from_id).filter(|s| *s != tw.side()) {
                    dock.sides.insert(tw, side);
                }
            }
        }
        for (id, st) in map {
            if let Some(tw) = ToolWindow::from_id(id)
                && st.visible
            {
                dock.show(tw);
            }
        }
        dock
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_window_per_side_and_session_round_trip() {
        let mut d = Dock::default();
        d.toggle(ToolWindow::Terminal);
        d.toggle(ToolWindow::Build);
        assert_eq!(d.bottom, Some(ToolWindow::Build));
        d.toggle(ToolWindow::Build);
        assert_eq!(d.bottom, None);
        d.toggle(ToolWindow::Git);
        d.sizes.insert(ToolWindow::Explorer, 333.0);
        let back = Dock::from_session(&d.to_session());
        assert_eq!(back, Dock { sizes: back.sizes.clone(), ..d.clone() });
        assert_eq!(back.size(ToolWindow::Explorer), 333.0);
    }

    #[test]
    fn moving_a_window_to_another_side_keeps_it_open_and_is_remembered() {
        let mut d = Dock::default();
        d.move_to(ToolWindow::Explorer, Side::Right);
        assert_eq!((d.left, d.right), (None, Some(ToolWindow::Explorer)));
        d.toggle(ToolWindow::Commit);
        assert_eq!(d.left, Some(ToolWindow::Commit));
        let back = Dock::from_session(&d.to_session());
        assert_eq!(back.side_of(ToolWindow::Explorer), Side::Right);
        assert_eq!(back.right, Some(ToolWindow::Explorer));
        d.move_to(ToolWindow::Explorer, Side::Left);
        assert!(d.sides.is_empty());
        assert_eq!(d.left, Some(ToolWindow::Explorer));
    }
}
