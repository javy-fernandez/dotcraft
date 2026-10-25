//! The debugger in the IDE: starting a session for the selected run configuration (with
//! netcoredbg, downloaded on first use), breakpoints kept in sync with the editors, the
//! execution point, the debug console, watches, hover values and expression evaluation.

use crate::docs::Docs;
use crate::runner::{ConfigKind, RunConfig};
use rc_core::{Inbox, Waker};
use rc_dap::{BreakpointSpec, Breakpoints, DebugEvent, DebugOptions, DebugSession, DebugTarget, DownloadEvent, EvalContext, EvalValue, OutputCategory, SessionState};
use rc_kit::i18n::{tr, trf};
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

/// What the debugger asks the rest of the UI to do.
pub enum DebugOutput {
    Notify(String, bool),
    /// Show the execution point: open the file at this 0-based line.
    ShowLine(PathBuf, usize),
    ShowDebugWindow,
}

/// An Evaluate request: id, expression and its result once it arrives.
pub type Evaluation = (u64, String, Option<Result<EvalValue, String>>);

pub struct Debugger {
    pub session: Option<DebugSession>,
    pub console: Option<crate::runner::Console>,
    pub name: String,
    downloads: Inbox<DownloadEvent>,
    download: Option<rc_dap::netcoredbg::DownloadHandle>,
    pub downloading: Option<(u64, Option<u64>)>,
    pending: Option<(RunConfig, rc_dotnet::RunRequest, PathBuf)>,
    waker: Waker,
    /// Expanded rows of the Variables tree, by path of names (kept across steps).
    pub expanded: BTreeSet<String>,
    pub new_watch: String,
    /// Evaluate dialog: (request id, expression, result).
    pub eval: Option<Evaluation>,
    /// Hover evaluation: (request id, doc, pos, screen, expression).
    hover: Option<(u64, u64, usize, egui::Pos2, String)>,
    pub hover_result: Option<(u64, usize, egui::Pos2, String)>,
    /// Breakpoint conditions by (file, 0-based line).
    pub conditions: HashMap<(PathBuf, usize), String>,
    pub show_console: bool,
    /// Breakpoint set by Run to Cursor, removed at the next stop (file, 1-based line).
    temp: Option<(PathBuf, u32)>,
    /// URLs the debugged web app listens on.
    pub urls: Vec<String>,
    /// Extra environment for the debugged program (tests pick free ports with it).
    pub env: Vec<(String, String)>,
}

impl Debugger {
    pub fn new(waker: Waker) -> Self {
        Debugger {
            session: None,
            console: None,
            name: String::new(),
            downloads: Inbox::new(waker.clone()),
            download: None,
            downloading: None,
            pending: None,
            waker,
            expanded: BTreeSet::new(),
            new_watch: String::new(),
            eval: None,
            hover: None,
            hover_result: None,
            conditions: HashMap::new(),
            show_console: false,
            temp: None,
            urls: Vec::new(),
            env: Vec::new(),
        }
    }

    pub fn active(&self) -> bool {
        self.session.as_ref().is_some_and(|s| !s.state().is_ended())
    }

    pub fn stopped(&self) -> bool {
        self.session.as_ref().is_some_and(|s| s.state().is_stopped())
    }

    /// Breakpoints of every open editor (1-based lines for the adapter).
    fn breakpoints(&self, docs: &Docs) -> Breakpoints {
        let mut bps = Breakpoints::new();
        for d in &docs.list {
            for (&line, kind) in &d.editor.breakpoints {
                let mut spec = BreakpointSpec::new(d.path.clone(), line as u32 + 1);
                if let Some(c) = self.conditions.get(&(d.path.clone(), line)) {
                    spec = spec.with_condition(c.clone());
                }
                spec.enabled = *kind != rc_editor::BreakpointKind::Disabled;
                bps.set(spec);
            }
        }
        bps
    }

    /// Starts debugging `config` (apps only); downloads netcoredbg first when needed.
    pub fn start(&mut self, config: RunConfig, request: Option<rc_dotnet::RunRequest>, dotnet: &Path, configured: Option<&Path>, docs: &Docs) -> Vec<DebugOutput> {
        let (ConfigKind::App { .. }, Some(mut run)) = (&config.kind, request) else {
            return vec![DebugOutput::Notify(tr("Choose an application run configuration to debug").into(), false)];
        };
        if let Some(s) = &mut self.session {
            s.stop();
        }
        let Some(adapter) = rc_dap::netcoredbg::locate(configured) else {
            if self.download.is_none() {
                self.downloading = Some((0, None));
                self.download = Some(rc_dap::netcoredbg::start_download(self.downloads.sender()));
            }
            self.pending = Some((config.clone(), run, dotnet.to_path_buf()));
            return vec![DebugOutput::Notify(tr("Downloading the netcoredbg debugger (first use)…").into(), false)];
        };
        let Some(dotnet) = rc_dotnet::Dotnet::locate(Some(dotnet)) else {
            return vec![DebugOutput::Notify(tr("The .NET SDK was not found. Install it or set its path in Settings › Tools.").into(), true)];
        };
        run.env.extend(self.env.iter().cloned());
        self.urls.clear();
        let target = DebugTarget::Project { dotnet, run };
        let console = crate::runner::Console::new(&self.waker);
        console.note(&trf("Building {name}…", &[("name", &config.name)]));
        self.console = Some(console);
        self.name = config.name.clone();
        self.expanded.clear();
        let watches: Vec<String> = self.session.as_ref().map(|s| s.watches().iter().map(|w| w.expression.clone()).collect()).unwrap_or_default();
        let mut session = DebugSession::start(adapter, target, DebugOptions::default(), self.breakpoints(docs), self.waker.clone());
        for w in watches {
            session.add_watch(w);
        }
        self.session = Some(session);
        vec![DebugOutput::ShowDebugWindow]
    }

    pub fn stop(&mut self) {
        if let Some(s) = &mut self.session {
            s.stop();
        }
    }

    /// Keeps the session's breakpoints equal to the editors' (toggled in the gutter, edited
    /// conditions, lines moved by edits).
    fn sync_breakpoints(&mut self, docs: &Docs) {
        let Some(s) = &mut self.session else { return };
        if s.state().is_ended() {
            return;
        }
        for d in &docs.list {
            let current: Vec<(u32, Option<String>, bool)> = s.breakpoints().in_file(&d.path).iter().map(|b| (b.spec.line, b.spec.condition.clone(), b.spec.enabled)).collect();
            let wanted: Vec<(u32, Option<String>, bool)> = d
                .editor
                .breakpoints
                .iter()
                .map(|(&l, k)| (l as u32 + 1, self.conditions.get(&(d.path.clone(), l)).cloned(), *k != rc_editor::BreakpointKind::Disabled))
                .chain(self.temp.iter().filter(|(p, _)| *p == d.path).map(|(_, l)| (*l, None, true)))
                .collect();
            let same = current.len() == wanted.len() && wanted.iter().all(|w| current.contains(w));
            if !same {
                let specs = wanted
                    .into_iter()
                    .map(|(line, cond, enabled)| {
                        let mut spec = BreakpointSpec::new(d.path.clone(), line);
                        spec.condition = cond;
                        spec.enabled = enabled;
                        spec
                    })
                    .collect();
                s.set_file_breakpoints(&d.path, specs);
            }
        }
    }

    /// Continues to `line` (0-based) of `path` with a one-shot breakpoint.
    pub fn run_to_cursor(&mut self, path: &Path, line: usize) {
        if !self.stopped() {
            return;
        }
        self.temp = Some((path.to_path_buf(), line as u32 + 1));
        if let Some(s) = &mut self.session {
            s.set_breakpoint(BreakpointSpec::new(path.to_path_buf(), line as u32 + 1));
            s.continue_();
        }
    }

    /// Evaluates the expression under the mouse while stopped (hover values).
    pub fn hover(&mut self, doc: u64, pos: usize, screen: egui::Pos2, expression: String) {
        if expression.is_empty() || !self.stopped() {
            return;
        }
        if let Some(s) = &mut self.session {
            let id = s.evaluate(&expression, EvalContext::Hover);
            self.hover = Some((id, doc, pos, screen, expression));
        }
    }

    pub fn evaluate(&mut self, expression: String) {
        if let Some(s) = &mut self.session {
            let id = s.evaluate(&expression, EvalContext::Repl);
            self.eval = Some((id, expression, None));
        }
    }

    pub fn update(&mut self, docs: &mut Docs) -> Vec<DebugOutput> {
        let mut out = Vec::new();
        for ev in self.downloads.drain() {
            match ev {
                DownloadEvent::Progress { done, total } => self.downloading = Some((done, total)),
                DownloadEvent::Unpacking => {}
                DownloadEvent::Done(_) => {
                    self.downloading = None;
                    self.download = None;
                    out.push(DebugOutput::Notify(tr("netcoredbg is ready").into(), false));
                    if let Some((cfg, run, dotnet)) = self.pending.take() {
                        out.extend(self.start(cfg, Some(run), &dotnet, None, docs));
                    }
                }
                DownloadEvent::Failed(e) => {
                    self.downloading = None;
                    self.download = None;
                    self.pending = None;
                    out.push(DebugOutput::Notify(trf("Could not download netcoredbg: {error}", &[("error", &e)]), true));
                }
            }
        }
        self.sync_breakpoints(docs);
        let temp = self.temp.clone();
        let Some(session) = &mut self.session else { return out };
        let events = session.pump();
        for ev in events {
            match ev {
                DebugEvent::Build(rc_dotnet::BuildEvent::Output(line)) => {
                    if let Some(c) = &self.console {
                        c.line(&line);
                    }
                }
                DebugEvent::Build(_) => {}
                DebugEvent::Prepared(plan) => {
                    self.urls = plan.urls.clone();
                    if let Some(c) = &self.console {
                        c.note(&plan.command_line());
                        if !plan.urls.is_empty() {
                            c.note(&trf("Listening on {urls}", &[("urls", &plan.urls.join(", "))]));
                        }
                    }
                }
                DebugEvent::Output { category, text } => {
                    if let Some(c) = &self.console {
                        let stream = if category == OutputCategory::Stderr { rc_dotnet::Stream::Stderr } else { rc_dotnet::Stream::Stdout };
                        let console_msg = matches!(category, OutputCategory::Console | OutputCategory::Important);
                        if console_msg {
                            c.note(text.trim_end());
                        } else {
                            c.raw(&text, stream);
                        }
                    }
                }
                DebugEvent::Stopped(info) => {
                    if let Some((p, l)) = &temp {
                        let user = docs.list.iter().any(|d| d.path == *p && d.editor.breakpoints.contains_key(&(*l as usize - 1)));
                        if !user {
                            session.remove_breakpoint(p, *l);
                        }
                        self.temp = None;
                    }
                    if let Some((path, line)) = session.execution_point() {
                        out.push(DebugOutput::ShowLine(path, line.saturating_sub(1) as usize));
                    }
                    if info.reason == rc_dap::StopReason::Exception {
                        out.push(DebugOutput::Notify(info.description.clone().or(info.text.clone()).unwrap_or_else(|| tr("Exception").into()), true));
                    }
                    out.push(DebugOutput::ShowDebugWindow);
                }
                DebugEvent::FrameSelected { .. } => {
                    if let Some((path, line)) = session.execution_point() {
                        out.push(DebugOutput::ShowLine(path, line.saturating_sub(1) as usize));
                    }
                }
                DebugEvent::Continued => {
                    self.hover_result = None;
                }
                DebugEvent::Exited(code) => {
                    if let Some(c) = &self.console {
                        c.note(&trf("Process finished with exit code {code}", &[("code", &code)]));
                    }
                }
                DebugEvent::Terminated => {
                    if let Some(c) = &self.console {
                        c.note(tr("Debugger disconnected"));
                    }
                }
                DebugEvent::Failed(f) => {
                    let (tpl, args) = f.user_message();
                    let args: Vec<(&str, &dyn std::fmt::Display)> = args.iter().map(|(k, v)| (*k, v as &dyn std::fmt::Display)).collect();
                    let msg = trf(tpl, &args);
                    if let Some(c) = &self.console {
                        c.note(&msg);
                    }
                    out.push(DebugOutput::Notify(msg, true));
                }
                DebugEvent::Evaluated { id, result, .. } => {
                    if let Some((eid, _, slot)) = &mut self.eval
                        && *eid == id
                    {
                        *slot = Some(result.clone().map_err(|e| e.to_string()));
                    }
                    if let Some((hid, doc, pos, screen, expr)) = &self.hover
                        && *hid == id
                    {
                        if let Ok(v) = &result {
                            let text = match &v.type_name {
                                Some(t) if !t.is_empty() => format!("{expr} = {}  ({t})", v.result),
                                _ => format!("{expr} = {}", v.result),
                            };
                            self.hover_result = Some((*doc, *pos, *screen, text));
                        }
                        self.hover = None;
                    }
                }
                _ => {}
            }
        }
        // Execution point and breakpoint state in the editors.
        let exec = if session.state().is_stopped() { session.execution_point() } else { None };
        for d in &mut docs.list {
            d.editor.execution_line = exec.as_ref().filter(|(p, _)| *p == d.path).map(|(_, l)| l.saturating_sub(1) as usize);
            for st in session.breakpoints().in_file(&d.path) {
                let line = st.spec.line.saturating_sub(1) as usize;
                if temp.as_ref().is_some_and(|(p, l)| *p == d.path && *l == st.spec.line) && !d.editor.breakpoints.contains_key(&line) {
                    continue;
                }
                if let Some(kind) = d.editor.breakpoints.get_mut(&line) {
                    *kind = if !st.spec.enabled {
                        rc_editor::BreakpointKind::Disabled
                    } else if st.spec.condition.is_some() {
                        rc_editor::BreakpointKind::Conditional
                    } else if st.verified {
                        rc_editor::BreakpointKind::Verified
                    } else if session.state().is_ended() {
                        rc_editor::BreakpointKind::Normal
                    } else {
                        rc_editor::BreakpointKind::Invalid
                    };
                }
            }
        }
        if session.state().is_ended() {
            for d in &mut docs.list {
                d.editor.execution_line = None;
                for k in d.editor.breakpoints.values_mut() {
                    if matches!(k, rc_editor::BreakpointKind::Verified | rc_editor::BreakpointKind::Invalid) {
                        *k = rc_editor::BreakpointKind::Normal;
                    }
                }
            }
        }
        out
    }

    pub fn state_label(&self) -> String {
        match self.session.as_ref().map(|s| s.state()) {
            None => String::new(),
            Some(SessionState::Preparing) => tr("Building…").into(),
            Some(SessionState::Starting) => tr("Starting…").into(),
            Some(SessionState::Running) => tr("Running").into(),
            Some(SessionState::Stopped(info)) => tr(info.reason.label()).into(),
            Some(SessionState::Exited(c)) => trf("Exited with code {code}", &[("code", c)]),
            Some(SessionState::Terminated) => tr("Disconnected").into(),
            Some(SessionState::Failed(f)) => {
                let (tpl, args) = f.user_message();
                let args: Vec<(&str, &dyn std::fmt::Display)> = args.iter().map(|(k, v)| (*k, v as &dyn std::fmt::Display)).collect();
                trf(tpl, &args)
            }
        }
    }
}

/// The expression under `pos` for hover values: the identifier with its member-access chain
/// (`order.Customer`), without calls.
pub fn expression_at(rope: &rc_text::Rope, pos: usize) -> String {
    let (s, e) = rc_text::commands::word_at(rope, pos);
    if s == e {
        return String::new();
    }
    let mut start = s;
    while start >= 2 && rope.char(start - 1) == '.' && {
        let c = rope.char(start - 2);
        c.is_alphanumeric() || c == '_'
    } {
        let (ps, _) = rc_text::commands::word_at(rope, start - 2);
        start = ps;
    }
    rope.slice(start..e).to_string()
}

#[cfg(test)]
mod tests {
    #[test]
    fn hover_expression_follows_member_chains() {
        let rope = rc_text::Rope::from_str("var t = order.Customer.Name;");
        assert_eq!(super::expression_at(&rope, 25), "order.Customer.Name");
        assert_eq!(super::expression_at(&rope, 9), "order");
    }
}
