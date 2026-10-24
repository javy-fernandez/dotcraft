//! Code-insight popups shown by the editor: completion list, parameter info and hover/quick
//! documentation. The application fills them from the language server; the editor draws them,
//! routes the keyboard to them and reports the choice.

use rc_kit::Ico;

#[derive(Clone, Debug, PartialEq)]
pub struct CompletionItem {
    pub label: String,
    /// Shown greyed after the label (signature, namespace…).
    pub detail: String,
    /// Shown right-aligned (return type).
    pub type_text: String,
    pub icon: Ico,
    /// Text inserted when accepted (when the server sends no edit).
    pub insert: String,
    /// Char range replaced on accept (`None`: the word before the caret).
    pub replace: Option<(usize, usize)>,
    pub deprecated: bool,
    /// Opaque index back into the server's list.
    pub data: usize,
}

#[derive(Clone, Debug, Default)]
pub struct Completion {
    /// Char offset where the typed prefix starts.
    pub anchor: usize,
    pub items: Vec<CompletionItem>,
    /// Indices of `items` matching the prefix, best first.
    pub filtered: Vec<usize>,
    pub selected: usize,
    pub prefix: String,
    /// Documentation of the selected item, if resolved.
    pub doc: Option<String>,
    /// Set while the server has not answered yet.
    pub loading: bool,
    pub(crate) scroll_to_selected: bool,
}

impl Completion {
    /// Re-filters the items for `prefix`: camel-hump and substring matches, prefix matches
    /// first (IntelliJ's ordering, simplified).
    pub fn filter(&mut self, prefix: &str) {
        self.prefix = prefix.to_string();
        let mut scored: Vec<(i32, usize)> = self
            .items
            .iter()
            .enumerate()
            .filter_map(|(i, it)| fuzzy_score(&it.label, prefix).map(|s| (s, i)))
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(self.items[a.1].label.len().cmp(&self.items[b.1].label.len())).then(a.1.cmp(&b.1)));
        self.filtered = scored.into_iter().map(|(_, i)| i).collect();
        self.selected = 0;
        self.scroll_to_selected = true;
    }

    pub fn selected_item(&self) -> Option<&CompletionItem> {
        self.filtered.get(self.selected).map(|&i| &self.items[i])
    }

    pub(crate) fn move_selection(&mut self, delta: isize) {
        if self.filtered.is_empty() {
            return;
        }
        let n = self.filtered.len() as isize;
        self.selected = ((self.selected as isize + delta).rem_euclid(n)) as usize;
        self.scroll_to_selected = true;
    }
}

/// Score of `label` for the typed `pattern` (None: no match). Case-insensitive; a prefix
/// match beats a camel-hump match, which beats a plain subsequence.
pub fn fuzzy_score(label: &str, pattern: &str) -> Option<i32> {
    if pattern.is_empty() {
        return Some(0);
    }
    let l: Vec<char> = label.chars().collect();
    let p: Vec<char> = pattern.chars().collect();
    let lower = |c: char| c.to_lowercase().next().unwrap_or(c);
    if l.len() >= p.len() && l.iter().zip(&p).all(|(a, b)| lower(*a) == lower(*b)) {
        let exact_case = l.iter().zip(&p).all(|(a, b)| a == b);
        return Some(1000 - l.len() as i32 + if exact_case { 50 } else { 0 });
    }
    // Camel humps: every pattern char matches a word start in order (`GBN` → `GetByName`).
    let starts: Vec<usize> = (0..l.len())
        .filter(|&i| i == 0 || l[i].is_uppercase() && !l[i - 1].is_uppercase() || l[i - 1] == '_' || l[i].is_ascii_digit() && !l[i - 1].is_ascii_digit())
        .collect();
    let mut pi = 0;
    let mut li = 0;
    let mut humps = 0;
    while pi < p.len() && li < l.len() {
        if lower(l[li]) == lower(p[pi]) {
            if starts.contains(&li) {
                humps += 1;
            }
            pi += 1;
        }
        li += 1;
    }
    if pi < p.len() {
        return None;
    }
    Some(if humps == p.len() { 500 } else { 100 + humps as i32 * 10 } - l.len() as i32)
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SignatureHelp {
    /// Char offset of the call's opening parenthesis (the popup sits above it).
    pub anchor: usize,
    pub signatures: Vec<Signature>,
    pub active: usize,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Signature {
    pub label: String,
    /// Char ranges of each parameter inside `label`.
    pub params: Vec<(usize, usize)>,
    pub active_param: Option<usize>,
    pub doc: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Hover {
    /// Char offset the hover belongs to.
    pub pos: usize,
    /// Markdown-ish text (code blocks rendered monospace).
    pub text: String,
    /// Diagnostic message shown above the documentation.
    pub problem: Option<(crate::state::Severity, String)>,
    pub screen: egui::Pos2,
}

#[derive(Default)]
pub struct Popups {
    pub completion: Option<Completion>,
    pub signature: Option<SignatureHelp>,
    pub hover: Option<Hover>,
}

impl Popups {
    pub fn any(&self) -> bool {
        self.completion.is_some() || self.signature.is_some() || self.hover.is_some()
    }

    pub fn close_all(&mut self) {
        *self = Popups::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_beats_camel_humps_beats_subsequence() {
        let a = fuzzy_score("WriteLine", "Wri").unwrap();
        let b = fuzzy_score("WriteLine", "WL").unwrap();
        let c = fuzzy_score("WriteLine", "ine").unwrap();
        assert!(a > b && b > c, "{a} {b} {c}");
        assert!(fuzzy_score("WriteLine", "xyz").is_none());
        assert!(fuzzy_score("GetByName", "gbn").is_some());
    }
}
