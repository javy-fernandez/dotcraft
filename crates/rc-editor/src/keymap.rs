//! Rider keymaps: every shortcut of the IDE maps to an [`Action`]. The editor handles the
//! editing actions itself and reports the rest to the application.

use egui::{Key, Modifiers};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    // --- caret movement (the bool is "extend selection")
    Left(bool),
    Right(bool),
    Up(bool),
    Down(bool),
    WordLeft(bool),
    WordRight(bool),
    LineStart(bool),
    LineEnd(bool),
    DocStart(bool),
    DocEnd(bool),
    PageUp(bool),
    PageDown(bool),
    ScrollUp,
    ScrollDown,
    // --- editing
    Enter,
    StartNewLine,
    StartNewLineBefore,
    Backspace,
    Delete,
    DeleteWordLeft,
    DeleteWordRight,
    DeleteLine,
    DuplicateLine,
    MoveLineUp,
    MoveLineDown,
    CommentLine,
    CommentBlock,
    Indent,
    Unindent,
    JoinLines,
    ToggleCase,
    Undo,
    Redo,
    Copy,
    Cut,
    Paste,
    SelectAll,
    ExtendSelection,
    ShrinkSelection,
    SelectNextOccurrence,
    UnselectOccurrence,
    SelectAllOccurrences,
    CloneCaretAbove,
    CloneCaretBelow,
    Escape,
    // --- folding
    Fold,
    Unfold,
    FoldAll,
    UnfoldAll,
    // --- code insight (reported to the app)
    Complete,
    ParameterInfo,
    QuickDoc,
    ShowIntentions,
    GotoDeclaration,
    GotoImplementation,
    GotoTypeDeclaration,
    FindUsages,
    Rename,
    Reformat,
    FileStructure,
    GotoLine,
    Find,
    Replace,
    FindNext,
    FindPrevious,
    NextError,
    PreviousError,
    ToggleBreakpoint,
    // --- IDE (reported to the app)
    SearchEverywhere,
    GotoFile,
    GotoSymbol,
    GotoClass,
    FindInFiles,
    ReplaceInFiles,
    RecentFiles,
    RecentLocations,
    NavigateBack,
    NavigateForward,
    Save,
    SaveAll,
    CloseTab,
    NextTab,
    PreviousTab,
    Build,
    Run,
    Debug,
    StopRun,
    RunTests,
    StepOver,
    StepInto,
    StepOut,
    Resume,
    RunToCursor,
    Evaluate,
    ShowSettings,
    ToggleProjectWindow,
    ToggleTerminal,
    ToggleGitWindow,
    ToggleProblems,
    HideToolWindows,
    Commit,
    OpenFolder,
    ZoomIn,
    ZoomOut,
    ZoomReset,
}

impl Action {
    /// Actions the editor widget executes itself.
    pub fn is_editing(self) -> bool {
        use Action::*;
        !matches!(
            self,
            Complete
                | ParameterInfo
                | QuickDoc
                | ShowIntentions
                | GotoDeclaration
                | GotoImplementation
                | GotoTypeDeclaration
                | FindUsages
                | Rename
                | Reformat
                | FileStructure
                | GotoLine
                | Find
                | Replace
                | FindNext
                | FindPrevious
                | NextError
                | PreviousError
                | ToggleBreakpoint
        ) && !self.is_ide()
    }

    /// Global IDE actions (they work without an editor).
    pub fn is_ide(self) -> bool {
        use Action::*;
        matches!(
            self,
            SearchEverywhere
                | GotoFile
                | GotoSymbol
                | GotoClass
                | FindInFiles
                | ReplaceInFiles
                | RecentFiles
                | RecentLocations
                | NavigateBack
                | NavigateForward
                | Save
                | SaveAll
                | CloseTab
                | NextTab
                | PreviousTab
                | Build
                | Run
                | Debug
                | StopRun
                | RunTests
                | StepOver
                | StepInto
                | StepOut
                | Resume
                | RunToCursor
                | Evaluate
                | ShowSettings
                | ToggleProjectWindow
                | ToggleTerminal
                | ToggleGitWindow
                | ToggleProblems
                | HideToolWindows
                | Commit
                | OpenFolder
                | ZoomIn
                | ZoomOut
                | ZoomReset
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum KeymapKind {
    /// Rider's default keymap on macOS.
    #[default]
    RiderMac,
    /// Rider's default keymap on Windows/Linux.
    Rider,
    /// Visual Studio keymap (Windows).
    VisualStudio,
}

/// A key with modifiers. `cmd` is ⌘ on macOS and Ctrl elsewhere (egui's `command`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chord {
    pub key: Key,
    pub mods: Modifiers,
}

const fn m(alt: bool, ctrl: bool, shift: bool, mac_cmd: bool) -> Modifiers {
    Modifiers { alt, ctrl, shift, mac_cmd, command: mac_cmd || ctrl }
}

pub struct Keymap {
    pub kind: KeymapKind,
    bindings: Vec<(Chord, Action)>,
}

impl Keymap {
    pub fn new(kind: KeymapKind) -> Self {
        let bindings = match kind {
            KeymapKind::RiderMac => mac_bindings(),
            KeymapKind::Rider => win_bindings(false),
            KeymapKind::VisualStudio => win_bindings(true),
        };
        Keymap { kind, bindings }
    }

    /// The action bound to `key` with `mods`, if any. Modifiers must match exactly.
    pub fn lookup(&self, key: Key, mods: Modifiers) -> Option<Action> {
        let norm = |x: Modifiers| (x.alt, x.ctrl, x.shift, x.mac_cmd);
        self.bindings.iter().find(|(c, _)| c.key == key && norm(c.mods) == norm(mods)).map(|(_, a)| *a)
    }

    /// The first shortcut of `action`, formatted for menus and tooltips (⌘⇧F, Ctrl+Shift+F).
    pub fn shortcut_text(&self, action: Action) -> Option<String> {
        let (chord, _) = self.bindings.iter().find(|(_, a)| *a == action)?;
        Some(format_chord(*chord, self.kind == KeymapKind::RiderMac))
    }

    pub fn chord(&self, action: Action) -> Option<Chord> {
        self.bindings.iter().find(|(_, a)| *a == action).map(|(c, _)| *c)
    }
}

pub fn format_chord(c: Chord, mac: bool) -> String {
    let key = key_name(c.key, mac);
    if mac {
        let mut s = String::new();
        if c.mods.ctrl {
            s.push('⌃');
        }
        if c.mods.alt {
            s.push('⌥');
        }
        if c.mods.shift {
            s.push('⇧');
        }
        if c.mods.mac_cmd {
            s.push('⌘');
        }
        s + &key
    } else {
        let mut parts: Vec<String> = Vec::new();
        if c.mods.ctrl {
            parts.push("Ctrl".into());
        }
        if c.mods.alt {
            parts.push("Alt".into());
        }
        if c.mods.shift {
            parts.push("Shift".into());
        }
        parts.push(key);
        parts.join("+")
    }
}

fn key_name(key: Key, mac: bool) -> String {
    match key {
        Key::ArrowUp => "↑".into(),
        Key::ArrowDown => "↓".into(),
        Key::ArrowLeft => "←".into(),
        Key::ArrowRight => "→".into(),
        Key::Enter => if mac { "↩".into() } else { "Enter".into() },
        Key::Backspace => if mac { "⌫".into() } else { "Backspace".into() },
        Key::Delete => if mac { "⌦".into() } else { "Delete".into() },
        Key::Escape => if mac { "⎋".into() } else { "Esc".into() },
        Key::Tab => if mac { "⇥".into() } else { "Tab".into() },
        Key::Space => "Space".into(),
        Key::Home => "Home".into(),
        Key::End => "End".into(),
        Key::PageUp => "PgUp".into(),
        Key::PageDown => "PgDn".into(),
        Key::Slash => "/".into(),
        Key::Minus => "-".into(),
        Key::Equals | Key::Plus => "=".into(),
        Key::Period => ".".into(),
        Key::Comma => ",".into(),
        Key::OpenBracket => "[".into(),
        Key::CloseBracket => "]".into(),
        Key::Num0 => "0".into(),
        Key::Num1 => "1".into(),
        Key::Num9 => "9".into(),
        k => k.name().to_string(),
    }
}

fn mac_bindings() -> Vec<(Chord, Action)> {
    use Action::*;
    let none = m(false, false, false, false);
    let s = m(false, false, true, false);
    let c = m(false, false, false, true);
    let cs = m(false, false, true, true);
    let a = m(true, false, false, false);
    let as_ = m(true, false, true, false);
    let ca = m(true, false, false, true);
    let cas = m(true, false, true, true);
    let ctrl = m(false, true, false, false);
    let ctrl_s = m(false, true, true, false);
    let ctrl_c = m(false, true, false, true);
    let b = |key, mods, action| (Chord { key, mods }, action);
    vec![
        b(Key::ArrowLeft, none, Left(false)),
        b(Key::ArrowLeft, s, Left(true)),
        b(Key::ArrowRight, none, Right(false)),
        b(Key::ArrowRight, s, Right(true)),
        b(Key::ArrowUp, none, Up(false)),
        b(Key::ArrowUp, s, Up(true)),
        b(Key::ArrowDown, none, Down(false)),
        b(Key::ArrowDown, s, Down(true)),
        b(Key::ArrowLeft, a, WordLeft(false)),
        b(Key::ArrowLeft, as_, WordLeft(true)),
        b(Key::ArrowRight, a, WordRight(false)),
        b(Key::ArrowRight, as_, WordRight(true)),
        b(Key::ArrowLeft, c, LineStart(false)),
        b(Key::ArrowLeft, cs, LineStart(true)),
        b(Key::ArrowRight, c, LineEnd(false)),
        b(Key::ArrowRight, cs, LineEnd(true)),
        b(Key::Home, none, LineStart(false)),
        b(Key::Home, s, LineStart(true)),
        b(Key::End, none, LineEnd(false)),
        b(Key::End, s, LineEnd(true)),
        b(Key::Home, c, DocStart(false)),
        b(Key::Home, cs, DocStart(true)),
        b(Key::End, c, DocEnd(false)),
        b(Key::End, cs, DocEnd(true)),
        b(Key::ArrowUp, c, DocStart(false)),
        b(Key::ArrowUp, cs, DocStart(true)),
        b(Key::ArrowDown, c, DocEnd(false)),
        b(Key::ArrowDown, cs, DocEnd(true)),
        b(Key::PageUp, none, PageUp(false)),
        b(Key::PageUp, s, PageUp(true)),
        b(Key::PageDown, none, PageDown(false)),
        b(Key::PageDown, s, PageDown(true)),
        b(Key::Enter, none, Enter),
        b(Key::Enter, s, StartNewLine),
        b(Key::Enter, ca, StartNewLineBefore),
        b(Key::Backspace, none, Backspace),
        b(Key::Backspace, s, Backspace),
        b(Key::Delete, none, Delete),
        b(Key::Backspace, a, DeleteWordLeft),
        b(Key::Delete, a, DeleteWordRight),
        b(Key::Backspace, c, DeleteLine),
        b(Key::D, c, DuplicateLine),
        b(Key::ArrowUp, as_, MoveLineUp),
        b(Key::ArrowDown, as_, MoveLineDown),
        b(Key::Slash, c, CommentLine),
        b(Key::Slash, ca, CommentBlock),
        b(Key::Tab, none, Indent),
        b(Key::Tab, s, Unindent),
        b(Key::J, ctrl_s, JoinLines),
        b(Key::U, cs, ToggleCase),
        b(Key::Z, c, Undo),
        b(Key::Z, cs, Redo),
        b(Key::C, c, Copy),
        b(Key::X, c, Cut),
        b(Key::V, c, Paste),
        b(Key::A, c, SelectAll),
        b(Key::ArrowUp, a, ExtendSelection),
        b(Key::ArrowDown, a, ShrinkSelection),
        b(Key::G, ctrl, SelectNextOccurrence),
        b(Key::G, ctrl_s, UnselectOccurrence),
        b(Key::G, ctrl_c, SelectAllOccurrences),
        b(Key::Escape, none, Escape),
        b(Key::Minus, c, Fold),
        b(Key::Equals, c, Unfold),
        b(Key::Plus, c, Unfold),
        b(Key::Minus, cs, FoldAll),
        b(Key::Equals, cs, UnfoldAll),
        b(Key::Plus, cs, UnfoldAll),
        b(Key::Space, ctrl, Complete),
        b(Key::P, c, ParameterInfo),
        b(Key::F1, none, QuickDoc),
        b(Key::J, ctrl, QuickDoc),
        b(Key::Enter, a, ShowIntentions),
        b(Key::B, c, GotoDeclaration),
        b(Key::B, ca, GotoImplementation),
        b(Key::B, cs, GotoTypeDeclaration),
        b(Key::F7, a, FindUsages),
        b(Key::F6, s, Rename),
        b(Key::L, ca, Reformat),
        b(Key::F12, c, FileStructure),
        b(Key::L, c, GotoLine),
        b(Key::F, c, Find),
        b(Key::R, c, Replace),
        b(Key::G, c, FindNext),
        b(Key::G, cs, FindPrevious),
        b(Key::F3, none, FindNext),
        b(Key::F3, s, FindPrevious),
        b(Key::F2, none, NextError),
        b(Key::F2, s, PreviousError),
        b(Key::F8, c, ToggleBreakpoint),
        b(Key::O, cs, GotoFile),
        b(Key::O, cas, GotoSymbol),
        b(Key::O, c, GotoClass),
        b(Key::T, c, SearchEverywhere),
        b(Key::F, cs, FindInFiles),
        b(Key::R, cs, ReplaceInFiles),
        b(Key::E, c, RecentFiles),
        b(Key::E, cs, RecentLocations),
        b(Key::OpenBracket, c, NavigateBack),
        b(Key::CloseBracket, c, NavigateForward),
        b(Key::ArrowLeft, ca, NavigateBack),
        b(Key::ArrowRight, ca, NavigateForward),
        b(Key::S, c, SaveAll),
        b(Key::W, c, CloseTab),
        b(Key::CloseBracket, cs, NextTab),
        b(Key::OpenBracket, cs, PreviousTab),
        b(Key::F9, c, Build),
        b(Key::R, ctrl, Run),
        b(Key::D, ctrl, Debug),
        b(Key::F2, c, StopRun),
        b(Key::F8, none, StepOver),
        b(Key::F7, none, StepInto),
        b(Key::F8, s, StepOut),
        b(Key::R, ca, Resume),
        b(Key::F9, a, RunToCursor),
        b(Key::F8, a, Evaluate),
        b(Key::Comma, c, ShowSettings),
        b(Key::Num1, c, ToggleProjectWindow),
        b(Key::F12, a, ToggleTerminal),
        b(Key::Num9, c, ToggleGitWindow),
        b(Key::Num6, c, ToggleProblems),
        b(Key::F12, cs, HideToolWindows),
        b(Key::K, c, Commit),
        b(Key::O, ca, OpenFolder),
        b(Key::Equals, ca, ZoomIn),
        b(Key::Plus, ca, ZoomIn),
        b(Key::Minus, ca, ZoomOut),
        b(Key::Num0, ca, ZoomReset),
    ]
}

fn win_bindings(visual_studio: bool) -> Vec<(Chord, Action)> {
    use Action::*;
    let none = m(false, false, false, false);
    let s = m(false, false, true, false);
    let c = m(false, true, false, false);
    let cs = m(false, true, true, false);
    let a = m(true, false, false, false);
    let as_ = m(true, false, true, false);
    let ca = m(true, true, false, false);
    let cas = m(true, true, true, false);
    let b = |key, mods, action| (Chord { key, mods }, action);
    let mut v = vec![
        b(Key::ArrowLeft, none, Left(false)),
        b(Key::ArrowLeft, s, Left(true)),
        b(Key::ArrowRight, none, Right(false)),
        b(Key::ArrowRight, s, Right(true)),
        b(Key::ArrowUp, none, Up(false)),
        b(Key::ArrowUp, s, Up(true)),
        b(Key::ArrowDown, none, Down(false)),
        b(Key::ArrowDown, s, Down(true)),
        b(Key::ArrowLeft, c, WordLeft(false)),
        b(Key::ArrowLeft, cs, WordLeft(true)),
        b(Key::ArrowRight, c, WordRight(false)),
        b(Key::ArrowRight, cs, WordRight(true)),
        b(Key::Home, none, LineStart(false)),
        b(Key::Home, s, LineStart(true)),
        b(Key::End, none, LineEnd(false)),
        b(Key::End, s, LineEnd(true)),
        b(Key::Home, c, DocStart(false)),
        b(Key::Home, cs, DocStart(true)),
        b(Key::End, c, DocEnd(false)),
        b(Key::End, cs, DocEnd(true)),
        b(Key::PageUp, none, PageUp(false)),
        b(Key::PageUp, s, PageUp(true)),
        b(Key::PageDown, none, PageDown(false)),
        b(Key::PageDown, s, PageDown(true)),
        b(Key::ArrowUp, c, ScrollUp),
        b(Key::ArrowDown, c, ScrollDown),
        b(Key::Enter, none, Enter),
        b(Key::Enter, s, StartNewLine),
        b(Key::Enter, ca, StartNewLineBefore),
        b(Key::Backspace, none, Backspace),
        b(Key::Backspace, s, Backspace),
        b(Key::Delete, none, Delete),
        b(Key::Backspace, c, DeleteWordLeft),
        b(Key::Delete, c, DeleteWordRight),
        b(Key::Tab, none, Indent),
        b(Key::Tab, s, Unindent),
        b(Key::J, cs, JoinLines),
        b(Key::U, cs, ToggleCase),
        b(Key::Z, c, Undo),
        b(Key::Z, cs, Redo),
        b(Key::C, c, Copy),
        b(Key::X, c, Cut),
        b(Key::V, c, Paste),
        b(Key::Insert, c, Copy),
        b(Key::Insert, s, Paste),
        b(Key::Delete, s, Cut),
        b(Key::A, c, SelectAll),
        b(Key::Escape, none, Escape),
        b(Key::Space, c, Complete),
        b(Key::P, c, ParameterInfo),
        b(Key::Enter, a, ShowIntentions),
        b(Key::F7, a, FindUsages),
        b(Key::G, c, GotoLine),
        b(Key::F, c, Find),
        b(Key::H, c, Replace),
        b(Key::F3, none, FindNext),
        b(Key::F3, s, FindPrevious),
        b(Key::F2, none, NextError),
        b(Key::F2, s, PreviousError),
        b(Key::F9, none, ToggleBreakpoint),
        b(Key::F, cs, FindInFiles),
        b(Key::H, cs, ReplaceInFiles),
        b(Key::Minus, ca, NavigateBack),
        b(Key::S, c, Save),
        b(Key::S, cs, SaveAll),
        b(Key::F4, c, CloseTab),
        b(Key::Tab, c, NextTab),
        b(Key::Tab, cs, PreviousTab),
        b(Key::F5, none, Debug),
        b(Key::F5, c, Run),
        b(Key::F5, s, StopRun),
        b(Key::F10, none, StepOver),
        b(Key::F11, none, StepInto),
        b(Key::F11, s, StepOut),
        b(Key::F10, c, RunToCursor),
        b(Key::F9, s, Evaluate),
    ];
    if visual_studio {
        v.extend([
            b(Key::F12, none, GotoDeclaration),
            b(Key::F12, c, GotoImplementation),
            b(Key::L, cs, DeleteLine),
            b(Key::D, c, DuplicateLine),
            b(Key::ArrowUp, a, MoveLineUp),
            b(Key::ArrowDown, a, MoveLineDown),
            b(Key::Slash, c, CommentLine),
            b(Key::W, c, ExtendSelection),
            b(Key::W, cs, ShrinkSelection),
            b(Key::Period, cs, SelectNextOccurrence),
            b(Key::Comma, cs, UnselectOccurrence),
            b(Key::Semicolon, cs, SelectAllOccurrences),
            b(Key::R, c, Rename),
            b(Key::R, ca, Rename),
            b(Key::K, ca, Reformat),
            b(Key::T, c, SearchEverywhere),
            b(Key::T, cs, GotoFile),
            b(Key::Comma, c, SearchEverywhere),
            b(Key::Q, c, QuickDoc),
            b(Key::Tab, ca, RecentFiles),
            b(Key::B, cs, Build),
            b(Key::Comma, ca, ShowSettings),
            b(Key::L, ca, ToggleProjectWindow),
            b(Key::Backtick, c, ToggleTerminal),
            b(Key::Num0, ca, ToggleGitWindow),
            b(Key::M, cs, Fold),
            b(Key::L, c, Unfold),
            b(Key::Equals, c, ZoomIn),
            b(Key::Plus, c, ZoomIn),
            b(Key::Minus, c, ZoomOut),
            b(Key::Num0, c, ZoomReset),
        ]);
    } else {
        v.extend([
            b(Key::Y, c, DeleteLine),
            b(Key::D, c, DuplicateLine),
            b(Key::ArrowUp, as_, MoveLineUp),
            b(Key::ArrowDown, as_, MoveLineDown),
            b(Key::Slash, c, CommentLine),
            b(Key::Slash, cs, CommentBlock),
            b(Key::W, c, ExtendSelection),
            b(Key::W, cs, ShrinkSelection),
            b(Key::J, a, SelectNextOccurrence),
            b(Key::J, as_, UnselectOccurrence),
            b(Key::J, cas, SelectAllOccurrences),
            b(Key::Minus, c, Fold),
            b(Key::Equals, c, Unfold),
            b(Key::Plus, c, Unfold),
            b(Key::Minus, cs, FoldAll),
            b(Key::Equals, cs, UnfoldAll),
            b(Key::Q, c, QuickDoc),
            b(Key::B, c, GotoDeclaration),
            b(Key::B, ca, GotoImplementation),
            b(Key::B, cs, GotoTypeDeclaration),
            b(Key::F6, s, Rename),
            b(Key::R, cs, Rename),
            b(Key::L, ca, Reformat),
            b(Key::F12, c, FileStructure),
            b(Key::R, c, Replace),
            b(Key::N, cs, GotoFile),
            b(Key::N, cas, GotoSymbol),
            b(Key::N, c, GotoClass),
            b(Key::T, c, SearchEverywhere),
            b(Key::E, c, RecentFiles),
            b(Key::E, cs, RecentLocations),
            b(Key::ArrowLeft, ca, NavigateBack),
            b(Key::ArrowRight, ca, NavigateForward),
            b(Key::F9, cs, Build),
            b(Key::F5, a, Resume),
            b(Key::F8, a, Evaluate),
            b(Key::S, ca, ShowSettings),
            b(Key::Num1, a, ToggleProjectWindow),
            b(Key::F12, a, ToggleTerminal),
            b(Key::Num9, a, ToggleGitWindow),
            b(Key::Num6, a, ToggleProblems),
            b(Key::F12, cs, HideToolWindows),
            b(Key::K, c, Commit),
            b(Key::Equals, cas, ZoomIn),
            b(Key::Plus, cas, ZoomIn),
            b(Key::Minus, cas, ZoomOut),
            b(Key::Num0, cas, ZoomReset),
        ]);
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rider_mac_shortcuts() {
        let k = Keymap::new(KeymapKind::RiderMac);
        assert_eq!(k.lookup(Key::D, m(false, false, false, true)), Some(Action::DuplicateLine));
        assert_eq!(k.lookup(Key::Enter, m(true, false, false, false)), Some(Action::ShowIntentions));
        assert_eq!(k.lookup(Key::B, m(false, false, false, true)), Some(Action::GotoDeclaration));
        assert_eq!(k.shortcut_text(Action::FindInFiles).as_deref(), Some("⇧⌘F"));
    }

    #[test]
    fn no_chord_is_bound_twice() {
        for kind in [KeymapKind::RiderMac, KeymapKind::Rider, KeymapKind::VisualStudio] {
            let k = Keymap::new(kind);
            for (i, (a, _)) in k.bindings.iter().enumerate() {
                for (b, act) in &k.bindings[i + 1..] {
                    assert!(
                        !(a.key == b.key && (a.mods.alt, a.mods.ctrl, a.mods.shift, a.mods.mac_cmd) == (b.mods.alt, b.mods.ctrl, b.mods.shift, b.mods.mac_cmd)),
                        "{kind:?}: {a:?} bound twice ({act:?})"
                    );
                }
            }
        }
        assert_eq!(Keymap::new(KeymapKind::Rider).shortcut_text(Action::FindInFiles).as_deref(), Some("Ctrl+Shift+F"));
    }
}
