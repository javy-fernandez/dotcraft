//! The `.resx` editor tab (Rider's Resources editor): every string key of a resource file and of
//! its culture siblings (`Strings.resx`, `Strings.es.resx`, …) in one grid, one column per
//! culture. The files load in the background; the grid saves with its document (⌘S, Save All,
//! autosave, closing the tab) through `Docs::save`, which also updates `Strings.Designer.cs`.

use std::path::Path;

use rc_core::Runtime;
use rc_core::inbox::{Inbox, Waker};
use rc_dotnet::resx::{ResourceCell, ResourceGroup, ResourceRow, Saved};

/// What the `.resx` tab shows: the grid or the XML text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResxView {
    Grid,
    Xml,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Column {
    Key,
    Culture(usize),
    Comment,
}

/// The cell being edited. Edits apply to the row as you type; Escape puts `original` back.
#[derive(Clone, Debug)]
pub struct CellEdit {
    pub row: usize,
    pub column: Column,
    pub text: String,
    pub original: CellValue,
    /// First frame: focus the field (and select its text for a new key).
    pub focus: bool,
    pub select_all: bool,
}

#[derive(Clone, Debug)]
pub enum CellValue {
    Key(String),
    Cell(Option<ResourceCell>),
}

pub struct ResxEditor {
    pub view: ResxView,
    pub group: Option<ResourceGroup>,
    pub loading: bool,
    pub error: Option<String>,
    /// The files changed on disk while the grid had unsaved edits: the disk version waits here
    /// until the user keeps theirs or loads it.
    pub disk: Option<ResourceGroup>,
    /// The file whose comments the Comment column shows.
    pub comment_column: usize,
    pub filter: String,
    pub selected: Option<usize>,
    pub edit: Option<CellEdit>,
    pub scroll_to: Option<usize>,
    saved_rows: Vec<ResourceRow>,
    dirty: bool,
    revision: u64,
    cache: Option<ViewCache>,
    generation: u64,
    /// The next load replaces the grid even if it has edits (Load from Disk).
    replace: bool,
    inbox: Inbox<(u64, Result<ResourceGroup, String>)>,
}

/// Rows that match the filter and translation counts, recomputed only after an edit, a load or a
/// filter change (never per frame).
struct ViewCache {
    revision: u64,
    filter: String,
    rows: Vec<usize>,
    missing: Vec<usize>,
    /// Keys that are empty or used twice, by row (red key cells).
    problems: std::collections::HashSet<usize>,
}

impl ResxEditor {
    pub fn new(waker: Waker) -> Self {
        Self {
            view: ResxView::Grid,
            group: None,
            loading: false,
            error: None,
            disk: None,
            comment_column: 0,
            filter: String::new(),
            selected: None,
            edit: None,
            scroll_to: None,
            saved_rows: Vec::new(),
            dirty: false,
            revision: 0,
            cache: None,
            generation: 0,
            replace: true,
            inbox: Inbox::new(waker),
        }
    }

    pub fn dirty(&self) -> bool {
        self.dirty
    }

    /// Reads the resource files again in the background. Without `replace`, a result equal to
    /// what the grid shows is dropped, and one that differs while the grid has edits waits in
    /// [`ResxEditor::disk`].
    pub fn load(&mut self, path: &Path, replace: bool) {
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.loading = true;
        self.replace |= replace;
        let out = self.inbox.sender();
        let path = path.to_path_buf();
        Runtime::handle().spawn_blocking(move || {
            out.send((generation, ResourceGroup::load(&path).map_err(|error| format!("{error:#}"))));
        });
    }

    /// One of the files (or a sibling culture) changed on disk.
    pub fn on_disk_change(&mut self, path: &Path) {
        self.load(path, false);
    }

    /// Takes finished loads; call once per frame before drawing.
    pub fn update(&mut self) {
        for (generation, result) in self.inbox.drain() {
            if generation != self.generation {
                continue;
            }
            self.loading = false;
            let replace = std::mem::take(&mut self.replace);
            match result {
                Ok(group) => match &self.group {
                    _ if replace => self.install(group),
                    None => self.install(group),
                    Some(current) if current.same_files(&group) => self.error = None,
                    Some(_) if self.dirty => self.disk = Some(group),
                    Some(_) => self.install(group),
                },
                Err(error) => self.error = Some(error),
            }
        }
    }

    fn install(&mut self, group: ResourceGroup) {
        self.saved_rows = group.rows.clone();
        self.comment_column = self.comment_column.min(group.files.len().saturating_sub(1));
        self.selected = self.selected.filter(|&row| row < group.rows.len());
        self.group = Some(group);
        self.dirty = false;
        self.disk = None;
        self.error = None;
        self.edit = None;
        self.revision += 1;
    }

    /// Loads the disk version, dropping the grid's edits.
    pub fn take_disk(&mut self) {
        if let Some(group) = self.disk.take() {
            self.install(group);
        }
    }

    /// Keeps the grid's edits over the disk version: the next save writes them on top of it.
    pub fn keep_mine(&mut self) -> bool {
        let (Some(group), Some(disk)) = (self.group.as_mut(), self.disk.take()) else { return false };
        let same_columns = group.files.len() == disk.files.len() && group.files.iter().zip(&disk.files).all(|(a, b)| a.path == b.path);
        if !same_columns {
            // A culture file came or went: the rows no longer line up with the files.
            self.install(disk);
            return false;
        }
        group.files = disk.files;
        true
    }

    /// Call after changing `group.rows`.
    pub fn edited(&mut self) {
        self.revision += 1;
        self.dirty = self.group.as_ref().is_some_and(|group| group.rows != self.saved_rows);
    }

    /// Writes the edits (see [`ResourceGroup::save`]).
    pub fn save(&mut self) -> Result<Saved, String> {
        let Some(group) = self.group.as_mut() else { return Ok(Saved::default()) };
        let saved = group.save().map_err(|error| format!("{error:#}"))?;
        self.saved_rows = group.rows.clone();
        self.dirty = false;
        // Rows are sorted again after a save: indexes into them are stale.
        self.edit = None;
        self.selected = None;
        self.revision += 1;
        Ok(saved)
    }

    /// The rows the grid shows (indexes into `group.rows`) and, per column, how many of the
    /// shown keys lack a translation.
    pub fn visible(&mut self) -> (&[usize], &[usize]) {
        let view = self.view_cache();
        (&view.rows, &view.missing)
    }

    /// Keys that are empty or used twice, by row.
    pub fn key_problems(&mut self) -> &std::collections::HashSet<usize> {
        &self.view_cache().problems
    }

    fn view_cache(&mut self) -> &ViewCache {
        let stale = self.cache.as_ref().is_none_or(|view| view.revision != self.revision || view.filter != self.filter);
        if stale {
            let needle = self.filter.to_lowercase();
            let editing = self.edit.as_ref().map(|edit| edit.row);
            let mut rows = Vec::new();
            let mut missing = Vec::new();
            if let Some(group) = &self.group {
                missing = vec![0; group.files.len()];
                for (index, row) in group.rows.iter().enumerate() {
                    let matches = needle.is_empty()
                        || editing == Some(index)
                        || row.name.to_lowercase().contains(&needle)
                        || row.cells.iter().flatten().any(|cell| cell.value.to_lowercase().contains(&needle));
                    if !matches {
                        continue;
                    }
                    rows.push(index);
                    for (column, cell) in row.cells.iter().enumerate() {
                        missing[column] += usize::from(cell.is_none());
                    }
                }
            }
            let mut problems = std::collections::HashSet::new();
            let mut seen = std::collections::HashMap::new();
            for (index, row) in self.group.iter().flat_map(|group| group.rows.iter().enumerate()) {
                if row.name.trim().is_empty() {
                    problems.insert(index);
                } else if let Some(first) = seen.insert(row.name.as_str(), index) {
                    problems.insert(first);
                    problems.insert(index);
                }
            }
            self.cache = Some(ViewCache { revision: self.revision, filter: self.filter.clone(), rows, missing, problems });
        }
        self.cache.as_ref().expect("view cache was just filled")
    }

    /// Adds a key (`NewKey`, `NewKey1`, …) and starts editing its name.
    pub fn add_key(&mut self) {
        let Some(group) = self.group.as_mut() else { return };
        let mut name = "NewKey".to_string();
        let mut n = 1;
        while group.rows.iter().any(|row| row.name == name) {
            name = format!("NewKey{n}");
            n += 1;
        }
        let row = group.add_row(name.clone());
        self.filter.clear();
        self.selected = Some(row);
        self.scroll_to = Some(row);
        self.edit = Some(CellEdit { row, column: Column::Key, text: name.clone(), original: CellValue::Key(name), focus: true, select_all: true });
        self.edited();
    }

    /// Removes a key from every culture.
    pub fn remove_key(&mut self, row: usize) {
        let Some(group) = self.group.as_mut() else { return };
        if row >= group.rows.len() {
            return;
        }
        group.rows.remove(row);
        self.edit = None;
        self.selected = (!group.rows.is_empty()).then(|| row.min(group.rows.len() - 1));
        self.edited();
    }

    /// Removes one culture's translation of a key.
    pub fn remove_translation(&mut self, row: usize, column: usize) {
        if let Some(cell) = self.group.as_mut().and_then(|group| group.rows.get_mut(row)).and_then(|row| row.cells.get_mut(column)) {
            *cell = None;
            self.edit = None;
            self.edited();
        }
    }

    /// The text a cell shows and edits.
    pub fn cell_text(&self, row: usize, column: Column) -> Option<String> {
        let row = self.group.as_ref()?.rows.get(row)?;
        match column {
            Column::Key => Some(row.name.clone()),
            Column::Culture(index) => row.cells.get(index)?.as_ref().map(|cell| cell.value.clone()),
            Column::Comment => row.cells.get(self.comment_column)?.as_ref().and_then(|cell| cell.comment.clone()),
        }
    }

    /// Starts editing a cell (a click on it). The Comment column needs that culture's entry.
    pub fn start_edit(&mut self, row: usize, column: Column) {
        let Some(group) = &self.group else { return };
        let Some(data) = group.rows.get(row) else { return };
        let original = match column {
            Column::Key => CellValue::Key(data.name.clone()),
            Column::Culture(index) => CellValue::Cell(data.cells.get(index).cloned().flatten()),
            Column::Comment => match data.cells.get(self.comment_column) {
                Some(Some(cell)) => CellValue::Cell(Some(cell.clone())),
                _ => return,
            },
        };
        let text = self.cell_text(row, column).unwrap_or_default();
        self.selected = Some(row);
        self.edit = Some(CellEdit { row, column, text, original, focus: true, select_all: false });
    }

    /// Writes the edit's text into its row (as the user types).
    pub fn apply_edit(&mut self) {
        let Some(edit) = &self.edit else { return };
        let comment_column = self.comment_column;
        let Some(row) = self.group.as_mut().and_then(|group| group.rows.get_mut(edit.row)) else { return };
        match (edit.column, &edit.original) {
            (Column::Key, _) => row.name = edit.text.clone(),
            (Column::Culture(index), CellValue::Cell(original)) => {
                let Some(cell) = row.cells.get_mut(index) else { return };
                *cell = match (cell.take(), original) {
                    // Typing into a missing translation adds it; clearing it again removes it.
                    (None | Some(_), None) if edit.text.is_empty() => None,
                    (Some(mut cell), _) => {
                        cell.value = edit.text.clone();
                        Some(cell)
                    }
                    (None, original) => Some(ResourceCell { value: edit.text.clone(), comment: original.as_ref().and_then(|cell| cell.comment.clone()) }),
                };
            }
            (Column::Comment, _) => {
                if let Some(Some(cell)) = row.cells.get_mut(comment_column) {
                    cell.comment = (!edit.text.is_empty()).then(|| edit.text.clone());
                }
            }
            _ => return,
        }
        self.edited();
    }

    /// Escape: the cell gets its value from before the edit back.
    pub fn cancel_edit(&mut self) {
        let Some(edit) = self.edit.take() else { return };
        let comment_column = self.comment_column;
        let Some(row) = self.group.as_mut().and_then(|group| group.rows.get_mut(edit.row)) else { return };
        match (edit.column, edit.original) {
            (Column::Key, CellValue::Key(name)) => row.name = name,
            (Column::Culture(index), CellValue::Cell(cell)) => {
                if let Some(slot) = row.cells.get_mut(index) {
                    *slot = cell;
                }
            }
            (Column::Comment, CellValue::Cell(Some(cell))) => {
                if let Some(Some(slot)) = row.cells.get_mut(comment_column) {
                    slot.comment = cell.comment;
                }
            }
            _ => {}
        }
        self.edited();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pump(editor: &mut ResxEditor) {
        for _ in 0..200 {
            editor.update();
            if !editor.loading {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("the resource files did not load");
    }

    #[test]
    fn edits_mark_the_grid_dirty_and_escape_restores_the_cell() {
        let dir = tempfile::tempdir().unwrap();
        let neutral = dir.path().join("Strings.resx");
        std::fs::write(&neutral, "<root>\n  <data name=\"Hello\"><value>Hello</value></data>\n</root>\n").unwrap();
        std::fs::write(dir.path().join("Strings.fr.resx"), "<root>\n</root>\n").unwrap();
        let mut editor = ResxEditor::new(Waker::noop());
        editor.load(&neutral, true);
        pump(&mut editor);
        assert_eq!(editor.visible().1, &[0, 1], "the French column misses one translation");
        // Typing into the missing French cell adds the translation; Escape takes it away again.
        editor.start_edit(0, Column::Culture(1));
        editor.edit.as_mut().unwrap().text = "Bonjour".into();
        editor.apply_edit();
        assert!(editor.dirty());
        assert_eq!(editor.visible().1, &[0, 0]);
        editor.cancel_edit();
        assert!(!editor.dirty());
        // Clearing a translation typed into a missing cell leaves it missing.
        editor.start_edit(0, Column::Culture(1));
        editor.edit.as_mut().unwrap().text = "B".into();
        editor.apply_edit();
        editor.edit.as_mut().unwrap().text.clear();
        editor.apply_edit();
        assert!(!editor.dirty());
        // Comments belong to the chosen culture's entry.
        editor.start_edit(0, Column::Comment);
        editor.edit.as_mut().unwrap().text = "Greeting".into();
        editor.apply_edit();
        assert_eq!(editor.group.as_ref().unwrap().rows[0].cells[0].as_ref().unwrap().comment.as_deref(), Some("Greeting"));
        editor.add_key();
        editor.add_key();
        let names: Vec<_> = editor.group.as_ref().unwrap().rows.iter().map(|row| row.name.clone()).collect();
        assert_eq!(names, ["Hello", "NewKey", "NewKey1"]);
        editor.edit.as_mut().unwrap().text = "Hello".into();
        editor.apply_edit();
        assert_eq!(editor.key_problems(), &[0, 2].into_iter().collect());
        editor.remove_key(2);
        editor.filter = "new".into();
        assert_eq!(editor.visible().0, &[1]);
        editor.save().unwrap();
        assert!(!editor.dirty());
        let text = std::fs::read_to_string(&neutral).unwrap();
        assert!(text.contains("<comment>Greeting</comment>") && text.contains("name=\"NewKey\""));
        // A change on disk with no edits in the grid reloads it; with edits, it waits.
        editor.group.as_mut().unwrap().rows[0].name = "Changed".into();
        editor.edited();
        std::fs::write(&neutral, "<root>\n  <data name=\"Other\"><value>x</value></data>\n</root>\n").unwrap();
        editor.on_disk_change(&neutral);
        pump(&mut editor);
        assert!(editor.disk.is_some() && editor.dirty());
        editor.take_disk();
        assert_eq!(editor.group.as_ref().unwrap().rows.iter().map(|row| row.name.as_str()).collect::<Vec<_>>(), ["Other"]);
    }
}
