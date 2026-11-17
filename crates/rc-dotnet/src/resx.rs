//! String resources of a `.resx` file and its sibling culture files (`Strings.resx`,
//! `Strings.es.resx`, …), edited without reserializing the XML: a save rewrites only the spans of
//! the entries that changed, so headers, the schema, comments, typed entries (images, file
//! references, byte arrays), metadata, the BOM and the line endings stay byte for byte.
//! [`designer_edits`] keeps the `ResXFileCodeGenerator` output (`Strings.Designer.cs`) in step.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};

const BOM: &str = "\u{FEFF}";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResourceCell {
    pub value: String,
    pub comment: Option<String>,
}

impl ResourceCell {
    pub fn new(value: impl Into<String>) -> Self {
        Self { value: value.into(), comment: None }
    }
}

#[derive(Clone, Debug)]
pub struct ResourceFile {
    pub path: PathBuf,
    /// `None` for the neutral file (`Strings.resx`), `Some("es")` for `Strings.es.resx`.
    pub culture: Option<String>,
    bom: bool,
    /// The file as read (without the BOM): the base every save patches.
    original: String,
    entries: BTreeMap<String, ResourceCell>,
    /// Every `<data>` name, typed entries included.
    all_names: BTreeSet<String>,
}

impl ResourceFile {
    fn read(path: PathBuf, culture: Option<String>) -> Result<Self> {
        let text = std::fs::read_to_string(&path).with_context(|| format!("cannot read {}", path.display()))?;
        let (bom, original) = match text.strip_prefix(BOM) {
            Some(rest) => (true, rest.to_string()),
            None => (false, text),
        };
        let (entries, all_names) = parse_entries(&original).with_context(|| format!("invalid resource XML in {}", path.display()))?;
        Ok(Self { path, culture, bom, original, entries, all_names })
    }

    fn disk_text(&self, text: &str) -> String {
        if self.bom { format!("{BOM}{text}") } else { text.to_string() }
    }

    /// Entries the grid does not show: images, file references, byte arrays…
    pub fn typed_entries(&self) -> usize {
        self.all_names.len() - self.entries.len()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResourceRow {
    /// The key as loaded; `None` for a key added in the editor.
    pub original_name: Option<String>,
    pub name: String,
    /// One per file of the group: `None` where the culture has no translation.
    pub cells: Vec<Option<ResourceCell>>,
}

#[derive(Clone, Debug)]
pub struct ResourceGroup {
    pub selected_path: PathBuf,
    pub files: Vec<ResourceFile>,
    pub rows: Vec<ResourceRow>,
}

/// What a save changed in the neutral file, for its `.Designer.cs`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesignerUpdate {
    pub path: PathBuf,
    /// String keys and values before and after the save.
    pub before: BTreeMap<String, String>,
    pub after: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default)]
pub struct Saved {
    /// Files written: path, text before and text after (without the BOM).
    pub written: Vec<(PathBuf, String, String)>,
    pub designer: Option<DesignerUpdate>,
}

/// The resource base name and culture of a `.resx` path: `Strings.es.resx` → (`Strings`, `es`).
pub fn base_and_culture(path: &Path) -> Option<(String, Option<String>)> {
    if !path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("resx")) {
        return None;
    }
    let stem = path.file_stem()?.to_string_lossy().into_owned();
    Some(match stem.rsplit_once('.') {
        Some((base, suffix)) if is_culture(suffix) => (base.to_string(), Some(suffix.to_string())),
        _ => (stem, None),
    })
}

/// Whether `path` belongs to the same culture family as `other` (same folder and base name).
pub fn same_family(path: &Path, other: &Path) -> bool {
    path.parent() == other.parent()
        && match (base_and_culture(path), base_and_culture(other)) {
            (Some((a, _)), Some((b, _))) => a == b,
            _ => false,
        }
}

impl ResourceGroup {
    /// Reads `selected` and its sibling culture files; the neutral file comes first.
    pub fn load(selected: &Path) -> Result<Self> {
        let selected = dunce::canonicalize(selected).with_context(|| format!("cannot open {}", selected.display()))?;
        let dir = selected.parent().context("resource file has no parent directory")?;
        let (base, _) = base_and_culture(&selected).context("not a .resx file")?;
        let mut paths = Vec::new();
        for entry in std::fs::read_dir(dir).with_context(|| format!("cannot list {}", dir.display()))? {
            let path = entry?.path();
            if !path.is_file() {
                continue;
            }
            if let Some((candidate, culture)) = base_and_culture(&path)
                && candidate == base
            {
                paths.push((culture, path));
            }
        }
        paths.sort_by(|a, b| a.0.cmp(&b.0));
        ensure!(!paths.is_empty(), "no resource files found for {}", selected.display());
        let files = paths.into_iter().map(|(culture, path)| ResourceFile::read(path, culture)).collect::<Result<Vec<_>>>()?;
        let rows = rows_of(&files);
        Ok(Self { selected_path: selected, files, rows })
    }

    /// Appends a key with an empty value in the first (neutral) column; returns its row.
    pub fn add_row(&mut self, name: impl Into<String>) -> usize {
        let mut cells = vec![None; self.files.len()];
        if let Some(first) = cells.first_mut() {
            *first = Some(ResourceCell::new(""));
        }
        self.rows.push(ResourceRow { original_name: None, name: name.into(), cells });
        self.rows.len() - 1
    }

    /// The column of the neutral file, if the group has one.
    pub fn neutral(&self) -> Option<usize> {
        self.files.iter().position(|file| file.culture.is_none())
    }

    /// Whether the files on disk are the ones this group was loaded from.
    pub fn same_files(&self, other: &ResourceGroup) -> bool {
        self.files.len() == other.files.len()
            && self.files.iter().zip(&other.files).all(|(a, b)| a.path == b.path && a.bom == b.bom && a.original == b.original)
    }

    /// Validates every edit before touching the disk, then writes (atomically, one by one) only
    /// the files whose XML changed. Refuses to overwrite a file modified outside this editor
    /// since it was loaded. On success the group reflects the saved files.
    pub fn save(&mut self) -> Result<Saved> {
        let mut names = BTreeSet::new();
        for row in &self.rows {
            ensure!(!row.name.trim().is_empty(), "resource key cannot be empty");
            ensure!(names.insert(row.name.as_str()), "duplicate resource key: {}", row.name);
            ensure!(row.cells.len() == self.files.len(), "resource column count changed");
        }
        let mut writes = Vec::new();
        for (column, file) in self.files.iter().enumerate() {
            let current = std::fs::read_to_string(&file.path).with_context(|| format!("cannot read {}", file.path.display()))?;
            ensure!(current == file.disk_text(&file.original), "{} changed on disk; reload it before saving", file.path.display());
            let edited = patch_file(file, &self.rows, column)?;
            if edited != file.original {
                writes.push((column, edited));
            }
        }
        let neutral = self.neutral();
        let before = neutral.map(|column| strings(&self.files[column]));
        let mut saved = Saved::default();
        for (column, text) in writes {
            let file = &mut self.files[column];
            let disk = file.disk_text(&text);
            rc_core::paths::write_atomic(&file.path, disk.as_bytes()).with_context(|| format!("cannot save {}", file.path.display()))?;
            let (entries, all_names) = parse_entries(&text)?;
            let previous = std::mem::replace(&mut file.original, text.clone());
            (file.entries, file.all_names) = (entries, all_names);
            saved.written.push((file.path.clone(), previous, text));
        }
        self.rows = rows_of(&self.files);
        if let (Some(column), Some(before)) = (neutral, before) {
            let file = &self.files[column];
            let after = strings(file);
            if after != before {
                let designer = file.path.with_extension("Designer.cs");
                saved.designer = Some(DesignerUpdate { path: designer, before, after });
            }
        }
        Ok(saved)
    }
}

fn strings(file: &ResourceFile) -> BTreeMap<String, String> {
    file.entries.iter().map(|(name, cell)| (name.clone(), cell.value.clone())).collect()
}

/// One row per string key of any file, sorted like Rider (case-insensitive).
fn rows_of(files: &[ResourceFile]) -> Vec<ResourceRow> {
    let mut names: Vec<&String> = files.iter().flat_map(|file| file.entries.keys()).collect::<BTreeSet<_>>().into_iter().collect();
    names.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()).then_with(|| a.cmp(b)));
    names
        .into_iter()
        .map(|name| ResourceRow { original_name: Some(name.clone()), name: name.clone(), cells: files.iter().map(|file| file.entries.get(name).cloned()).collect() })
        .collect()
}

fn is_culture(value: &str) -> bool {
    let mut parts = value.split('-');
    let Some(language) = parts.next() else {
        return false;
    };
    (2..=3).contains(&language.len()) && language.bytes().all(|b| b.is_ascii_alphabetic()) && parts.all(|part| (2..=8).contains(&part.len()) && part.bytes().all(|b| b.is_ascii_alphanumeric()))
}

/// A `<data>` entry is a string unless it names a type or MIME type (other than `System.String`).
fn is_string_entry(data: roxmltree::Node<'_, '_>) -> bool {
    data.attribute("mimetype").is_none() && data.attribute("type").is_none_or(|ty| ty.split(',').next().is_some_and(|ty| ty.trim() == "System.String"))
}

fn element_text(node: roxmltree::Node<'_, '_>) -> String {
    node.children().filter(|child| child.is_text()).filter_map(|child| child.text()).collect()
}

fn parse_entries(text: &str) -> Result<(BTreeMap<String, ResourceCell>, BTreeSet<String>)> {
    let document = roxmltree::Document::parse(text)?;
    ensure!(document.root_element().has_tag_name("root"), "expected a <root> element");
    let mut entries = BTreeMap::new();
    let mut names = BTreeSet::new();
    for data in document.root_element().children().filter(|node| node.has_tag_name("data")) {
        let Some(name) = data.attribute("name") else {
            continue;
        };
        ensure!(names.insert(name.to_string()), "duplicate resource key: {name}");
        if !is_string_entry(data) {
            continue;
        }
        let Some(value) = data.children().find(|node| node.has_tag_name("value")) else {
            continue;
        };
        if value.children().any(|node| node.is_element()) {
            continue;
        }
        let comment = data.children().find(|node| node.has_tag_name("comment")).map(element_text);
        entries.insert(name.to_string(), ResourceCell { value: element_text(value), comment });
    }
    Ok((entries, names))
}

struct Edit {
    range: Range<usize>,
    replacement: String,
}

fn patch_file(file: &ResourceFile, rows: &[ResourceRow], column: usize) -> Result<String> {
    let text = &file.original;
    let newline = line_ending(text);
    let document = roxmltree::Document::parse(text)?;
    let mut nodes = BTreeMap::new();
    for data in document.root_element().children().filter(|node| node.has_tag_name("data")) {
        if let Some(name) = data.attribute("name") {
            nodes.insert(name.to_string(), data);
        }
    }
    let mut edits = Vec::new();
    let mut additions = Vec::new();
    let retained: BTreeSet<_> = rows.iter().filter_map(|row| row.original_name.as_deref()).collect();
    for original in file.entries.keys() {
        if !retained.contains(original.as_str()) {
            edits.push(Edit { range: whole_line(text, nodes[original].range()), replacement: String::new() });
        }
    }
    for row in rows {
        let desired = &row.cells[column];
        let original = row.original_name.as_deref().and_then(|name| file.entries.get(name).map(|cell| (name, cell)));
        match (original, desired) {
            (None, None) => {}
            (None, Some(cell)) => {
                ensure!(!file.all_names.contains(&row.name) || row.original_name.as_deref() == Some(row.name.as_str()), "resource key conflicts with an existing entry: {}", row.name);
                additions.push((row.name.as_str(), cell));
            }
            (Some((name, _)), None) => edits.push(Edit { range: whole_line(text, nodes[name].range()), replacement: String::new() }),
            (Some((name, previous)), Some(cell)) => {
                let data = nodes[name];
                if row.name != name {
                    let attr = data.attributes().find(|attr| attr.name() == "name").context("resource key has no name attribute")?;
                    edits.push(Edit { range: attr.range_value(), replacement: xml_escape(&row.name, true, newline) });
                }
                if cell.value != previous.value {
                    let value = data.children().find(|node| node.has_tag_name("value")).context("resource has no value")?;
                    edits.push(element_content(text, value, &xml_escape(&cell.value, false, newline))?);
                }
                if cell.comment != previous.comment {
                    if let Some(comment) = data.children().find(|node| node.has_tag_name("comment")) {
                        match &cell.comment {
                            Some(value) => edits.push(element_content(text, comment, &xml_escape(value, false, newline))?),
                            None => edits.push(Edit { range: whole_line(text, comment.range()), replacement: String::new() }),
                        }
                    } else if let Some(value) = &cell.comment {
                        let closing = text[data.range()].rfind("</data>").context("resource has no closing data tag")? + data.range().start;
                        let line = line_start(text, closing);
                        let on_own_line = text[line..closing].trim().is_empty();
                        let value_indent = data.children().find(|node| node.has_tag_name("value")).map(|node| indent_of(text, node.range().start)).unwrap_or("    ");
                        let comment = format!("<comment>{}</comment>", xml_escape(value, false, newline));
                        let (at, replacement) = if on_own_line { (line, format!("{value_indent}{comment}{newline}")) } else { (closing, comment) };
                        edits.push(Edit { range: at..at, replacement });
                    }
                }
            }
        }
    }
    if !additions.is_empty() {
        let root = document.root_element();
        let closing = text[root.range()].rfind("</root>").context("resource has no closing root tag")? + root.range().start;
        let line = line_start(text, closing);
        let on_own_line = text[line..closing].trim().is_empty();
        let at = if on_own_line { line } else { closing };
        // New entries copy the indentation of the last existing one (Visual Studio: two spaces).
        let (data_indent, value_indent) = nodes
            .values()
            .max_by_key(|node| node.range().start)
            .map(|data| {
                let outer = indent_of(text, data.range().start);
                let inner = data.children().find(|node| node.has_tag_name("value")).map(|node| indent_of(text, node.range().start)).filter(|inner| inner.len() > outer.len());
                (outer, inner.unwrap_or("    "))
            })
            .unwrap_or(("  ", "    "));
        let mut inserted = if on_own_line { String::new() } else { newline.to_string() };
        for (name, cell) in additions {
            inserted.push_str(&format!("{data_indent}<data name=\"{}\" xml:space=\"preserve\">{newline}{value_indent}<value>{}</value>{newline}", xml_escape(name, true, newline), xml_escape(&cell.value, false, newline)));
            if let Some(comment) = &cell.comment {
                inserted.push_str(&format!("{value_indent}<comment>{}</comment>{newline}", xml_escape(comment, false, newline)));
            }
            inserted.push_str(&format!("{data_indent}</data>{newline}"));
        }
        edits.push(Edit { range: at..at, replacement: inserted });
    }
    let result = apply(text, edits)?;
    parse_entries(&result).context("edited resource XML is invalid")?;
    Ok(result)
}

/// Applies non-overlapping edits from the end of the text backwards.
fn apply(text: &str, mut edits: Vec<Edit>) -> Result<String> {
    edits.sort_by(|a, b| b.range.start.cmp(&a.range.start).then_with(|| b.range.end.cmp(&a.range.end)));
    let mut result = text.to_string();
    let mut limit = text.len();
    for edit in edits {
        ensure!(edit.range.end <= limit, "overlapping resource edits");
        limit = edit.range.start;
        result.replace_range(edit.range, &edit.replacement);
    }
    Ok(result)
}

fn line_start(text: &str, offset: usize) -> usize {
    text[..offset].rfind('\n').map_or(0, |index| index + 1)
}

/// The whitespace before `offset` on its line, when nothing else precedes it.
fn indent_of(text: &str, offset: usize) -> &str {
    let start = line_start(text, offset);
    let indent = &text[start..offset];
    if indent.trim().is_empty() { indent } else { "" }
}

/// `range` widened to its whole line when the element is alone on it.
fn whole_line(text: &str, range: Range<usize>) -> Range<usize> {
    let start = line_start(text, range.start);
    if !text[start..range.start].trim().is_empty() {
        return range;
    }
    let mut end = range.end;
    if text[end..].starts_with("\r\n") {
        end += 2;
    } else if text[end..].starts_with('\n') {
        end += 1;
    } else {
        return range;
    }
    start..end
}

fn line_ending(text: &str) -> &'static str {
    if text.contains("\r\n") { "\r\n" } else { "\n" }
}

/// Replaces everything between the start and end tags of `node` (text, CDATA, entities) with
/// `replacement`, already escaped.
fn element_content(text: &str, node: roxmltree::Node<'_, '_>, replacement: &str) -> Result<Edit> {
    ensure!(!node.children().any(|child| child.is_element()), "resource text contains nested XML");
    let range = node.range();
    let name = node.tag_name().name();
    if text[range.clone()].ends_with("/>") {
        return Ok(Edit { range, replacement: format!("<{name}>{replacement}</{name}>") });
    }
    let end = text[range.clone()].rfind("</").context("resource element has no closing tag")? + range.start;
    let start = node.first_child().map_or(end, |child| child.range().start);
    Ok(Edit { range: start..end, replacement: replacement.to_string() })
}

fn xml_escape(value: &str, attribute: bool, newline: &str) -> String {
    let mut escaped = value.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    if attribute {
        escaped = escaped.replace('"', "&quot;").replace('\'', "&apos;").replace('\n', "&#xA;");
    } else if newline != "\n" {
        escaped = escaped.replace("\r\n", "\n").replace('\n', newline);
    }
    escaped
}

// ------------------------------------------------------------------ Designer.cs

/// A member of the generated class: its whole block (doc comment to closing brace).
struct Member {
    range: Range<usize>,
    /// The blank line before the block, removed with it.
    blank_before: Option<Range<usize>>,
    name: String,
    /// The resource key a string property reads (`GetString("Key", …)`).
    key: Option<String>,
    /// The lines between `/// <summary>` and `/// </summary>`.
    summary: Option<Range<usize>>,
}

/// Edits that keep a `ResXFileCodeGenerator` class (`Strings.Designer.cs`, text in `text`) in
/// step with the string keys of its neutral `.resx`: properties for removed keys go, new keys get
/// one in alphabetical order, changed values refresh the doc comment. Typed resources and every
/// other line stay as they are. Files not written by the generator get no edits.
pub fn designer_edits(text: &str, before: &BTreeMap<String, String>, after: &BTreeMap<String, String>) -> Vec<(Range<usize>, String)> {
    if !text.contains("StronglyTypedResourceBuilder") {
        return Vec::new();
    }
    let newline = line_ending(text);
    let lines = lines_with_offsets(text);
    // The `Culture` property tells the member indentation and the class's accessibility.
    let Some(culture) = lines.iter().find(|(_, line)| line.contains(" static ") && line.trim_end().ends_with(" Culture {")) else {
        return Vec::new();
    };
    let indent = &culture.1[..culture.1.len() - culture.1.trim_start().len()];
    let access = culture.1.trim_start().split(" static ").next().unwrap_or("internal").to_string();
    let step = if indent.contains('\t') { "\t" } else { "    " };
    let members = members(text, &lines, indent);
    let blank = members.iter().find_map(|member| member.blank_before.clone()).map_or_else(|| newline.to_string(), |range| text[range].to_string());
    let mut edits = Vec::new();
    let removed: BTreeSet<&str> = before.keys().filter(|key| !after.contains_key(*key)).map(String::as_str).collect();
    let mut kept = Vec::new();
    for member in &members {
        match member.key.as_deref() {
            Some(key) if removed.contains(key) => {
                let start = member.blank_before.as_ref().map_or(member.range.start, |blank| blank.start);
                edits.push((start..member.range.end, String::new()));
            }
            Some(key) => {
                if let (Some(old), Some(new), Some(summary)) = (before.get(key), after.get(key), member.summary.clone())
                    && old != new
                {
                    edits.push((summary, summary_lines(indent, new, newline)));
                }
                kept.push(member);
            }
            None => kept.push(member),
        }
    }
    let mut taken: BTreeSet<String> = kept.iter().map(|member| member.name.clone()).collect();
    let present: BTreeSet<&str> = kept.iter().filter_map(|member| member.key.as_deref()).collect();
    let mut insertions: BTreeMap<usize, String> = BTreeMap::new();
    let mut added: Vec<(String, &String, &String)> = after
        .iter()
        .filter(|(key, _)| !before.contains_key(*key) && !present.contains(key.as_str()))
        .filter_map(|(key, value)| Some((identifier(key)?, key, value)))
        .collect();
    added.sort_by_key(|(name, _, _)| name.trim_start_matches('@').to_lowercase());
    for (name, key, value) in added {
        if !taken.insert(name.clone()) {
            continue;
        }
        let block = property(indent, step, &access, &name, key, value, newline);
        let order = name.trim_start_matches('@').to_lowercase();
        let next = kept.iter().filter(|member| member.name != "ResourceManager" && member.name != "Culture").find(|member| member.name.trim_start_matches('@').to_lowercase() > order);
        match next {
            Some(member) => insertions.entry(member.range.start).or_default().push_str(&format!("{block}{blank}")),
            None => {
                let Some(last) = kept.last() else { continue };
                insertions.entry(last.range.end).or_default().push_str(&format!("{blank}{block}"));
            }
        }
    }
    edits.extend(insertions.into_iter().map(|(at, text)| (at..at, text)));
    edits
}

fn lines_with_offsets(text: &str) -> Vec<(usize, &str)> {
    let mut offset = 0;
    text.split_inclusive('\n')
        .map(|line| {
            let start = offset;
            offset += line.len();
            (start, line)
        })
        .collect()
}

fn members(text: &str, lines: &[(usize, &str)], indent: &str) -> Vec<Member> {
    let doc = format!("{indent}///");
    let close = format!("{indent}}}");
    let mut members = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let (start, line) = lines[i];
        let previous_is_doc = i > 0 && lines[i - 1].1.starts_with(&doc);
        if !line.starts_with(&doc) || previous_is_doc {
            i += 1;
            continue;
        }
        let blank_before = (i > 0 && lines[i - 1].1.trim().is_empty()).then(|| lines[i - 1].0..start);
        let mut summary = None;
        let mut j = i;
        while j < lines.len() && lines[j].1.starts_with(&doc) {
            if lines[j].1.trim() == "/// <summary>" {
                let first = j + 1;
                let mut k = first;
                while k < lines.len() && lines[k].1.starts_with(&doc) && lines[k].1.trim() != "/// </summary>" {
                    k += 1;
                }
                if k < lines.len() && lines[k].1.trim() == "/// </summary>" {
                    summary = Some(lines[first].0..lines[k].0);
                }
            }
            j += 1;
        }
        // Attributes, then the declaration.
        while j < lines.len() && lines[j].1.trim_start().starts_with('[') {
            j += 1;
        }
        let Some(&(_, declaration)) = lines.get(j) else { break };
        let name = declaration.trim().trim_end_matches('{').trim().rsplit(' ').next().unwrap_or_default().trim_end_matches("()").to_string();
        let mut k = j + 1;
        while k < lines.len() && lines[k].1.trim_end() != close {
            k += 1;
        }
        let Some(&(end_line, end_text)) = lines.get(k) else { break };
        let range = start..end_line + end_text.len();
        let key = text[range.clone()].split_once("GetString(\"").and_then(|(_, rest)| csharp_string(rest));
        members.push(Member { range, blank_before, name, key, summary });
        i = k + 1;
    }
    members
}

/// The C# regular string literal at the start of `rest` (after its opening quote), unescaped.
fn csharp_string(rest: &str) -> Option<String> {
    let mut out = String::new();
    let mut chars = rest.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(out),
            '\\' => out.push(match chars.next()? {
                'n' => '\n',
                'r' => '\r',
                't' => '\t',
                '0' => '\0',
                other => other,
            }),
            c => out.push(c),
        }
    }
    None
}

fn csharp_literal(value: &str) -> String {
    let mut out = String::new();
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\0' => out.push_str("\\0"),
            c => out.push(c),
        }
    }
    out
}

/// The property name `ResXFileCodeGenerator` gives a key, or `None` when it skips the key.
fn identifier(key: &str) -> Option<String> {
    if key == "ResourceManager" || key == "Culture" || key.starts_with(">>") || key.starts_with("$this") {
        return None;
    }
    let mut name: String = key.chars().map(|c| if c.is_alphanumeric() || c == '_' { c } else { '_' }).collect();
    if name.is_empty() {
        return None;
    }
    if name.starts_with(|c: char| c.is_ascii_digit()) {
        name.insert(0, '_');
    }
    if CSHARP_KEYWORDS.contains(&name.as_str()) {
        name.insert(0, '@');
    }
    Some(name)
}

const CSHARP_KEYWORDS: &[&str] = &[
    "abstract", "as", "base", "bool", "break", "byte", "case", "catch", "char", "checked", "class", "const", "continue", "decimal", "default", "delegate", "do", "double", "else", "enum", "event", "explicit", "extern", "false", "finally", "fixed", "float", "for", "foreach", "goto", "if", "implicit", "in", "int", "interface", "internal", "is", "lock", "long", "namespace", "new", "null", "object", "operator", "out", "override", "params", "private", "protected", "public", "readonly", "ref", "return", "sbyte", "sealed", "short", "sizeof", "stackalloc", "static", "string", "struct", "switch", "this", "throw", "true", "try", "typeof", "uint", "ulong", "unchecked", "unsafe", "ushort", "using", "virtual", "void", "volatile", "while",
];

/// The doc comment lines of a string property, like the generator writes them.
fn summary_lines(indent: &str, value: &str, newline: &str) -> String {
    const LIMIT: usize = 512;
    let mut shown: String = value.chars().take(LIMIT).collect();
    if value.chars().count() > LIMIT {
        shown.push_str(" [rest of string was truncated]");
    }
    let shown = shown.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace("\r\n", "\n");
    let text = format!("Looks up a localized string similar to {shown}.");
    let mut out = String::new();
    for (index, line) in text.split('\n').enumerate() {
        if index == 0 {
            out.push_str(&format!("{indent}///   {line}{newline}"));
        } else {
            out.push_str(&format!("{indent}///{line}{newline}"));
        }
    }
    out
}

fn property(indent: &str, step: &str, access: &str, name: &str, key: &str, value: &str, newline: &str) -> String {
    let mut out = format!("{indent}/// <summary>{newline}");
    out.push_str(&summary_lines(indent, value, newline));
    out.push_str(&format!("{indent}/// </summary>{newline}"));
    out.push_str(&format!("{indent}{access} static string {name} {{{newline}"));
    out.push_str(&format!("{indent}{step}get {{{newline}"));
    out.push_str(&format!("{indent}{step}{step}return ResourceManager.GetString(\"{}\", resourceCulture);{newline}", csharp_literal(key)));
    out.push_str(&format!("{indent}{step}}}{newline}"));
    out.push_str(&format!("{indent}}}{newline}"));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_across_cultures_keep_headers_comments_and_non_string_data() {
        let dir = tempfile::tempdir().unwrap();
        let neutral = dir.path().join("Messages.resx");
        let french = dir.path().join("Messages.fr.resx");
        let header = "<?xml version=\"1.0\"?><root>\n  <!-- keep this header -->\n";
        std::fs::write(&neutral, format!("{header}  <data name=\"Hello\"><value>Hello</value><comment>Greeting</comment></data>\n  <data name=\"Binary\" type=\"System.Byte[]\"><value>AA==</value></data>\n</root>\n")).unwrap();
        std::fs::write(&french, format!("{header}  <data name=\"Hello\"><value>Bonjour</value></data>\n</root>\n")).unwrap();
        let mut group = ResourceGroup::load(&french).unwrap();
        assert_eq!(group.files.len(), 2);
        assert_eq!(group.files[0].culture, None);
        assert_eq!(group.files[0].typed_entries(), 1);
        group.rows[0].name = "Welcome".into();
        group.rows[0].cells[1].as_mut().unwrap().value = "Salut & bonjour".into();
        let added = group.add_row("Bye");
        group.rows[added].cells[0].as_mut().unwrap().value = "Goodbye".into();
        group.rows[added].cells[1] = Some(ResourceCell::new("Au revoir"));
        let saved = group.save().unwrap();
        assert_eq!(saved.written.len(), 2);
        assert!(group.rows.iter().any(|row| row.name == "Welcome" && row.original_name.as_deref() == Some("Welcome")));
        let neutral_text = std::fs::read_to_string(&neutral).unwrap();
        let french_text = std::fs::read_to_string(&french).unwrap();
        assert!(neutral_text.contains("<!-- keep this header -->"));
        assert!(neutral_text.contains("type=\"System.Byte[]\""));
        assert!(neutral_text.contains("<comment>Greeting</comment>"));
        assert!(neutral_text.contains("name=\"Welcome\""));
        assert!(french_text.contains("Salut &amp; bonjour"));
        assert!(french_text.contains("name=\"Bye\""));
        let designer = saved.designer.unwrap();
        assert_eq!(designer.path, dir.path().canonicalize().unwrap().join("Messages.Designer.cs"));
        assert!(designer.before.contains_key("Hello") && designer.after.contains_key("Welcome") && designer.after.contains_key("Bye"));
    }

    #[test]
    fn changed_file_blocks_save_before_any_write() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("Strings.resx");
        std::fs::write(&file, "<root>\n  <data name=\"A\"><value>x</value></data>\n</root>\n").unwrap();
        let mut group = ResourceGroup::load(&file).unwrap();
        group.rows[0].cells[0].as_mut().unwrap().value = "y".into();
        std::fs::write(&file, "<root>\n  <data name=\"A\"><value>external</value></data>\n</root>\n").unwrap();
        assert!(group.save().is_err());
        assert!(std::fs::read_to_string(&file).unwrap().contains("external"));
    }

    #[test]
    fn inline_root_and_comment_insertions_remain_inside_their_elements() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("Strings.resx");
        std::fs::write(&file, "<root><data name='A'><value>x</value></data></root>").unwrap();
        let mut group = ResourceGroup::load(&file).unwrap();
        group.rows[0].cells[0].as_mut().unwrap().comment = Some("hint".into());
        let added = group.add_row("B");
        group.rows[added].cells[0].as_mut().unwrap().value = "y".into();
        group.save().unwrap();
        let text = std::fs::read_to_string(file).unwrap();
        assert!(text.starts_with("<root><data"));
        assert!(text.contains("<comment>hint</comment></data>"));
        assert!(text.contains("name=\"B\""));
        assert!(text.ends_with("</root>"));
    }

    #[test]
    fn removing_a_key_drops_it_from_every_culture() {
        let dir = tempfile::tempdir().unwrap();
        let neutral = dir.path().join("Strings.resx");
        let german = dir.path().join("Strings.de.resx");
        for file in [&neutral, &german] {
            std::fs::write(file, "<root>\n  <data name=\"Keep\"><value>yes</value></data>\n  <data name=\"Remove\"><value>no</value></data>\n</root>\n").unwrap();
        }
        let mut group = ResourceGroup::load(&neutral).unwrap();
        group.rows.retain(|row| row.name != "Remove");
        group.save().unwrap();
        for file in [&neutral, &german] {
            let text = std::fs::read_to_string(file).unwrap();
            assert_eq!(text, "<root>\n  <data name=\"Keep\"><value>yes</value></data>\n</root>\n");
        }
    }

    /// A file as Visual Studio writes it: BOM, CRLF, the schema, resheaders, an assembly alias,
    /// metadata, a file reference, a base64 image, `xml:space` and comments.
    fn visual_studio_resx(hello: &str) -> String {
        let text = format!(
            r#"<?xml version="1.0" encoding="utf-8"?>
<root>
  <!--
    Microsoft ResX Schema
    Version 2.0
  -->
  <xsd:schema id="root" xmlns="" xmlns:xsd="http://www.w3.org/2001/XMLSchema" xmlns:msdata="urn:schemas-microsoft-com:xml-msdata">
    <xsd:import namespace="http://www.w3.org/XML/1998/namespace" />
    <xsd:element name="root" msdata:IsDataSet="true">
      <xsd:complexType>
        <xsd:choice maxOccurs="unbounded">
          <xsd:element name="data">
            <xsd:complexType>
              <xsd:sequence>
                <xsd:element name="value" type="xsd:string" minOccurs="0" msdata:Ordinal="1" />
                <xsd:element name="comment" type="xsd:string" minOccurs="0" msdata:Ordinal="2" />
              </xsd:sequence>
              <xsd:attribute name="name" type="xsd:string" use="required" msdata:Ordinal="1" />
              <xsd:attribute ref="xml:space" />
            </xsd:complexType>
          </xsd:element>
        </xsd:choice>
      </xsd:complexType>
    </xsd:element>
  </xsd:schema>
  <resheader name="resmimetype">
    <value>text/microsoft-resx</value>
  </resheader>
  <resheader name="version">
    <value>2.0</value>
  </resheader>
  <resheader name="reader">
    <value>System.Resources.ResXResourceReader, System.Windows.Forms, Version=4.0.0.0, Culture=neutral, PublicKeyToken=b77a5c561934e089</value>
  </resheader>
  <resheader name="writer">
    <value>System.Resources.ResXResourceWriter, System.Windows.Forms, Version=4.0.0.0, Culture=neutral, PublicKeyToken=b77a5c561934e089</value>
  </resheader>
  <assembly alias="System.Windows.Forms" name="System.Windows.Forms, Version=4.0.0.0, Culture=neutral, PublicKeyToken=b77a5c561934e089" />
  <metadata name="toolTip1.TrayLocation" type="System.Drawing.Point, System.Drawing">
    <value>17, 17</value>
  </metadata>
  <data name="Logo" type="System.Resources.ResXFileRef, System.Windows.Forms">
    <value>..\Resources\logo.png;System.Drawing.Bitmap, System.Drawing, Version=4.0.0.0, Culture=neutral, PublicKeyToken=b03f5f7f11d50a3a</value>
  </data>
  <data name="Icon" type="System.Drawing.Bitmap, System.Drawing" mimetype="application/x-microsoft.net.object.bytearray.base64">
    <value>
        iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==
</value>
  </data>
  <data name="Hello" xml:space="preserve">
    <value>{hello}</value>
    <comment>Shown on the home page</comment>
  </data>
  <data name="Spaces" xml:space="preserve">
    <value>  padded  </value>
  </data>
  <data name="Multi" xml:space="preserve">
    <value>line one
line two</value>
  </data>
</root>"#
        );
        format!("\u{FEFF}{}", text.replace('\n', "\r\n"))
    }

    #[test]
    fn visual_studio_file_round_trips_byte_for_byte() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("Strings.resx");
        std::fs::write(&file, visual_studio_resx("Hello")).unwrap();
        let mut group = ResourceGroup::load(&file).unwrap();
        let names: Vec<_> = group.rows.iter().map(|row| row.name.as_str()).collect();
        assert_eq!(names, ["Hello", "Multi", "Spaces"], "typed entries, metadata and headers are not string rows");
        assert_eq!(group.files[0].typed_entries(), 2);
        let multi = group.rows.iter().find(|row| row.name == "Multi").unwrap();
        assert_eq!(multi.cells[0].as_ref().unwrap().value, "line one\nline two");
        let spaces = group.rows.iter().find(|row| row.name == "Spaces").unwrap();
        assert_eq!(spaces.cells[0].as_ref().unwrap().value, "  padded  ");
        // Nothing edited: nothing written.
        assert!(group.save().unwrap().written.is_empty());
        // One value edited: only its span changes; BOM, CRLF and everything else stay.
        group.rows[0].cells[0].as_mut().unwrap().value = "Hi <there> & \"you\"".into();
        group.save().unwrap();
        let bytes = std::fs::read(&file).unwrap();
        assert_eq!(String::from_utf8(bytes).unwrap(), visual_studio_resx("Hi &lt;there&gt; &amp; \"you\""));
    }

    #[test]
    fn multi_line_values_use_the_file_line_endings() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("Strings.resx");
        std::fs::write(&file, visual_studio_resx("Hello")).unwrap();
        let mut group = ResourceGroup::load(&file).unwrap();
        group.rows[0].cells[0].as_mut().unwrap().value = "a\nb".into();
        let added = group.add_row("Zeta");
        group.rows[added].cells[0].as_mut().unwrap().value = "z".into();
        group.save().unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.contains("<value>a\r\nb</value>"));
        assert!(text.contains("  <data name=\"Zeta\" xml:space=\"preserve\">\r\n    <value>z</value>\r\n  </data>\r\n</root>"));
        assert!(!text.replace("\r\n", "").contains('\n'), "no bare LF in a CRLF file");
        assert_eq!(ResourceGroup::load(&file).unwrap().rows[0].cells[0].as_ref().unwrap().value, "a\nb");
    }

    #[test]
    fn cdata_and_entity_values_are_replaced_whole() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("Strings.resx");
        std::fs::write(&file, "<root>\n  <data name=\"A\"><value>x &amp; <![CDATA[<y>]]> z</value></data>\n  <data name=\"B\"><value><![CDATA[only]]></value></data>\n</root>\n").unwrap();
        let mut group = ResourceGroup::load(&file).unwrap();
        assert_eq!(group.rows[0].cells[0].as_ref().unwrap().value, "x & <y> z");
        assert_eq!(group.rows[1].cells[0].as_ref().unwrap().value, "only");
        group.rows[0].cells[0].as_mut().unwrap().value = "new".into();
        group.rows[1].cells[0].as_mut().unwrap().value = "other".into();
        group.save().unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "<root>\n  <data name=\"A\"><value>new</value></data>\n  <data name=\"B\"><value>other</value></data>\n</root>\n");
    }

    #[test]
    fn renaming_onto_a_typed_entry_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("Strings.resx");
        let text = "<root>\n  <data name=\"A\"><value>x</value></data>\n  <data name=\"Img\" type=\"System.Byte[]\"><value>AA==</value></data>\n</root>\n";
        std::fs::write(&file, text).unwrap();
        let mut group = ResourceGroup::load(&file).unwrap();
        group.rows[0].name = "Img".into();
        assert!(group.save().is_err());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), text);
    }

    #[test]
    fn culture_files_are_grouped_by_base_name() {
        assert_eq!(base_and_culture(Path::new("/p/Strings.es-MX.resx")), Some(("Strings".into(), Some("es-MX".into()))));
        assert_eq!(base_and_culture(Path::new("/p/Strings.resx")), Some(("Strings".into(), None)));
        assert_eq!(base_and_culture(Path::new("/p/Form1.Designer.resx")), Some(("Form1.Designer".into(), None)));
        assert!(same_family(Path::new("/p/Strings.resx"), Path::new("/p/Strings.fr.resx")));
        assert!(!same_family(Path::new("/p/Strings.resx"), Path::new("/p/Other.fr.resx")));
        assert!(!same_family(Path::new("/p/Strings.resx"), Path::new("/q/Strings.fr.resx")));
    }

    fn designer(members: &str) -> String {
        let text = format!(
            r#"//------------------------------------------------------------------------------
// <auto-generated>
//     This code was generated by a tool.
// </auto-generated>
//------------------------------------------------------------------------------

namespace App.Properties {{
    using System;


    /// <summary>
    ///   A strongly-typed resource class, for looking up localized strings, etc.
    /// </summary>
    [global::System.CodeDom.Compiler.GeneratedCodeAttribute("System.Resources.Tools.StronglyTypedResourceBuilder", "17.0.0.0")]
    [global::System.Diagnostics.DebuggerNonUserCodeAttribute()]
    [global::System.Runtime.CompilerServices.CompilerGeneratedAttribute()]
    public class Strings {{

        private static global::System.Resources.ResourceManager resourceMan;

        private static global::System.Globalization.CultureInfo resourceCulture;

        [global::System.Diagnostics.CodeAnalysis.SuppressMessageAttribute("Microsoft.Performance", "CA1811:AvoidUncalledPrivateCode")]
        internal Strings() {{
        }}

        /// <summary>
        ///   Returns the cached ResourceManager instance used by this class.
        /// </summary>
        [global::System.ComponentModel.EditorBrowsableAttribute(global::System.ComponentModel.EditorBrowsableState.Advanced)]
        public static global::System.Resources.ResourceManager ResourceManager {{
            get {{
                if (object.ReferenceEquals(resourceMan, null)) {{
                    global::System.Resources.ResourceManager temp = new global::System.Resources.ResourceManager("App.Properties.Strings", typeof(Strings).Assembly);
                    resourceMan = temp;
                }}
                return resourceMan;
            }}
        }}

        /// <summary>
        ///   Overrides the current thread's CurrentUICulture property for all
        ///   resource lookups using this strongly typed resource class.
        /// </summary>
        [global::System.ComponentModel.EditorBrowsableAttribute(global::System.ComponentModel.EditorBrowsableState.Advanced)]
        public static global::System.Globalization.CultureInfo Culture {{
            get {{
                return resourceCulture;
            }}
            set {{
                resourceCulture = value;
            }}
        }}
{members}    }}
}}
"#
        );
        text.replace('\n', "\r\n")
    }

    fn string_property(name: &str, value: &str) -> String {
        format!(
            "\n        /// <summary>\n        ///   Looks up a localized string similar to {value}.\n        /// </summary>\n        public static string {name} {{\n            get {{\n                return ResourceManager.GetString(\"{name}\", resourceCulture);\n            }}\n        }}\n"
        )
    }

    const LOGO: &str = "\n        /// <summary>\n        ///   Looks up a localized resource of type System.Drawing.Bitmap.\n        /// </summary>\n        public static System.Drawing.Bitmap Logo {\n            get {\n                object obj = ResourceManager.GetObject(\"Logo\", resourceCulture);\n                return ((System.Drawing.Bitmap)(obj));\n            }\n        }\n";

    fn apply_designer(text: &str, before: &[(&str, &str)], after: &[(&str, &str)]) -> String {
        let map = |pairs: &[(&str, &str)]| pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<BTreeMap<_, _>>();
        let edits = designer_edits(text, &map(before), &map(after)).into_iter().map(|(range, replacement)| Edit { range, replacement }).collect();
        apply(text, edits).unwrap()
    }

    #[test]
    fn designer_gains_loses_and_refreshes_string_properties() {
        let original = designer(&format!("{}{LOGO}{}", string_property("Hello", "Hello"), string_property("Zebra", "Stripes")));
        let before = [("Hello", "Hello"), ("Zebra", "Stripes")];
        let after = [("Apple", "Red & <b>"), ("Hello", "Hi"), ("Mango", "Sweet")];
        let text = apply_designer(&original, &before, &after);
        let expected = designer(&format!(
            "{}{}{LOGO}{}",
            string_property("Apple", "Red &amp; &lt;b&gt;"),
            string_property("Hello", "Hi"),
            string_property("Mango", "Sweet")
        ));
        assert_eq!(text, expected);
        // Back again: the removed ones go and the original order returns.
        let back = apply_designer(&text, &after, &before);
        assert_eq!(back, original);
    }

    #[test]
    fn designer_appends_after_the_culture_property_when_empty() {
        let original = designer("");
        let text = apply_designer(&original, &[], &[("1st key", "One"), ("class", "Keyword")]);
        let expected = designer(&format!("{}{}", string_property("_1st_key", "One").replace("GetString(\"_1st_key\"", "GetString(\"1st key\""), string_property("@class", "Keyword").replace("GetString(\"@class\"", "GetString(\"class\"")));
        assert_eq!(text, expected);
    }

    #[test]
    fn designer_edits_skip_hand_written_files() {
        let text = "namespace A { public class Strings { public static string Hello => \"x\"; } }";
        assert!(designer_edits(text, &BTreeMap::new(), &BTreeMap::from([("New".to_string(), "v".to_string())])).is_empty());
    }
}
