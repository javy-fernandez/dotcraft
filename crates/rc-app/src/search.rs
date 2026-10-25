//! Search: the file index, Search Everywhere (files, types, symbols, actions), the editor's
//! find/replace bar and Find in Files.

use rc_core::{Inbox, Waker};
use rc_editor::Action;
use rc_kit::i18n::tr;
use rc_lsp::Pending;
use rc_search::{FileIndex, FindEvent, FindOptions, FindSummary, Finder, FileMatches, QueryOptions, ScanHandle};
use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeTab {
    All,
    Types,
    Files,
    Symbols,
    Actions,
}

impl SeTab {
    pub const ALL: [SeTab; 5] = [SeTab::All, SeTab::Types, SeTab::Files, SeTab::Symbols, SeTab::Actions];

    pub fn title(self) -> &'static str {
        match self {
            SeTab::All => tr("All"),
            SeTab::Types => tr("Types"),
            SeTab::Files => tr("Files"),
            SeTab::Symbols => tr("Symbols"),
            SeTab::Actions => tr("Actions"),
        }
    }
}

/// A symbol from `workspace/symbol`.
#[derive(Clone, Debug, PartialEq)]
pub struct SymbolHit {
    pub name: String,
    pub container: String,
    pub kind: rc_lsp::lsp_types::SymbolKind,
    pub path: PathBuf,
    pub line: usize,
    pub col: usize,
}

impl SymbolHit {
    pub fn is_type(&self) -> bool {
        use rc_lsp::lsp_types::SymbolKind as K;
        matches!(self.kind, K::CLASS | K::INTERFACE | K::STRUCT | K::ENUM | K::TYPE_PARAMETER)
    }
}

pub struct SearchEverywhere {
    pub text: String,
    pub tab: SeTab,
    pub selected: usize,
    pub focus: bool,
    pub symbols: Vec<SymbolHit>,
    symbol_query: String,
    symbol_pending: Option<Pending<Value>>,
}

impl SearchEverywhere {
    pub fn new(tab: SeTab, text: String) -> Self {
        SearchEverywhere { text, tab, selected: 0, focus: true, symbols: Vec::new(), symbol_query: String::new(), symbol_pending: None }
    }

    /// Asks the language server for symbols matching the text (latest query only).
    pub fn update_symbols(&mut self, insight: &crate::insight::Insight) {
        if let Some(p) = &mut self.symbol_pending
            && let Some(r) = p.poll()
        {
            self.symbol_pending = None;
            self.symbols = r.map(|v| parse_symbols(&v)).unwrap_or_default();
        }
        let wants = matches!(self.tab, SeTab::All | SeTab::Types | SeTab::Symbols) && self.text.trim().len() >= 2;
        if wants && self.text != self.symbol_query && self.symbol_pending.is_none() {
            self.symbol_query = self.text.clone();
            self.symbol_pending = insight.workspace_symbols(self.text.trim());
        }
        if !wants {
            self.symbols.clear();
        }
    }
}

fn parse_symbols(v: &Value) -> Vec<SymbolHit> {
    let Some(list) = v.as_array() else { return Vec::new() };
    list.iter()
        .filter_map(|s| {
            let name = s.get("name")?.as_str()?.to_string();
            let kind: rc_lsp::lsp_types::SymbolKind = serde_json::from_value(s.get("kind")?.clone()).ok()?;
            let loc = s.get("location")?;
            let uri: rc_lsp::lsp_types::Uri = loc.get("uri")?.as_str()?.parse().ok()?;
            let path = rc_lsp::uri_to_path(&uri)?;
            let (line, col) = loc
                .get("range")
                .and_then(|r| r.get("start"))
                .map(|p| (p.get("line").and_then(Value::as_u64).unwrap_or(0) as usize, p.get("character").and_then(Value::as_u64).unwrap_or(0) as usize))
                .unwrap_or((0, 0));
            Some(SymbolHit { name, container: s.get("containerName").and_then(Value::as_str).unwrap_or("").to_string(), kind, path, line, col })
        })
        .collect()
}

/// The IDE actions offered by Search Everywhere's Actions tab.
pub fn action_items() -> Vec<(&'static str, Action)> {
    use Action::*;
    vec![
        (tr("Build Solution"), Build),
        (tr("Run"), Run),
        (tr("Debug"), Debug),
        (tr("Stop"), StopRun),
        (tr("Run Tests"), RunTests),
        (tr("Settings…"), ShowSettings),
        (tr("Find in Files…"), FindInFiles),
        (tr("Replace in Files…"), ReplaceInFiles),
        (tr("Go to File…"), GotoFile),
        (tr("Go to Type…"), GotoClass),
        (tr("Go to Symbol…"), GotoSymbol),
        (tr("Go to Line…"), GotoLine),
        (tr("Recent Files"), RecentFiles),
        (tr("Reformat Code"), Reformat),
        (tr("File Structure"), FileStructure),
        (tr("Explorer"), ToggleProjectWindow),
        (tr("Terminal"), ToggleTerminal),
        (tr("Problems"), ToggleProblems),
        (tr("Git"), ToggleGitWindow),
        (tr("Commit…"), Commit),
        (tr("Hide All Tool Windows"), HideToolWindows),
        (tr("Save All"), SaveAll),
        (tr("Zoom In"), ZoomIn),
        (tr("Zoom Out"), ZoomOut),
        (tr("Reset Zoom"), ZoomReset),
        (tr("Collapse All"), FoldAll),
        (tr("Expand All"), UnfoldAll),
    ]
}

/// The workspace file index and its background scan.
pub struct Files {
    pub index: Option<FileIndex>,
    _scan: Option<ScanHandle>,
}

impl Files {
    pub fn new() -> Self {
        Files { index: None, _scan: None }
    }

    pub fn open(&mut self, root: &Path, waker: Waker) {
        let index = FileIndex::new(root);
        self._scan = Some(index.start_scan(waker));
        self.index = Some(index);
    }

    pub fn close(&mut self) {
        self.index = None;
        self._scan = None;
    }
}

/// The editor's find / replace bar.
#[derive(Clone, Debug, Default)]
pub struct FindBar {
    pub query: String,
    pub replace: Option<String>,
    pub case: bool,
    pub word: bool,
    pub regex: bool,
    pub error: Option<String>,
    pub focus: bool,
    /// Query and document version the matches were computed for.
    pub computed: Option<(String, bool, bool, bool, u64, u64)>,
}

impl FindBar {
    pub fn options(&self) -> QueryOptions {
        QueryOptions { pattern: self.query.clone(), regex: self.regex, case_sensitive: self.case, whole_word: self.word }
    }
}

/// Find in Files (the Find tool window's text search).
pub struct FindInFiles {
    pub query: String,
    pub replace: String,
    pub show_replace: bool,
    pub case: bool,
    pub word: bool,
    pub regex: bool,
    pub mask: String,
    pub results: Vec<FileMatches>,
    pub summary: Option<FindSummary>,
    pub error: Option<String>,
    pub focus: bool,
    finder: Option<Finder>,
    inbox: Inbox<FindEvent>,
    root: PathBuf,
}

impl FindInFiles {
    pub fn new(root: &Path, waker: Waker, query: String) -> Self {
        let inbox = Inbox::new(waker);
        let finder = Some(Finder::new(root, inbox.sender()));
        FindInFiles {
            query,
            replace: String::new(),
            show_replace: false,
            case: false,
            word: false,
            regex: false,
            mask: String::new(),
            results: Vec::new(),
            summary: None,
            error: None,
            focus: true,
            finder,
            inbox,
            root: root.to_path_buf(),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn options(&self) -> QueryOptions {
        QueryOptions { pattern: self.query.clone(), regex: self.regex, case_sensitive: self.case, whole_word: self.word }
    }

    /// Starts (or restarts) the search with the current fields.
    pub fn search(&mut self) {
        self.results.clear();
        self.summary = None;
        self.error = None;
        if self.query.is_empty() {
            if let Some(f) = &mut self.finder {
                f.cancel();
            }
            return;
        }
        let mask = match rc_search::FileMask::parse(&self.mask) {
            Ok(m) => m,
            Err(e) => {
                self.error = Some(format!("{e:?}"));
                return;
            }
        };
        let opts = FindOptions { query: self.options(), mask, max_results: 5000, ..Default::default() };
        if let Some(f) = &mut self.finder
            && let Err(e) = f.start(opts)
        {
            self.error = Some(format!("{e:?}"));
        }
    }

    pub fn searching(&self) -> bool {
        self.summary.is_none() && self.finder.as_ref().is_some_and(|f| f.current().is_some()) && !self.query.is_empty() && self.error.is_none()
    }

    pub fn poll(&mut self) {
        let current = self.finder.as_ref().and_then(|f| f.current());
        for ev in self.inbox.drain() {
            if Some(ev.search()) != current {
                continue;
            }
            match ev {
                FindEvent::File { file, .. } => self.results.push(file),
                FindEvent::Done { summary, .. } => self.summary = Some(summary),
            }
        }
    }

    pub fn match_count(&self) -> usize {
        self.results.iter().map(|f| f.match_count()).sum()
    }
}

/// The icon of an IDE action, shared by the native menu, popup menus and Search Everywhere.
pub fn action_icon(action: Action) -> Option<rc_kit::Ico> {
    use Action::*;
    use rc_kit::ico;
    Some(match action {
        Build => ico::BUILD,
        Run => ico::RUN,
        Debug => ico::DEBUG,
        StopRun => ico::STOP,
        RunTests => ico::RUN_ALL,
        StepOver => ico::STEP_OVER,
        StepInto => ico::STEP_INTO,
        StepOut => ico::STEP_OUT,
        RunToCursor => ico::RUN_TO_CURSOR,
        Resume => ico::RESUME,
        Evaluate => ico::EVALUATE,
        ToggleBreakpoint => ico::BREAKPOINT,
        ShowSettings => ico::SETTINGS,
        Find | SearchEverywhere => ico::SEARCH,
        Replace | ReplaceInFiles => ico::REPLACE,
        FindInFiles => ico::TW_FIND,
        GotoFile => ico::FILE_CS,
        GotoClass => ico::CLASS,
        GotoSymbol => ico::METHOD,
        GotoLine => ico::ARROW_RIGHT,
        NavigateBack => ico::CHEVRON_LEFT,
        NavigateForward => ico::CHEVRON_RIGHT,
        RecentFiles | RecentLocations => ico::HISTORY,
        Reformat => ico::REFORMAT,
        FileStructure => ico::TW_STRUCTURE,
        ToggleProjectWindow => ico::TW_PROJECT,
        ToggleTerminal => ico::FILE_SHELL,
        ToggleProblems => ico::TW_PROBLEMS,
        ToggleGitWindow => ico::TW_VCS,
        Commit => ico::COMMIT,
        HideToolWindows => ico::HIDE,
        SaveAll | Save => ico::SAVE,
        CloseTab => ico::CLOSE,
        FoldAll | Fold => ico::COLLAPSE_ALL,
        UnfoldAll | Unfold => ico::EXPAND_ALL,
        Rename => ico::EDIT,
        FindUsages => ico::TW_FIND,
        GotoDeclaration | GotoImplementation | GotoTypeDeclaration => ico::ARROW_RIGHT,
        ShowIntentions => ico::BULB,
        QuickDoc => ico::INFO,
        Complete => ico::METHOD,
        CommentLine | CommentBlock => ico::EDIT,
        DuplicateLine => ico::COPY,
        NextError | PreviousError => ico::ERROR,
        OpenFolder => ico::OPEN,
        _ => return None,
    })
}
