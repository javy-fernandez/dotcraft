//! Code insight for `appsettings*.json`, drawn with the editor's own completion list, hover and
//! diagnostics: keys and values from the bundled JSON schema (Logging, Kestrel,
//! ConnectionStrings, Serilog…) and from the options classes the workspace binds to sections
//! (`Configure<T>(GetSection("X"))`, `BindConfiguration("X")`, `SectionName` constants), with
//! ⌘-click / Go to Declaration from a key to its class or property.
//!
//! Analysis is synchronous (the files are small) after typing pauses; the C# files are scanned
//! for options classes on a background thread when the workspace opens and when they change.

use crate::docs::{Doc, Docs};
use rc_core::{Inbox, Runtime, Waker};
use rc_dotnet::appsettings::options::{self, OptionsIndex};
use rc_dotnet::appsettings::schema::{Problem, Schema, value_text};
use rc_dotnet::appsettings::{self, Bound, ItemKind, KeyInfo};
use rc_dotnet::project_xml::{byte_of, char_of};
use rc_editor::{Completion, CompletionItem, EditorEvent, Hover, Severity};
use rc_kit::i18n::{tr, trf};
use rc_kit::ico;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

const ANALYZE_DELAY: Duration = Duration::from_millis(250);
const SCAN_DELAY: Duration = Duration::from_millis(800);

pub use rc_dotnet::appsettings::is_appsettings;

/// What the rest of the UI must do.
#[derive(Debug, PartialEq)]
pub enum SettingsOutput {
    /// Open a declaration (0-based line and column).
    Open { path: PathBuf, line: usize, col: usize },
    Message(String),
}

enum Event {
    Index(u64, Arc<OptionsIndex>),
    Tick,
}

struct DocState {
    path: PathBuf,
    version: Option<u64>,
    seen: (u64, Instant),
    index_gen: u64,
    diags: Vec<rc_editor::Diagnostic>,
}

/// The documentation of the open list's items (by `data`).
struct CompletionDocs {
    doc: u64,
    anchor: usize,
    docs: Vec<Option<String>>,
}

pub struct SettingsInsight {
    inbox: Inbox<Event>,
    root: Option<PathBuf>,
    index: Arc<OptionsIndex>,
    index_gen: u64,
    scan_gen: u64,
    scan_due: Option<Instant>,
    scanning: bool,
    states: HashMap<u64, DocState>,
    published: HashMap<PathBuf, Vec<rc_editor::Diagnostic>>,
    problems_dirty: bool,
    completion: Option<CompletionDocs>,
    /// Declarations offered by the open hover's buttons.
    hover_targets: Vec<Bound>,
}

fn code(s: &str) -> String {
    format!("`{s}`")
}

fn list(items: &[String]) -> String {
    items.iter().map(|s| code(s)).collect::<Vec<_>>().join(", ")
}

/// The localized message of a problem (plain text: the editor shows it as is).
pub fn message(p: &Problem) -> String {
    match p {
        Problem::Type { expected, found } => {
            let expected = expected.join(&format!(" {} ", tr("or")));
            trf("Incorrect type: expected {expected}, found {found}", &[("expected", &expected), ("found", found)])
        }
        Problem::Enum { allowed } => trf("Value is not accepted. Valid values: {values}", &[("values", &allowed.join(", "))]),
        Problem::Bare(w) => trf("Value expected: write {value} in quotes", &[("value", w)]),
    }
}

impl SettingsInsight {
    pub fn new(waker: Waker) -> Self {
        SettingsInsight {
            inbox: Inbox::new(waker),
            root: None,
            index: Arc::new(OptionsIndex::default()),
            index_gen: 0,
            scan_gen: 0,
            scan_due: None,
            scanning: false,
            states: HashMap::new(),
            published: HashMap::new(),
            problems_dirty: false,
            completion: None,
            hover_targets: Vec::new(),
        }
    }

    /// A workspace was opened: its C# files are scanned for options classes.
    pub fn set_workspace(&mut self, root: &Path) {
        self.root = Some(root.to_path_buf());
        self.index = Arc::new(OptionsIndex::default());
        self.index_gen += 1;
        self.schedule_scan(Duration::ZERO);
    }

    /// Files changed on disk: C# files may bind other sections now.
    pub fn files_changed(&mut self, paths: &[PathBuf]) {
        if self.root.is_some() && paths.iter().any(|p| p.extension().is_some_and(|e| e == "cs")) {
            self.schedule_scan(SCAN_DELAY);
        }
    }

    fn schedule_scan(&mut self, delay: Duration) {
        self.scan_due = Some(Instant::now() + delay);
        self.wake_after(delay);
    }

    fn wake_after(&self, delay: Duration) {
        let out = self.inbox.sender();
        Runtime::spawn(async move {
            tokio::time::sleep(delay).await;
            out.send(Event::Tick);
        });
    }

    /// Build results replaced the Problems window's entries: show ours again.
    pub fn republish(&mut self) {
        self.published.clear();
        self.problems_dirty = true;
    }

    /// Once per frame: scan results, analysis of edited files, problems, completion docs.
    pub fn update(&mut self, docs: &mut Docs, problems: &mut BTreeMap<PathBuf, Vec<rc_editor::Diagnostic>>) {
        for ev in self.inbox.drain() {
            match ev {
                Event::Index(generation, index) if generation == self.scan_gen => {
                    self.scanning = false;
                    self.index = index;
                    self.index_gen += 1;
                }
                _ => {}
            }
        }
        if let Some(due) = self.scan_due
            && Instant::now() >= due
            && !self.scanning
            && let Some(root) = self.root.clone()
        {
            self.scan_due = None;
            self.scanning = true;
            self.scan_gen += 1;
            let (out, generation) = (self.inbox.sender(), self.scan_gen);
            std::thread::spawn(move || out.send(Event::Index(generation, Arc::new(options::scan(&root)))));
        }
        let open: Vec<u64> = docs.list.iter().filter(|d| is_appsettings(&d.path)).map(|d| d.editor.id).collect();
        let before = self.states.len();
        self.states.retain(|id, _| open.contains(id));
        if self.states.len() != before {
            self.problems_dirty = true;
        }
        for d in docs.list.iter_mut().filter(|d| is_appsettings(&d.path)) {
            self.refresh(d);
        }
        if self.problems_dirty {
            self.problems_dirty = false;
            self.sync_problems(docs, problems);
        }
        // Documentation of the selected completion item.
        if let Some(cd) = &self.completion
            && let Some(d) = docs.list.iter_mut().find(|d| d.editor.id == cd.doc)
        {
            match d.editor.popups.completion.as_mut() {
                Some(c) if c.anchor == cd.anchor => {
                    let doc = c.selected_item().and_then(|i| cd.docs.get(i.data).cloned().flatten());
                    if c.doc != doc {
                        c.doc = doc;
                    }
                }
                _ => self.completion = None,
            }
        }
    }

    fn refresh(&mut self, d: &mut Doc) {
        let id = d.editor.id;
        let version = d.editor.buffer.version();
        let now = Instant::now();
        let st = self.states.entry(id).or_insert_with(|| DocState { path: d.path.clone(), version: None, seen: (version, now.checked_sub(ANALYZE_DELAY).unwrap_or(now)), index_gen: 0, diags: Vec::new() });
        st.path = d.path.clone();
        if st.seen.0 != version {
            st.seen = (version, now);
        }
        let changed = st.version != Some(version);
        if changed && now - st.seen.1 < ANALYZE_DELAY {
            let wait = ANALYZE_DELAY - (now - st.seen.1);
            self.wake_after(wait);
            return;
        }
        if !changed && st.index_gen == self.index_gen {
            return;
        }
        st.version = Some(version);
        st.index_gen = self.index_gen;
        let text = d.editor.buffer.rope().to_string();
        let diags: Vec<rc_editor::Diagnostic> = appsettings::validate(&text, Schema::appsettings(), &self.index)
            .into_iter()
            .map(|f| rc_editor::Diagnostic {
                start: char_of(&text, f.range.start),
                end: char_of(&text, f.range.end),
                severity: if matches!(f.problem, Problem::Bare(_)) { Severity::Error } else { Severity::Warning },
                message: message(&f.problem),
                code: None,
                unnecessary: false,
                deprecated: false,
            })
            .collect();
        if st.diags != diags {
            st.diags = diags;
            self.problems_dirty = true;
        }
    }

    /// Puts the open files' problems in the Problems window and their editors (next to the
    /// build's).
    fn sync_problems(&mut self, docs: &mut Docs, problems: &mut BTreeMap<PathBuf, Vec<rc_editor::Diagnostic>>) {
        let mut want: BTreeMap<PathBuf, Vec<rc_editor::Diagnostic>> = BTreeMap::new();
        for st in self.states.values().filter(|s| s.version.is_some()) {
            want.insert(st.path.clone(), st.diags.clone());
        }
        for p in self.published.keys() {
            want.entry(p.clone()).or_default();
        }
        for (path, list) in want {
            let old = self.published.remove(&path).unwrap_or_default();
            let entry = problems.entry(path.clone()).or_default();
            entry.retain(|d| !old.contains(d));
            for d in &list {
                if !entry.contains(d) {
                    entry.push(d.clone());
                }
            }
            if entry.is_empty() {
                problems.remove(&path);
            }
            if !list.is_empty() {
                self.published.insert(path, list);
            }
        }
        for d in docs.list.iter_mut() {
            let Some(st) = self.states.get(&d.editor.id).filter(|s| s.version.is_some()) else { continue };
            let mut all = st.diags.clone();
            all.extend(problems.get(&d.path).into_iter().flatten().filter(|x| !st.diags.contains(x)).cloned());
            d.editor.set_diagnostics(all);
        }
    }

    // ------------------------------------------------------------------ editor events

    /// Reacts to what an `appsettings*.json` editor did this frame.
    pub fn on_editor_events(&mut self, d: &mut Doc, events: &[EditorEvent]) -> Vec<SettingsOutput> {
        let mut out = Vec::new();
        for ev in events {
            match ev {
                EditorEvent::Typed(c) if *c == '"' || c.is_alphanumeric() || *c == '_' => {
                    if d.editor.popups.completion.is_none() {
                        self.complete(d, false);
                    }
                }
                EditorEvent::Changed => self.fix_replace_ranges(d),
                EditorEvent::HoverAt { pos, screen } => self.hover(d, *pos, *screen),
                EditorEvent::HoverAction(i) => {
                    if let Some(b) = self.hover_targets.get(*i) {
                        out.push(SettingsOutput::Open { path: b.file.clone(), line: b.line, col: b.col });
                    }
                }
                EditorEvent::NavigateAt(pos) => out.extend(self.declaration(d, *pos)),
                _ => {}
            }
        }
        out
    }

    /// A completion item was accepted: a key whose value has known values lists them.
    pub fn accepted(&mut self, d: &mut Doc) {
        let text = d.editor.buffer.rope().to_string();
        let b = byte_of(&text, d.editor.caret());
        if let Some(rc_dotnet::appsettings::json::Spot::Value { .. }) = rc_dotnet::appsettings::json::parse(&text).and_then(|r| rc_dotnet::appsettings::json::spot_at(&r, &text, b)) {
            self.complete(d, false);
        }
    }

    /// Completion of a key or a value (typing, or ⌃Space when `explicit`).
    pub fn complete(&mut self, d: &mut Doc, explicit: bool) {
        let text = d.editor.buffer.rope().to_string();
        let caret = d.editor.caret();
        let Some(c) = appsettings::complete(&text, byte_of(&text, caret), Schema::appsettings(), &self.index) else {
            if explicit {
                d.editor.popups.completion = None;
            }
            return;
        };
        let (anchor, start, end) = (char_of(&text, c.anchor), char_of(&text, c.range.start), char_of(&text, c.range.end));
        let mut docs = Vec::new();
        let items: Vec<CompletionItem> = c
            .items
            .into_iter()
            .enumerate()
            .map(|(i, it)| {
                docs.push(it.doc.clone());
                CompletionItem {
                    label: it.label,
                    detail: it.detail,
                    type_text: if it.is_default { tr("default").to_string() } else { it.type_text },
                    icon: match it.kind {
                        ItemKind::Key | ItemKind::Property => ico::PROPERTY,
                        ItemKind::Enum => ico::ENUM,
                        ItemKind::Literal => ico::KEYWORD,
                    },
                    insert: it.insert,
                    replace: Some((start, end)),
                    deprecated: false,
                    data: i,
                }
            })
            .collect();
        let prefix = d.editor.buffer.rope().slice(anchor..caret.max(anchor)).to_string();
        let mut list = Completion::new(anchor, items, false);
        list.keep_order = true;
        list.filter(&prefix);
        if list.filtered.is_empty() {
            d.editor.popups.completion = None;
            return;
        }
        self.completion = Some(CompletionDocs { doc: d.editor.id, anchor, docs });
        d.editor.popups.completion = Some(list);
    }

    /// Typing moved the end of the key or value: accepting must still replace all of it.
    fn fix_replace_ranges(&mut self, d: &mut Doc) {
        let Some(open) = d.editor.popups.completion.as_ref().map(|c| c.anchor) else { return };
        let text = d.editor.buffer.rope().to_string();
        let b = byte_of(&text, d.editor.caret());
        let Some(root) = rc_dotnet::appsettings::json::parse(&text) else { return };
        let range = match rc_dotnet::appsettings::json::spot_at(&root, &text, b) {
            Some(rc_dotnet::appsettings::json::Spot::Key { range, .. } | rc_dotnet::appsettings::json::Spot::Value { range, .. }) => range,
            None => return,
        };
        let (start, end) = (char_of(&text, range.start), char_of(&text, range.end));
        if let Some(c) = d.editor.popups.completion.as_mut()
            && (start == open || start + 1 == open)
        {
            for it in &mut c.items {
                it.replace = Some((start, end));
            }
        }
    }

    fn key_at(&self, d: &Doc, pos: usize) -> Option<KeyInfo> {
        let text = d.editor.buffer.rope().to_string();
        appsettings::key_at(&text, byte_of(&text, pos), Schema::appsettings(), &self.index)
    }

    fn hover(&mut self, d: &mut Doc, pos: usize, screen: egui::Pos2) {
        let problem = d.editor.diagnostic_at(pos).map(|x| (x.severity, x.message.clone()));
        let Some(k) = self.key_at(d, pos) else {
            if problem.is_some() {
                d.editor.popups.hover = Some(Hover { pos, text: String::new(), problem, screen, actions: Vec::new() });
            }
            return;
        };
        let mut lines = vec![code(&k.path)];
        if let Some(doc) = &k.schema {
            if let Some(desc) = &doc.description {
                lines.push(desc.clone());
            }
            let mut facts = Vec::new();
            if !doc.types.is_empty() {
                facts.push(trf("Type: {type}", &[("type", &doc.types.iter().map(|t| code(t)).collect::<Vec<_>>().join(" | "))]));
            }
            if !doc.enums.is_empty() {
                facts.push(trf("Valid values: {values}", &[("values", &list(&doc.enums.iter().map(value_text).collect::<Vec<_>>()))]));
            }
            if let Some(def) = &doc.default {
                facts.push(trf("Default: {value}", &[("value", &code(&value_text(def)))]));
            }
            if !facts.is_empty() {
                lines.push("---".into());
                lines.extend(facts);
            }
        }
        let mut actions = Vec::new();
        self.hover_targets.clear();
        if !k.bound.is_empty() {
            lines.push("---".into());
        }
        for b in &k.bound {
            let name = match &b.prop {
                Some(p) => format!("{}.{p}", b.class),
                None => b.class.clone(),
            };
            match &b.ty {
                Some(t) => lines.push(trf("Bound to {property} ({type})", &[("property", &code(&name)), ("type", &code(t))])),
                None => lines.push(trf("Section bound to {class}", &[("class", &code(&name))])),
            }
            if let Some(doc) = &b.doc {
                lines.push(doc.clone());
            }
            actions.push((if b.prop.is_some() { ico::PROPERTY } else { ico::CLASS }, trf("Go to {name}", &[("name", &name)])));
            self.hover_targets.push(b.clone());
        }
        if lines.len() == 1 && problem.is_none() {
            return;
        }
        d.editor.popups.hover = Some(Hover { pos, text: lines.join("\n"), problem, screen, actions });
    }

    /// F1: the hover of the caret's key, under the caret.
    pub fn quick_doc(&mut self, d: &mut Doc, screen: egui::Pos2) {
        self.hover(d, d.editor.caret(), screen);
    }

    /// The class or property the key at `pos` binds to.
    pub fn declaration(&self, d: &Doc, pos: usize) -> Vec<SettingsOutput> {
        match self.key_at(d, pos).and_then(|k| k.bound.into_iter().next()) {
            Some(b) => vec![SettingsOutput::Open { path: b.file, line: b.line, col: b.col }],
            None => vec![SettingsOutput::Message(tr("No options class is bound to this key").into())],
        }
    }

    #[cfg(test)]
    pub fn idle_for_test(&self) -> bool {
        self.scan_due.is_none() && !self.scanning
    }

    #[cfg(test)]
    pub fn index_for_test(&self) -> &OptionsIndex {
        &self.index
    }
}
