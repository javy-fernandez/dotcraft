//! Native macOS menu bar (`muda`).
//!
//! **Shortcuts:** menu items carry the accelerator of their action in the active keymap, so
//! menus show them. A menu accelerator consumes the key press before winit sees it, so egui
//! never gets that key: the command arrives through [`NativeMenu::poll`] and the app runs the
//! action exactly as if the shortcut had reached it.
//!
//! **Edit:** undo/redo/cut/copy/paste/select all are forwarded to egui as the events
//! egui-winit would have produced (`Event::Copy`, `ViewportCommand::RequestPaste`, a ⌘Z key…),
//! so the editor and every text field handle them. Call [`NativeMenu::poll`] from
//! `App::logic`, before the UI runs, so those events land in the same frame.
//!
//! **Language:** labels go through `tr`; call [`NativeMenu::relabel`] after changing it.

use rc_editor::{Action, Keymap};
use rc_kit::i18n::{tr, tr_ctx};
use muda::accelerator::{Accelerator, Code, Modifiers};
use muda::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use std::cell::{Cell, RefCell};
use std::sync::mpsc::{Receiver, channel};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuCommand {
    Act(Action),
    About,
    Quit,
    OpenProject,
    OpenFile,
    CloseProject,
    ThemeLight,
    ThemeDark,
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    SelectAll,
}

impl MenuCommand {
    fn id(self) -> String {
        format!("dotcraft.{self:?}")
    }
}

/// Native menu installed in NSApp. Keep it alive for the lifetime of the app.
pub struct NativeMenu {
    _menu: Menu,
    ctx: egui::Context,
    rx: Receiver<String>,
    commands: Vec<MenuCommand>,
    theme_light: CheckMenuItem,
    theme_dark: CheckMenuItem,
    labels: Vec<Labeled>,
    synced: Cell<Option<bool>>,
}

enum Entry {
    Item(MenuItem),
    Submenu(Submenu),
    Predefined(PredefinedMenuItem),
}

struct Labeled {
    entry: Entry,
    text: fn() -> &'static str,
}

impl Labeled {
    fn apply(&self) {
        let text = (self.text)();
        match &self.entry {
            Entry::Item(i) => i.set_text(text),
            Entry::Submenu(s) => s.set_text(text),
            Entry::Predefined(p) => p.set_text(text),
        }
    }
}

impl NativeMenu {
    /// Creates the menu and installs it in NSApp (macOS, main thread only).
    pub fn install(ctx: &egui::Context, keymap: &Keymap) -> Option<Self> {
        if !cfg!(target_os = "macos") || std::thread::current().name() != Some("main") {
            return None;
        }
        let built = match build(keymap) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("native menu: {e}");
                return None;
            }
        };
        #[cfg(target_os = "macos")]
        built.menu.init_for_nsapp();
        let (tx, rx) = channel();
        let repaint = ctx.clone();
        MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
            let _ = tx.send(e.id().as_ref().to_string());
            repaint.request_repaint();
        }));
        Some(NativeMenu {
            _menu: built.menu,
            ctx: ctx.clone(),
            rx,
            commands: built.commands,
            theme_light: built.theme_light,
            theme_dark: built.theme_dark,
            labels: built.labels,
            synced: Cell::new(None),
        })
    }

    /// Commands clicked since the last frame; edit commands are forwarded to egui instead.
    pub fn poll(&self) -> Vec<MenuCommand> {
        let mut out = Vec::new();
        for id in self.rx.try_iter() {
            self.synced.set(None);
            let Some(cmd) = self.commands.iter().copied().find(|c| c.id() == id) else { continue };
            if !forward_edit(&self.ctx, cmd) {
                out.push(cmd);
            }
        }
        out
    }

    pub fn sync(&self, dark: bool) {
        if self.synced.get() == Some(dark) {
            return;
        }
        self.theme_light.set_checked(!dark);
        self.theme_dark.set_checked(dark);
        self.synced.set(Some(dark));
    }

    pub fn relabel(&self) {
        for l in &self.labels {
            l.apply();
        }
    }
}

/// Edit commands become the egui events of their shortcut; returns whether it was one.
fn forward_edit(ctx: &egui::Context, cmd: MenuCommand) -> bool {
    use egui::{Event, Key, Modifiers as M};
    let cmd_mods = M::MAC_CMD | M::COMMAND;
    let key = |key: Key, modifiers: M| Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers };
    let event = match cmd {
        MenuCommand::Copy => Event::Copy,
        MenuCommand::Cut => Event::Cut,
        MenuCommand::Paste => {
            ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste);
            ctx.request_repaint();
            return true;
        }
        MenuCommand::Undo => key(Key::Z, cmd_mods),
        MenuCommand::Redo => key(Key::Z, cmd_mods | M::SHIFT),
        MenuCommand::SelectAll => key(Key::A, cmd_mods),
        _ => return false,
    };
    ctx.input_mut(|i| i.events.push(event));
    true
}

/// The muda code of an egui key (only keys used in menu shortcuts).
fn code_of(key: egui::Key) -> Option<Code> {
    use egui::Key as K;
    Some(match key {
        K::A => Code::KeyA,
        K::B => Code::KeyB,
        K::C => Code::KeyC,
        K::D => Code::KeyD,
        K::E => Code::KeyE,
        K::F => Code::KeyF,
        K::G => Code::KeyG,
        K::H => Code::KeyH,
        K::I => Code::KeyI,
        K::J => Code::KeyJ,
        K::K => Code::KeyK,
        K::L => Code::KeyL,
        K::M => Code::KeyM,
        K::N => Code::KeyN,
        K::O => Code::KeyO,
        K::P => Code::KeyP,
        K::Q => Code::KeyQ,
        K::R => Code::KeyR,
        K::S => Code::KeyS,
        K::T => Code::KeyT,
        K::U => Code::KeyU,
        K::V => Code::KeyV,
        K::W => Code::KeyW,
        K::X => Code::KeyX,
        K::Y => Code::KeyY,
        K::Z => Code::KeyZ,
        K::Num0 => Code::Digit0,
        K::Num1 => Code::Digit1,
        K::Num2 => Code::Digit2,
        K::Num3 => Code::Digit3,
        K::Num4 => Code::Digit4,
        K::Num5 => Code::Digit5,
        K::Num6 => Code::Digit6,
        K::Num7 => Code::Digit7,
        K::Num8 => Code::Digit8,
        K::Num9 => Code::Digit9,
        K::F1 => Code::F1,
        K::F2 => Code::F2,
        K::F3 => Code::F3,
        K::F5 => Code::F5,
        K::F6 => Code::F6,
        K::F7 => Code::F7,
        K::F8 => Code::F8,
        K::F9 => Code::F9,
        K::F10 => Code::F10,
        K::F11 => Code::F11,
        K::F12 => Code::F12,
        K::Slash => Code::Slash,
        K::Minus => Code::Minus,
        K::Equals | K::Plus => Code::Equal,
        K::Comma => Code::Comma,
        K::Period => Code::Period,
        K::OpenBracket => Code::BracketLeft,
        K::CloseBracket => Code::BracketRight,
        K::Backspace => Code::Backspace,
        K::Enter => Code::Enter,
        K::ArrowLeft => Code::ArrowLeft,
        K::ArrowRight => Code::ArrowRight,
        K::ArrowUp => Code::ArrowUp,
        K::ArrowDown => Code::ArrowDown,
        K::Space => Code::Space,
        _ => return None,
    })
}

/// The menu accelerator of `action`: only shortcuts with ⌘ or ⌃ (plain keys and ⌥ chords must
/// keep reaching the editor: arrows, Enter, Tab, ⌥⏎…), plus function keys.
fn accelerator(keymap: &Keymap, action: Action) -> Option<Accelerator> {
    let chord = keymap.chord(action)?;
    let is_fn = matches!(chord.key, egui::Key::F1 | egui::Key::F2 | egui::Key::F3 | egui::Key::F5 | egui::Key::F6 | egui::Key::F7 | egui::Key::F8 | egui::Key::F9 | egui::Key::F10 | egui::Key::F11 | egui::Key::F12);
    if !(chord.mods.mac_cmd || chord.mods.ctrl || is_fn) {
        return None;
    }
    // ⌘ + arrows / ⌘⌫ are editing keys the text fields need too.
    if matches!(chord.key, egui::Key::ArrowLeft | egui::Key::ArrowRight | egui::Key::ArrowUp | egui::Key::ArrowDown | egui::Key::Backspace) {
        return None;
    }
    let mut mods = Modifiers::empty();
    if chord.mods.mac_cmd {
        mods |= Modifiers::META;
    }
    if chord.mods.ctrl {
        mods |= Modifiers::CONTROL;
    }
    if chord.mods.alt {
        mods |= Modifiers::ALT;
    }
    if chord.mods.shift {
        mods |= Modifiers::SHIFT;
    }
    Some(Accelerator::new(mods, code_of(chord.key)?))
}

struct Built {
    menu: Menu,
    commands: Vec<MenuCommand>,
    theme_light: CheckMenuItem,
    theme_dark: CheckMenuItem,
    labels: Vec<Labeled>,
}

type Label = fn() -> &'static str;

fn build(keymap: &Keymap) -> muda::Result<Built> {
    let labels = RefCell::new(Vec::new());
    let commands = RefCell::new(Vec::new());
    let keep = |entry: Entry, text: Label| labels.borrow_mut().push(Labeled { entry, text });
    let cmd = Modifiers::META;
    let acc = |mods: Modifiers, code: Code| Some(Accelerator::new(mods, code));
    let item = |c: MenuCommand, text: Label, a: Option<Accelerator>| {
        commands.borrow_mut().push(c);
        let i = MenuItem::with_id(c.id(), text(), true, a);
        keep(Entry::Item(i.clone()), text);
        i
    };
    let act = |action: Action, text: Label| item(MenuCommand::Act(action), text, accelerator(keymap, action));
    let predefined = |p: PredefinedMenuItem, text: Label| {
        keep(Entry::Predefined(p.clone()), text);
        p
    };
    let submenu = |text: Label, items: &[&dyn muda::IsMenuItem]| -> muda::Result<Submenu> {
        let s = Submenu::with_items(text(), true, items)?;
        keep(Entry::Submenu(s.clone()), text);
        Ok(s)
    };
    let sep = PredefinedMenuItem::separator;

    let app_menu = Submenu::with_items(
        "DotCraft",
        true,
        &[
            &item(MenuCommand::About, || tr("About DotCraft"), None),
            &sep(),
            &act(Action::ShowSettings, || tr("Settings…")),
            &sep(),
            &predefined(PredefinedMenuItem::services(Some(tr("Services"))), || tr("Services")),
            &sep(),
            &predefined(PredefinedMenuItem::hide(Some(tr("Hide DotCraft"))), || tr("Hide DotCraft")),
            &predefined(PredefinedMenuItem::hide_others(Some(tr("Hide Others"))), || tr("Hide Others")),
            &predefined(PredefinedMenuItem::show_all(Some(tr("Show All"))), || tr("Show All")),
            &sep(),
            &item(MenuCommand::Quit, || tr("Quit DotCraft"), acc(cmd, Code::KeyQ)),
        ],
    )?;
    let file_menu = submenu(
        || tr("File"),
        &[
            &item(MenuCommand::OpenProject, || tr("Open…"), None),
            &item(MenuCommand::OpenFile, || tr("Open File…"), None),
            &item(MenuCommand::CloseProject, || tr("Close Project"), None),
            &sep(),
            &act(Action::SaveAll, || tr("Save All")),
            &act(Action::CloseTab, || tr("Close Tab")),
        ],
    )?;
    let edit_menu = submenu(
        || tr_ctx("menu", "Edit"),
        &[
            &item(MenuCommand::Undo, || tr("Undo"), acc(cmd, Code::KeyZ)),
            &item(MenuCommand::Redo, || tr("Redo"), acc(cmd | Modifiers::SHIFT, Code::KeyZ)),
            &sep(),
            &item(MenuCommand::Cut, || tr("Cut"), acc(cmd, Code::KeyX)),
            &item(MenuCommand::Copy, || tr("Copy"), acc(cmd, Code::KeyC)),
            &item(MenuCommand::Paste, || tr("Paste"), acc(cmd, Code::KeyV)),
            &item(MenuCommand::SelectAll, || tr("Select All"), acc(cmd, Code::KeyA)),
            &sep(),
            &act(Action::Find, || tr("Find…")),
            &act(Action::Replace, || tr("Replace…")),
            &act(Action::FindInFiles, || tr("Find in Files…")),
            &act(Action::ReplaceInFiles, || tr("Replace in Files…")),
            &sep(),
            &act(Action::DuplicateLine, || tr("Duplicate Line or Selection")),
            &act(Action::CommentLine, || tr("Comment with Line Comment")),
            &act(Action::CommentBlock, || tr("Comment with Block Comment")),
            &act(Action::SelectNextOccurrence, || tr("Add Selection for Next Occurrence")),
            &act(Action::SelectAllOccurrences, || tr("Select All Occurrences")),
            &act(Action::JoinLines, || tr("Join Lines")),
            &act(Action::ToggleCase, || tr("Toggle Case")),
        ],
    )?;
    let theme_light = CheckMenuItem::with_id(MenuCommand::ThemeLight.id(), "Islands Light", true, false, None);
    let theme_dark = CheckMenuItem::with_id(MenuCommand::ThemeDark.id(), "Islands Dark", true, false, None);
    commands.borrow_mut().extend([MenuCommand::ThemeLight, MenuCommand::ThemeDark]);
    let theme_menu = submenu(|| tr("Theme"), &[&theme_light, &theme_dark])?;
    let tool_windows = submenu(
        || tr("Tool Windows"),
        &[
            &act(Action::ToggleProjectWindow, || tr("Explorer")),
            &act(Action::ToggleTerminal, || tr("Terminal")),
            &act(Action::ToggleProblems, || tr("Problems")),
            &act(Action::ToggleGitWindow, || tr("Git")),
            &sep(),
            &act(Action::HideToolWindows, || tr("Hide All Tool Windows")),
        ],
    )?;
    let view_menu = submenu(
        || tr("View"),
        &[
            &tool_windows,
            &theme_menu,
            &sep(),
            &act(Action::ZoomIn, || tr("Zoom In")),
            &act(Action::ZoomOut, || tr("Zoom Out")),
            &act(Action::ZoomReset, || tr("Reset Zoom")),
            &sep(),
            &act(Action::Fold, || tr("Collapse")),
            &act(Action::Unfold, || tr("Expand")),
            &act(Action::FoldAll, || tr("Collapse All")),
            &act(Action::UnfoldAll, || tr("Expand All")),
        ],
    )?;
    let navigate_menu = submenu(
        || tr("Navigate"),
        &[
            &act(Action::SearchEverywhere, || tr("Search Everywhere")),
            &act(Action::GotoFile, || tr("Go to File…")),
            &act(Action::GotoClass, || tr("Go to Type…")),
            &act(Action::GotoSymbol, || tr("Go to Symbol…")),
            &act(Action::GotoLine, || tr("Go to Line…")),
            &sep(),
            &act(Action::NavigateBack, || tr("Back")),
            &act(Action::NavigateForward, || tr("Forward")),
            &act(Action::RecentFiles, || tr("Recent Files")),
            &act(Action::RecentLocations, || tr("Recent Locations")),
            &sep(),
            &act(Action::GotoDeclaration, || tr("Go to Declaration")),
            &act(Action::GotoImplementation, || tr("Go to Implementation")),
            &act(Action::GotoTypeDeclaration, || tr("Go to Type Declaration")),
            &act(Action::FindUsages, || tr("Find Usages")),
            &act(Action::FileStructure, || tr("File Structure")),
            &sep(),
            &act(Action::NextError, || tr("Next Highlighted Error")),
            &act(Action::PreviousError, || tr("Previous Highlighted Error")),
        ],
    )?;
    let code_menu = submenu(
        || tr("Code"),
        &[
            &act(Action::Complete, || tr("Code Completion")),
            &act(Action::ParameterInfo, || tr("Parameter Info")),
            &act(Action::QuickDoc, || tr("Quick Documentation")),
            &act(Action::ShowIntentions, || tr("Show Context Actions")),
            &sep(),
            &act(Action::Rename, || tr("Rename…")),
            &act(Action::Reformat, || tr("Reformat Code")),
        ],
    )?;
    let build_menu = submenu(|| tr("Build"), &[&act(Action::Build, || tr("Build Solution"))])?;
    let run_menu = submenu(
        || tr("Run"),
        &[
            &act(Action::Run, || tr("Run")),
            &act(Action::Debug, || tr("Debug")),
            &act(Action::StopRun, || tr("Stop")),
            &act(Action::RunTests, || tr("Run Tests")),
            &sep(),
            &act(Action::StepOver, || tr("Step Over")),
            &act(Action::StepInto, || tr("Step Into")),
            &act(Action::StepOut, || tr("Step Out")),
            &act(Action::RunToCursor, || tr("Run to Cursor")),
            &act(Action::Resume, || tr("Resume Program")),
            &act(Action::Evaluate, || tr("Evaluate Expression…")),
            &sep(),
            &act(Action::ToggleBreakpoint, || tr("Toggle Line Breakpoint")),
        ],
    )?;
    let git_menu = submenu(|| tr("Git"), &[&act(Action::Commit, || tr("Commit…"))])?;
    let window_menu = submenu(
        || tr("Window"),
        &[
            &predefined(PredefinedMenuItem::minimize(Some(tr("Minimize"))), || tr("Minimize")),
            &PredefinedMenuItem::maximize(Some(tr("Zoom"))),
            &sep(),
            &predefined(PredefinedMenuItem::bring_all_to_front(Some(tr("Bring All to Front"))), || tr("Bring All to Front")),
        ],
    )?;
    #[cfg(target_os = "macos")]
    window_menu.set_as_windows_menu_for_nsapp();
    let help_menu = submenu(|| tr("Help"), &[&item(MenuCommand::About, || tr("About DotCraft"), None)])?;
    #[cfg(target_os = "macos")]
    help_menu.set_as_help_menu_for_nsapp();

    let menu = Menu::with_items(&[
        &app_menu,
        &file_menu,
        &edit_menu,
        &view_menu,
        &navigate_menu,
        &code_menu,
        &build_menu,
        &run_menu,
        &git_menu,
        &window_menu,
        &help_menu,
    ])?;
    Ok(Built { menu, commands: commands.into_inner(), theme_light, theme_dark, labels: labels.into_inner() })
}
