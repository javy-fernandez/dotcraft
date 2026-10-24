//! A terminal session: a process on a pseudo-terminal (`portable-pty`) whose output feeds an
//! `alacritty_terminal::Term` on a background thread. The UI only locks the term to paint it;
//! input goes to a writer thread, so nothing here blocks the UI.

use crate::colors::TermColors;
use alacritty_terminal::event::{Event as TermEvent, EventListener, WindowSize};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::vte::ansi::{CursorShape, CursorStyle, Processor, StdSyncHandler};
use anyhow::Context as _;
use parking_lot::{Mutex, MutexGuard};
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
use rc_core::inbox::Outbox;
use rc_core::{Inbox, Waker};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// What runs in the terminal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Program {
    /// The user's shell: `$SHELL` as a login shell on macOS/Linux; `pwsh`, `powershell` or
    /// `cmd` on Windows.
    Shell,
    /// A specific command (a "Run" console).
    Command { program: String, args: Vec<String> },
}

/// How to start a session.
#[derive(Clone, Debug)]
pub struct SpawnSpec {
    pub program: Program,
    /// Working directory (the home directory when `None` or missing).
    pub cwd: Option<PathBuf>,
    /// Extra environment, on top of the IDE's and the terminal variables.
    pub env: Vec<(String, String)>,
    pub cols: u16,
    pub rows: u16,
    /// Lines of scrollback history.
    pub scrollback: usize,
    /// Print "Process finished with exit code N" when the process ends (Run consoles).
    pub announce_exit: bool,
}

impl SpawnSpec {
    pub fn shell(cwd: Option<PathBuf>) -> Self {
        SpawnSpec { program: Program::Shell, cwd, env: Vec::new(), cols: 80, rows: 24, scrollback: 10_000, announce_exit: false }
    }

    pub fn command(program: impl Into<String>, args: impl IntoIterator<Item = impl Into<String>>, cwd: Option<PathBuf>) -> Self {
        SpawnSpec {
            program: Program::Command { program: program.into(), args: args.into_iter().map(Into::into).collect() },
            announce_exit: true,
            ..SpawnSpec::shell(cwd)
        }
    }

    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }

    pub fn size(mut self, cols: u16, rows: u16) -> Self {
        self.cols = cols.max(2);
        self.rows = rows.max(1);
        self
    }
}

/// How the process ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExitInfo {
    pub code: u32,
    /// Signal that killed it (Unix), if any.
    pub signal: Option<String>,
}

impl ExitInfo {
    pub fn success(&self) -> bool {
        self.code == 0 && self.signal.is_none()
    }
}

/// Things the UI may want to react to (drained with [`TermSession::poll_events`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionEvent {
    /// The application set the title (OSC 0/2); `None` when it reset it.
    Title(Option<String>),
    Bell,
    /// The application asked to copy text to the clipboard (OSC 52).
    Clipboard(String),
    Exited(ExitInfo),
}

/// Selection kinds for [`TermSession::select`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectKind {
    Simple,
    /// Whole words (double click).
    Word,
    /// Whole lines (triple click).
    Lines,
    Block,
}

impl SelectKind {
    fn ty(self) -> SelectionType {
        match self {
            SelectKind::Simple => SelectionType::Simple,
            SelectKind::Word => SelectionType::Semantic,
            SelectKind::Lines => SelectionType::Lines,
            SelectKind::Block => SelectionType::Block,
        }
    }
}

/// Grid size in cells (implements alacritty's `Dimensions`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Size {
    cols: usize,
    rows: usize,
}

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.rows
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

/// Receives the term's events on the reader thread (opaque to users of the crate).
#[derive(Clone)]
pub struct EventProxy {
    writer: mpsc::Sender<Vec<u8>>,
    events: Outbox<SessionEvent>,
    dark: Arc<AtomicBool>,
    window: Arc<Mutex<WindowSize>>,
}

impl EventListener for EventProxy {
    fn send_event(&self, event: TermEvent) {
        match event {
            TermEvent::Title(t) => {
                self.events.send(SessionEvent::Title(Some(t)));
            }
            TermEvent::ResetTitle => {
                self.events.send(SessionEvent::Title(None));
            }
            TermEvent::Bell => {
                self.events.send(SessionEvent::Bell);
            }
            TermEvent::ClipboardStore(_, text) => {
                self.events.send(SessionEvent::Clipboard(text));
            }
            TermEvent::PtyWrite(text) => {
                let _ = self.writer.send(text.into_bytes());
            }
            TermEvent::ColorRequest(index, format) => {
                let colors = TermColors::for_theme(self.dark.load(Ordering::Relaxed));
                let _ = self.writer.send(format(colors.rgb(index)).into_bytes());
            }
            TermEvent::TextAreaSizeRequest(format) => {
                let size = *self.window.lock();
                let _ = self.writer.send(format(size).into_bytes());
            }
            _ => {}
        }
    }
}

/// The term and its parser, behind the session's lock.
pub struct TermState {
    pub term: Term<EventProxy>,
    parser: Processor<StdSyncHandler>,
    title: Option<String>,
    exit: Option<ExitInfo>,
    scrollback: usize,
    /// Bumped on every change of content (for tests and idle detection).
    pub generation: u64,
}

impl TermState {
    /// Feeds bytes as if the process printed them.
    pub fn advance(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.term, bytes);
        self.generation += 1;
    }

    /// Ends a synchronized update (`CSI ? 2026 h`) whose timeout passed; returns when the
    /// next one expires so the UI can repaint then.
    pub fn flush_sync(&mut self) -> Option<Instant> {
        let deadline = self.parser.sync_timeout().sync_timeout()?;
        if Instant::now() >= deadline {
            self.parser.stop_sync(&mut self.term);
            self.generation += 1;
            None
        } else {
            Some(deadline)
        }
    }

    pub fn mode(&self) -> TermMode {
        *self.term.mode()
    }

    /// Text of a viewport row (0 = top of what is shown), without trailing blanks.
    pub fn row_text(&self, row: usize) -> String {
        let grid = self.term.grid();
        let line = Line(row as i32 - grid.display_offset() as i32);
        line_text(&self.term, line)
    }

    /// How the process ended, once it did.
    pub fn exit(&self) -> Option<&ExitInfo> {
        self.exit.as_ref()
    }

    /// Selected text, if any.
    pub fn selection_text(&self) -> Option<String> {
        self.term.selection_to_string().filter(|s| !s.is_empty())
    }
}

fn line_text(term: &Term<EventProxy>, line: Line) -> String {
    let grid = term.grid();
    let mut s = String::new();
    for col in 0..grid.columns() {
        let cell = &grid[line][Column(col)];
        if cell.flags.intersects(alacritty_terminal::term::cell::Flags::WIDE_CHAR_SPACER | alacritty_terminal::term::cell::Flags::LEADING_WIDE_CHAR_SPACER) {
            continue;
        }
        s.push(cell.c);
        if let Some(zw) = cell.zerowidth() {
            s.extend(zw);
        }
    }
    s.truncate(s.trim_end().len());
    s
}

/// A process running on a pseudo-terminal, or an output-only terminal.
pub struct TermSession {
    id: u64,
    state: Arc<Mutex<TermState>>,
    writer: mpsc::Sender<Vec<u8>>,
    master: Option<Box<dyn MasterPty + Send>>,
    killer: Option<Box<dyn ChildKiller + Send + Sync>>,
    events: Inbox<SessionEvent>,
    dark: Arc<AtomicBool>,
    window: Arc<Mutex<WindowSize>>,
    name: String,
    cwd: Option<PathBuf>,
    pid: Option<u32>,
    exited: Arc<AtomicBool>,
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

fn term_config(scrollback: usize) -> Config {
    Config { scrolling_history: scrollback, default_cursor_style: CursorStyle { shape: CursorShape::Block, blinking: true }, ..Config::default() }
}

impl TermSession {
    /// Starts `spec` on a new pseudo-terminal. `waker` is called whenever there is new output
    /// or an event to drain.
    pub fn spawn(spec: &SpawnSpec, waker: Waker) -> anyhow::Result<Self> {
        let cols = spec.cols.max(2);
        let rows = spec.rows.max(1);
        let pair = native_pty_system().openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 }).context("openpty")?;

        let (mut cmd, name) = command_for(&spec.program);
        if let Some(cwd) = &spec.cwd {
            cmd.cwd(cwd);
        }
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        cmd.env("TERM_PROGRAM", "DotCraft");
        cmd.env("TERM_PROGRAM_VERSION", env!("CARGO_PKG_VERSION"));
        // GUI apps on macOS start without a locale: shells then print UTF-8 as escapes.
        if ["LANG", "LC_ALL", "LC_CTYPE"].iter().all(|k| std::env::var_os(k).is_none()) {
            cmd.env("LANG", "en_US.UTF-8");
        }
        for (k, v) in &spec.env {
            cmd.env(k, v);
        }

        let mut child = pair.slave.spawn_command(cmd).with_context(|| format!("spawning {name}"))?;
        // Only the child keeps the slave open, so reads end when it exits.
        drop(pair.slave);
        let pid = child.process_id();
        let killer = child.clone_killer();
        let mut reader = pair.master.try_clone_reader().context("pty reader")?;
        let mut writer = pair.master.take_writer().context("pty writer")?;

        // Writer thread: input (and the term's replies to queries) never blocks the UI.
        let (wtx, wrx) = mpsc::channel::<Vec<u8>>();
        let mut session = Self::build(cols, rows, spec.scrollback, waker.clone(), wtx, name, spec.cwd.clone());
        session.master = Some(pair.master);
        session.killer = Some(killer);
        session.pid = pid;
        std::thread::Builder::new().name("rc-term-writer".into()).spawn(move || {
            for bytes in wrx {
                if writer.write_all(&bytes).and_then(|_| writer.flush()).is_err() {
                    break;
                }
            }
        })?;

        // Reader thread: parse output into the term and wake the UI.
        let (done_tx, done_rx) = mpsc::channel::<()>();
        let state = session.state.clone();
        let wake = waker.clone();
        std::thread::Builder::new().name("rc-term-reader".into()).spawn(move || {
            let mut buf = vec![0u8; 16 * 1024];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        state.lock().advance(&buf[..n]);
                        wake.wake();
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
            let _ = done_tx.send(());
        })?;

        // Waiter thread: reports the exit once the output is drained.
        let state = session.state.clone();
        let events = session.events.sender();
        let exited = session.exited.clone();
        let announce = spec.announce_exit;
        std::thread::Builder::new().name("rc-term-wait".into()).spawn(move || {
            let status = child.wait();
            let _ = done_rx.recv_timeout(Duration::from_millis(1500));
            let info = match status {
                Ok(s) => ExitInfo { code: s.exit_code(), signal: s.signal().map(str::to_string) },
                Err(_) => ExitInfo { code: u32::MAX, signal: None },
            };
            {
                let mut st = state.lock();
                if announce {
                    let msg = rc_kit::i18n::trf("Process finished with exit code {code}", &[("code", &info.code)]);
                    st.advance(format!("\r\n\x1b[0m{msg}\r\n").as_bytes());
                }
                st.exit = Some(info.clone());
                st.generation += 1;
            }
            exited.store(true, Ordering::SeqCst);
            events.send(SessionEvent::Exited(info));
        })?;

        Ok(session)
    }

    /// A terminal without a process: text is fed with [`TermSession::feed`] (output consoles,
    /// tests). Input written to it is dropped.
    pub fn output_only(cols: u16, rows: u16, waker: Waker) -> Self {
        let (writer, _) = mpsc::channel();
        Self::build(cols.max(2), rows.max(1), 10_000, waker, writer, String::new(), None)
    }

    fn build(cols: u16, rows: u16, scrollback: usize, waker: Waker, writer: mpsc::Sender<Vec<u8>>, name: String, cwd: Option<PathBuf>) -> Self {
        let events = Inbox::new(waker);
        let dark = Arc::new(AtomicBool::new(true));
        let window = Arc::new(Mutex::new(WindowSize { num_lines: rows, num_cols: cols, cell_width: 8, cell_height: 16 }));
        let proxy = EventProxy { writer: writer.clone(), events: events.sender(), dark: dark.clone(), window: window.clone() };
        let size = Size { cols: cols as usize, rows: rows as usize };
        let term = Term::new(term_config(scrollback), &size, proxy);
        let state = TermState { term, parser: Processor::new(), title: None, exit: None, scrollback, generation: 0 };
        TermSession {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            state: Arc::new(Mutex::new(state)),
            writer,
            master: None,
            killer: None,
            events,
            dark,
            window,
            name,
            cwd,
            pid: None,
            exited: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Unique id of this session (stable for the run).
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Short name of the program (`zsh`, `pwsh`, `dotnet`…).
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The directory it was started in.
    pub fn cwd(&self) -> Option<&Path> {
        self.cwd.as_deref()
    }

    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    /// Locks the term (to paint or inspect it). Keep the guard short: output parsing waits.
    pub fn lock(&self) -> MutexGuard<'_, TermState> {
        self.state.lock()
    }

    /// Sends input bytes to the process.
    pub fn write(&self, bytes: impl Into<Vec<u8>>) {
        let bytes = bytes.into();
        if !bytes.is_empty() {
            let _ = self.writer.send(bytes);
        }
    }

    /// Pastes text (bracketed when the application asked for it).
    pub fn paste(&self, text: &str) {
        let bracketed = self.lock().mode().contains(TermMode::BRACKETED_PASTE);
        self.write(crate::keys::paste_bytes(text, bracketed));
    }

    /// Shows bytes as output without sending them to the process.
    pub fn feed(&self, bytes: &[u8]) {
        self.lock().advance(bytes);
    }

    /// Resizes the pty and the grid (no-op when unchanged). Cell size is in pixels.
    pub fn resize(&self, cols: u16, rows: u16, cell_width: u16, cell_height: u16) {
        let cols = cols.max(2);
        let rows = rows.max(1);
        {
            let mut w = self.window.lock();
            let same = w.num_cols == cols && w.num_lines == rows;
            *w = WindowSize { num_lines: rows, num_cols: cols, cell_width, cell_height };
            if same {
                return;
            }
        }
        if let Some(master) = &self.master {
            let _ = master.resize(PtySize { rows, cols, pixel_width: cols.saturating_mul(cell_width), pixel_height: rows.saturating_mul(cell_height) });
        }
        let mut st = self.lock();
        st.term.resize(Size { cols: cols as usize, rows: rows as usize });
        st.generation += 1;
    }

    /// Current size in cells (columns, rows).
    pub fn size(&self) -> (u16, u16) {
        let w = self.window.lock();
        (w.num_cols, w.num_lines)
    }

    /// Theme used to answer colour queries from applications.
    pub fn set_dark(&self, dark: bool) {
        self.dark.store(dark, Ordering::Relaxed);
    }

    /// Default cursor shape (applications can still change it with DECSCUSR).
    pub fn set_cursor_style(&self, shape: CursorShape, blinking: bool) {
        let mut st = self.lock();
        let mut config = term_config(st.scrollback);
        config.default_cursor_style = CursorStyle { shape, blinking };
        st.term.set_options(config);
    }

    /// Title set by the application (OSC 0/2), if any.
    pub fn title(&self) -> Option<String> {
        self.lock().title.clone()
    }

    /// Exit status once the process ended.
    pub fn exit_status(&self) -> Option<ExitInfo> {
        self.lock().exit.clone()
    }

    pub fn is_running(&self) -> bool {
        self.killer.is_some() && !self.exited.load(Ordering::SeqCst)
    }

    /// Events since the last call. Also keeps the title up to date: call it once per frame.
    pub fn poll_events(&self) -> Vec<SessionEvent> {
        let events = self.events.drain();
        if events.iter().any(|e| matches!(e, SessionEvent::Title(_))) {
            let mut st = self.lock();
            for e in &events {
                if let SessionEvent::Title(t) = e {
                    st.title = t.clone();
                }
            }
        }
        events
    }

    /// Terminates the process.
    pub fn kill(&self) {
        if let Some(k) = &self.killer
            && !self.exited.load(Ordering::SeqCst)
        {
            let _ = k.clone_killer().kill();
        }
    }

    /// The visible rows as text (trailing blanks trimmed).
    pub fn screen_lines(&self) -> Vec<String> {
        let st = self.lock();
        (0..st.term.screen_lines()).map(|r| st.row_text(r)).collect()
    }

    /// Selects from `start` to `end` (grid coordinates: line 0 is the top of the screen,
    /// negative lines are scrollback).
    pub fn select(&self, kind: SelectKind, start: (i32, usize), end: (i32, usize)) {
        let mut st = self.lock();
        let mut sel = Selection::new(kind.ty(), Point::new(Line(start.0), Column(start.1)), Side::Left);
        sel.update(Point::new(Line(end.0), Column(end.1)), Side::Right);
        st.term.selection = Some(sel);
    }

    /// Selects everything, scrollback included.
    pub fn select_all(&self) {
        let mut st = self.lock();
        let top = st.term.topmost_line();
        let bottom = st.term.bottommost_line();
        let last = st.term.last_column();
        let mut sel = Selection::new(SelectionType::Simple, Point::new(top, Column(0)), Side::Left);
        sel.update(Point::new(bottom, last), Side::Right);
        st.term.selection = Some(sel);
    }

    pub fn clear_selection(&self) {
        self.lock().term.selection = None;
    }

    pub fn selection_text(&self) -> Option<String> {
        self.lock().selection_text()
    }

    /// Clears the scrollback and asks the shell to redraw its prompt (Ctrl+L).
    pub fn clear(&self) {
        {
            let mut st = self.lock();
            st.term.grid_mut().clear_history();
            st.term.scroll_display(Scroll::Bottom);
            st.term.selection = None;
            st.generation += 1;
        }
        if self.is_running() {
            self.write(vec![0x0c]);
        }
    }

    /// Blocks until `pred` holds for the term or `timeout` passes (tests, headless tools).
    pub fn wait_until(&self, timeout: Duration, mut pred: impl FnMut(&TermState) -> bool) -> bool {
        let end = Instant::now() + timeout;
        loop {
            if pred(&self.lock()) {
                return true;
            }
            if Instant::now() >= end {
                return false;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Blocks until a visible row contains `text`.
    pub fn wait_for_text(&self, text: &str, timeout: Duration) -> bool {
        self.wait_until(timeout, |st| (0..st.term.screen_lines()).any(|r| st.row_text(r).contains(text)))
    }
}

impl Drop for TermSession {
    fn drop(&mut self) {
        self.kill();
    }
}

impl std::fmt::Debug for TermSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TermSession").field("id", &self.id).field("name", &self.name).field("pid", &self.pid).finish()
    }
}

/// The command line for `program` and its short name.
fn command_for(program: &Program) -> (CommandBuilder, String) {
    match program {
        Program::Shell => {
            if cfg!(windows) {
                let shell = windows_shell();
                let name = Path::new(&shell).file_stem().map_or_else(|| shell.clone(), |s| s.to_string_lossy().into_owned());
                let mut cmd = CommandBuilder::new(&shell);
                if name.eq_ignore_ascii_case("pwsh") || name.eq_ignore_ascii_case("powershell") {
                    cmd.arg("-NoLogo");
                }
                (cmd, name)
            } else {
                // `new_default_prog` runs `$SHELL` (or the passwd shell) as a login shell.
                let cmd = CommandBuilder::new_default_prog();
                let shell = std::env::var("SHELL").unwrap_or_else(|_| "sh".into());
                let name = Path::new(&shell).file_name().map_or(shell.clone(), |s| s.to_string_lossy().into_owned());
                (cmd, name)
            }
        }
        Program::Command { program, args } => {
            let mut cmd = CommandBuilder::new(program);
            cmd.args(args);
            let name = Path::new(program).file_stem().map_or_else(|| program.clone(), |s| s.to_string_lossy().into_owned());
            (cmd, name)
        }
    }
}

/// `pwsh` if installed, else Windows PowerShell, else `%COMSPEC%` (`cmd.exe`).
fn windows_shell() -> String {
    let path = std::env::var_os("PATH").unwrap_or_default();
    for exe in ["pwsh.exe", "powershell.exe"] {
        if std::env::split_paths(&path).any(|dir| dir.join(exe).is_file()) {
            return exe.to_string();
        }
    }
    std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".into())
}
