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
    use Key::*;
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
        b(ArrowLeft, none, Left(false)),
        b(ArrowLeft, s, Left(true)),
        b(ArrowRight, none, Right(false)),
        b(ArrowRight, s, Right(true)),
        b(ArrowUp, none, Up(false)),
        b(ArrowUp, s, Up(true)),
        b(ArrowDown, none, Down(false)),
        b(ArrowDown, s, Down(true)),
        b(ArrowLeft, a, WordLeft(false)),
        b(ArrowLeft, as_, WordLeft(true)),
        b(ArrowRight, a, WordRight(false)),
        b(ArrowRight, as_, WordRight(true)),
        b(ArrowLeft, c, LineStart(false)),
        b(ArrowLeft, cs, LineStart(true)),
        b(ArrowRight, c, LineEnd(false)),
        b(ArrowRight, cs, LineEnd(true)),
        b(Home, none, LineStart(false)),
        b(Home, s, LineStart(true)),
        b(End, none, LineEnd(false)),
        b(End, s, LineEnd(true)),
        b(Home, c, DocStart(false)),
        b(Home, cs, DocStart(true)),
        b(End, c, DocEnd(false)),
        b(End, cs, DocEnd(true)),
        b(ArrowUp, c, DocStart(false)),
        b(ArrowUp, cs, DocStart(true)),
        b(ArrowDown, c, DocEnd(false)),
        b(ArrowDown, cs, DocEnd(true)),
        b(PageUp, none, PageUp(false)),
        b(PageUp, s, PageUp(true)),
        b(PageDown, none, PageDown(false)),
        b(PageDown, s, PageDown(true)),
        b(Enter, none, Enter),
        b(Enter, s, StartNewLine),
        b(Enter, ca, StartNewLineBefore),
        b(Backspace, none, Backspace),
        b(Backspace, s, Backspace),
        b(Delete, none, Delete),
        b(Backspace, a, DeleteWordLeft),
        b(Delete, a, DeleteWordRight),
        b(Backspace, c, DeleteLine),
        b(D, c, DuplicateLine),
        b(ArrowUp, as_, MoveLineUp),
        b(ArrowDown, as_, MoveLineDown),
        b(Slash, c, CommentLine),
        b(Slash, ca, CommentBlock),
        b(Tab, none, Indent),
        b(Tab, s, Unindent),
        b(J, ctrl_s, JoinLines),
        b(U, cs, ToggleCase),
        b(Z, c, Undo),
        b(Z, cs, Redo),
        b(C, c, Copy),
        b(X, c, Cut),
        b(V, c, Paste),
        b(A, c, SelectAll),
        b(ArrowUp, a, ExtendSelection),
        b(ArrowDown, a, ShrinkSelection),
        b(G, ctrl, SelectNextOccurrence),
        b(G, ctrl_s, UnselectOccurrence),
        b(G, ctrl_c, SelectAllOccurrences),
        b(Escape, none, Escape),
        b(Minus, c, Fold),
        b(Equals, c, Unfold),
        b(Plus, c, Unfold),
        b(Minus, cs, FoldAll),
        b(Equals, cs, UnfoldAll),
        b(Plus, cs, UnfoldAll),
        b(Space, ctrl, Complete),
        b(P, c, ParameterInfo),
        b(F1, none, QuickDoc),
        b(J, ctrl, QuickDoc),
        b(Enter, a, ShowIntentions),
        b(B, c, GotoDeclaration),
        b(B, ca, GotoImplementation),
        b(B, cs, GotoTypeDeclaration),
        b(F7, a, FindUsages),
        b(F6, s, Rename),
        b(L, ca, Reformat),
        b(F12, c, FileStructure),
        b(L, c, GotoLine),
        b(F, c, Find),
        b(R, c, Replace),
        b(G, c, FindNext),
        b(G, cs, FindPrevious),
        b(F3, none, FindNext),
        b(F3, s, FindPrevious),
        b(F2, none, NextError),
        b(F2, s, PreviousError),
        b(F8, c, ToggleBreakpoint),
        b(O, cs, GotoFile),
        b(O, cas, GotoSymbol),
        b(O, c, GotoClass),
        b(T, c, SearchEverywhere),
        b(F, cs, FindInFiles),
        b(R, cs, ReplaceInFiles),
        b(E, c, RecentFiles),
        b(E, cs, RecentLocations),
        b(OpenBracket, c, NavigateBack),
        b(CloseBracket, c, NavigateForward),
        b(ArrowLeft, ca, NavigateBack),
        b(ArrowRight, ca, NavigateForward),
        b(S, c, SaveAll),
        b(W, c, CloseTab),
        b(CloseBracket, cs, NextTab),
        b(OpenBracket, cs, PreviousTab),
        b(F9, c, Build),
        b(R, ctrl, Run),
        b(D, ctrl, Debug),
        b(F2, c, StopRun),
        b(F8, none, StepOver),
        b(F7, none, StepInto),
        b(F8, s, StepOut),
        b(R, ca, Resume),
        b(F9, a, RunToCursor),
        b(F8, a, Evaluate),
        b(Comma, c, ShowSettings),
        b(Num1, c, ToggleProjectWindow),
        b(F12, a, ToggleTerminal),
        b(Num9, c, ToggleGitWindow),
        b(Num6, c, ToggleProblems),
        b(F12, cs, HideToolWindows),
        b(K, c, Commit),
        b(O, ca, OpenFolder),
        b(Equals, ca, ZoomIn),
        b(Plus, ca, ZoomIn),
        b(Minus, ca, ZoomOut),
        b(Num0, ca, ZoomReset),
    ]
}

fn win_bindings(visual_studio: bool) -> Vec<(Chord, Action)> {
    use Action::*;
    use Key::*;
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
        b(ArrowLeft, none, Left(false)),
        b(ArrowLeft, s, Left(true)),
        b(ArrowRight, none, Right(false)),
        b(ArrowRight, s, Right(true)),
        b(ArrowUp, none, Up(false)),
        b(ArrowUp, s, Up(true)),
        b(ArrowDown, none, Down(false)),
        b(ArrowDown, s, Down(true)),
        b(ArrowLeft, c, WordLeft(false)),
        b(ArrowLeft, cs, WordLeft(true)),
        b(ArrowRight, c, WordRight(false)),
        b(ArrowRight, cs, WordRight(true)),
        b(Home, none, LineStart(false)),
        b(Home, s, LineStart(true)),
        b(End, none, LineEnd(false)),
        b(End, s, LineEnd(true)),
        b(Home, c, DocStart(false)),
        b(Home, cs, DocStart(true)),
        b(End, c, DocEnd(false)),
        b(End, cs, DocEnd(true)),
        b(PageUp, none, PageUp(false)),
        b(PageUp, s, PageUp(true)),
        b(PageDown, none, PageDown(false)),
        b(PageDown, s, PageDown(true)),
        b(ArrowUp, c, ScrollUp),
        b(ArrowDown, c, ScrollDown),
        b(Enter, none, Enter),
        b(Enter, s, StartNewLine),
        b(Enter, ca, StartNewLineBefore),
        b(Backspace, none, Backspace),
        b(Backspace, s, Backspace),
        b(Delete, none, Delete),
        b(Backspace, c, DeleteWordLeft),
        b(Delete, c, DeleteWordRight),
        b(Tab, none, Indent),
        b(Tab, s, Unindent),
        b(J, cs, JoinLines),
        b(U, cs, ToggleCase),
        b(Z, c, Undo),
        b(Z, cs, Redo),
        b(C, c, Copy),
        b(X, c, Cut),
        b(V, c, Paste),
        b(Insert, c, Copy),
        b(Insert, s, Paste),
        b(Delete, s, Cut),
        b(A, c, SelectAll),
        b(Escape, none, Escape),
        b(Space, c, Complete),
        b(P, c, ParameterInfo),
        b(Enter, a, ShowIntentions),
        b(F12, none, GotoDeclaration),
        b(F12, c, GotoImplementation),
        b(F7, a, FindUsages),
        b(G, c, GotoLine),
        b(F, c, Find),
        b(H, c, Replace),
        b(F3, none, FindNext),
        b(F3, s, FindPrevious),
        b(F2, none, NextError),
        b(F2, s, PreviousError),
        b(F9, none, ToggleBreakpoint),
        b(F, cs, FindInFiles),
        b(H, cs, ReplaceInFiles),
        b(Minus, ca, NavigateBack),
        b(S, c, Save),
        b(S, cs, SaveAll),
        b(F4, c, CloseTab),
        b(Tab, c, NextTab),
        b(Tab, cs, PreviousTab),
        b(F5, none, Debug),
        b(F5, c, Run),
        b(F5, s, StopRun),
        b(F10, none, StepOver),
        b(F11, none, StepInto),
        b(F11, s, StepOut),
        b(F10, c, RunToCursor),
        b(F9, s, Evaluate),
        b(Equals, c, ZoomIn),
        b(Plus, c, ZoomIn),
        b(Minus, c, ZoomOut),
        b(Num0, c, ZoomReset),
        b(OpenBracket, cas, FoldAll),
    ];
    if visual_studio {
        v.extend([
            b(L, cs, DeleteLine),
            b(D, c, DuplicateLine),
            b(ArrowUp, a, MoveLineUp),
            b(ArrowDown, a, MoveLineDown),
            b(Slash, c, CommentLine),
            b(W, c, ExtendSelection),
            b(W, cs, ShrinkSelection),
            b(Period, cs, SelectNextOccurrence),
            b(Comma, cs, UnselectOccurrence),
            b(Semicolon, cs, SelectAllOccurrences),
            b(R, c, Rename),
            b(R, ca, Rename),
            b(K, ca, Reformat),
            b(T, c, SearchEverywhere),
            b(T, cs, GotoFile),
            b(Comma, c, SearchEverywhere),
            b(Q, c, QuickDoc),
            b(Tab, ca, RecentFiles),
            b(B, cs, Build),
            b(Comma, ca, ShowSettings),
            b(L, ca, ToggleProjectWindow),
            b(Backtick, c, ToggleTerminal),
            b(Num0, ca, ToggleGitWindow),
            b(Minus, c, Fold),
            b(Plus, c, Unfold),
        ]);
    } else {
        v.extend([
            b(Y, c, DeleteLine),
            b(D, c, DuplicateLine),
            b(ArrowUp, as_, MoveLineUp),
            b(ArrowDown, as_, MoveLineDown),
            b(Slash, c, CommentLine),
            b(Slash, cs, CommentBlock),
            b(W, c, ExtendSelection),
            b(W, cs, ShrinkSelection),
            b(J, a, SelectNextOccurrence),
            b(J, as_, UnselectOccurrence),
            b(J, cas, SelectAllOccurrences),
            b(Minus, c, Fold),
            b(Equals, c, Unfold),
            b(Plus, c, Unfold),
            b(Minus, cs, FoldAll),
            b(Equals, cs, UnfoldAll),
            b(Q, c, QuickDoc),
            b(B, c, GotoDeclaration),
            b(B, ca, GotoImplementation),
            b(B, cs, GotoTypeDeclaration),
            b(F6, s, Rename),
            b(R, cs, Rename),
            b(L, ca, Reformat),
            b(F12, c, FileStructure),
            b(R, c, Replace),
            b(N, cs, GotoFile),
            b(N, cas, GotoSymbol),
            b(N, c, GotoClass),
            b(T, c, SearchEverywhere),
            b(E, c, RecentFiles),
            b(E, cs, RecentLocations),
            b(ArrowLeft, ca, NavigateBack),
            b(ArrowRight, ca, NavigateForward),
            b(F9, cs, Build),
            b(F5, a, Resume),
            b(F8, a, Evaluate),
            b(S, ca, ShowSettings),
            b(Num1, a, ToggleProjectWindow),
            b(F12, a, ToggleTerminal),
            b(Num9, a, ToggleGitWindow),
            b(Num6, a, ToggleProblems),
            b(F12, cs, HideToolWindows),
            b(K, c, Commit),
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
