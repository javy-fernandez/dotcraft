//! String resources from sibling `.resx` culture files, edited without reserializing XML.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResourceCell {
    pub value: String,
    pub comment: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ResourceFile {
    pub path: PathBuf,
    pub culture: Option<String>,
    original: String,
    entries: BTreeMap<String, ResourceCell>,
    all_names: BTreeSet<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResourceRow {
    pub original_name: Option<String>,
    pub name: String,
    pub cells: Vec<Option<ResourceCell>>,
}

#[derive(Clone, Debug)]
pub struct ResourceGroup {
    pub selected_path: PathBuf,
    pub files: Vec<ResourceFile>,
    pub rows: Vec<ResourceRow>,
}

impl ResourceGroup {
    /// Read only sibling culture files with the same resource base name.
    pub fn load(selected: &Path) -> Result<Self> {
        let selected = dunce::canonicalize(selected)
            .with_context(|| format!("cannot open {}", selected.display()))?;
        let dir = selected
            .parent()
            .context("resource file has no parent directory")?;
        let stem = selected
            .file_stem()
            .context("resource file has no stem")?
            .to_string_lossy();
        let base = stem
            .rsplit_once('.')
            .filter(|(_, suffix)| is_culture(suffix))
            .map_or(stem.as_ref(), |(base, _)| base);
        let mut paths = Vec::new();
        for entry in
            std::fs::read_dir(dir).with_context(|| format!("cannot list {}", dir.display()))?
        {
            let path = entry?.path();
            if !path.is_file()
                || !path
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("resx"))
            {
                continue;
            }
            let Some(candidate) = path.file_stem().map(|stem| stem.to_string_lossy()) else {
                continue;
            };
            let culture = if candidate == base {
                None
            } else if let Some(suffix) = candidate
                .strip_prefix(base)
                .and_then(|rest| rest.strip_prefix('.'))
                .filter(|suffix| is_culture(suffix))
            {
                Some(suffix.to_string())
            } else {
                continue;
            };
            paths.push((culture, path));
        }
        paths.sort_by(|a, b| a.0.cmp(&b.0));
        ensure!(
            !paths.is_empty(),
            "no resource files found for {}",
            selected.display()
        );
        let mut files = Vec::new();
        for (culture, path) in paths {
            let text = std::fs::read_to_string(&path)
                .with_context(|| format!("cannot read {}", path.display()))?;
            let (entries, all_names) = parse_entries(&text)
                .with_context(|| format!("invalid resource XML in {}", path.display()))?;
            files.push(ResourceFile {
                path,
                culture,
                original: text,
                entries,
                all_names,
            });
        }
        let mut names = BTreeSet::new();
        for file in &files {
            names.extend(file.entries.keys().cloned());
        }
        let rows = names
            .into_iter()
            .map(|name| ResourceRow {
                original_name: Some(name.clone()),
                cells: files
                    .iter()
                    .map(|file| file.entries.get(&name).cloned())
                    .collect(),
                name,
            })
            .collect();
        Ok(Self {
            selected_path: selected,
            files,
            rows,
        })
    }

    pub fn add_row(&mut self) {
        let mut cells = vec![None; self.files.len()];
        if let Some(first) = cells.first_mut() {
            *first = Some(ResourceCell {
                value: String::new(),
                comment: None,
            });
        }
        self.rows.push(ResourceRow {
            original_name: None,
            name: String::new(),
            cells,
        });
    }

    /// Validate every edit before touching disk, then write only files whose XML changed.
    /// Refuse to overwrite a file modified outside this editor since the group was loaded.
    pub fn save(&self) -> Result<Self> {
        let mut names = BTreeSet::new();
        for row in &self.rows {
            ensure!(!row.name.trim().is_empty(), "resource key cannot be empty");
            ensure!(
                names.insert(row.name.clone()),
                "duplicate resource key: {}",
                row.name
            );
            ensure!(
                row.cells.len() == self.files.len(),
                "resource column count changed"
            );
        }
        let mut writes = Vec::new();
        for (column, file) in self.files.iter().enumerate() {
            let current = std::fs::read_to_string(&file.path)
                .with_context(|| format!("cannot read {}", file.path.display()))?;
            ensure!(
                current == file.original,
                "{} changed on disk; refresh before saving",
                file.path.display()
            );
            let edited = patch_file(file, &self.rows, column)?;
            if edited != file.original {
                writes.push((file.path.clone(), edited));
            }
        }
        for (path, text) in writes {
            rc_core::paths::write_atomic(&path, text.as_bytes())
                .with_context(|| format!("cannot save {}", path.display()))?;
        }
        Self::load(&self.selected_path)
    }
}

fn is_culture(value: &str) -> bool {
    let mut parts = value.split('-');
    let Some(language) = parts.next() else {
        return false;
    };
    (2..=3).contains(&language.len())
        && language.bytes().all(|b| b.is_ascii_alphabetic())
        && parts.all(|part| {
            (2..=8).contains(&part.len()) && part.bytes().all(|b| b.is_ascii_alphanumeric())
        })
}

fn parse_entries(text: &str) -> Result<(BTreeMap<String, ResourceCell>, BTreeSet<String>)> {
    let document = roxmltree::Document::parse(text)?;
    ensure!(
        document.root_element().has_tag_name("root"),
        "expected a <root> element"
    );
    let mut entries = BTreeMap::new();
    let mut names = BTreeSet::new();
    for data in document
        .root_element()
        .children()
        .filter(|node| node.has_tag_name("data"))
    {
        let Some(name) = data.attribute("name") else {
            continue;
        };
        ensure!(
            names.insert(name.to_string()),
            "duplicate resource key: {name}"
        );
        if data.attribute("type").is_some() || data.attribute("mimetype").is_some() {
            continue;
        }
        let Some(value) = data.children().find(|node| node.has_tag_name("value")) else {
            continue;
        };
        ensure!(
            !value.children().any(|node| node.is_element()),
            "nested string value: {name}"
        );
        let comment = data
            .children()
            .find(|node| node.has_tag_name("comment"))
            .map(|node| node.text().unwrap_or_default().to_string());
        entries.insert(
            name.to_string(),
            ResourceCell {
                value: value.text().unwrap_or_default().to_string(),
                comment,
            },
        );
    }
    Ok((entries, names))
}

struct Edit {
    range: Range<usize>,
    replacement: String,
}

fn patch_file(file: &ResourceFile, rows: &[ResourceRow], column: usize) -> Result<String> {
    let text = &file.original;
    let document = roxmltree::Document::parse(text)?;
    let mut nodes = BTreeMap::new();
    for data in document
        .root_element()
        .children()
        .filter(|node| node.has_tag_name("data"))
    {
        if let Some(name) = data.attribute("name") {
            nodes.insert(name.to_string(), data);
        }
    }
    let mut edits = Vec::new();
    let mut additions = Vec::new();
    let retained: BTreeSet<_> = rows
        .iter()
        .filter_map(|row| row.original_name.as_deref())
        .collect();
    for original in file.entries.keys() {
        if !retained.contains(original.as_str()) {
            let data = nodes[original];
            edits.push(Edit {
                range: whole_line(text, data.range()),
                replacement: String::new(),
            });
        }
    }
    for row in rows {
        let desired = &row.cells[column];
        let original = row
            .original_name
            .as_deref()
            .and_then(|name| file.entries.get(name).map(|cell| (name, cell)));
        match (original, desired) {
            (None, None) => {}
            (None, Some(cell)) => {
                ensure!(
                    !file.all_names.contains(&row.name)
                        || row.original_name.as_deref() == Some(row.name.as_str()),
                    "resource key conflicts with a non-string entry: {}",
                    row.name
                );
                additions.push((row.name.as_str(), cell));
            }
            (Some((name, _)), None) => edits.push(Edit {
                range: whole_line(text, nodes[name].range()),
                replacement: String::new(),
            }),
            (Some((name, previous)), Some(cell)) => {
                let data = nodes[name];
                if row.name != name {
                    let attr = data
                        .attributes()
                        .find(|attr| attr.name() == "name")
                        .context("resource key has no name attribute")?;
                    edits.push(Edit {
                        range: attribute_value_range(text, attr.range())?,
                        replacement: xml_escape(&row.name, true),
                    });
                }
                if cell.value != previous.value {
                    let value = data
                        .children()
                        .find(|node| node.has_tag_name("value"))
                        .context("resource has no value")?;
                    edit_element_text(text, value, &cell.value, &mut edits)?;
                }
                if cell.comment != previous.comment {
                    if let Some(comment) = data.children().find(|node| node.has_tag_name("comment"))
                    {
                        if let Some(value) = &cell.comment {
                            edit_element_text(text, comment, value, &mut edits)?;
                        } else {
                            edits.push(Edit {
                                range: whole_line(text, comment.range()),
                                replacement: String::new(),
                            });
                        }
                    } else if let Some(value) = &cell.comment {
                        let closing = text[data.range()]
                            .rfind("</data>")
                            .context("resource has no closing data tag")?
                            + data.range().start;
                        let newline = line_ending(text);
                        let line = line_start(text, closing);
                        let on_own_line = text[line..closing].trim().is_empty();
                        let at = if on_own_line { line } else { closing };
                        let replacement = if on_own_line {
                            format!(
                                "    <comment>{}</comment>{newline}",
                                xml_escape(value, false)
                            )
                        } else {
                            format!("<comment>{}</comment>", xml_escape(value, false))
                        };
                        edits.push(Edit {
                            range: at..at,
                            replacement,
                        });
                    }
                }
            }
        }
    }
    if !additions.is_empty() {
        let root = document.root_element();
        let closing = text[root.range()]
            .rfind("</root>")
            .context("resource has no closing root tag")?
            + root.range().start;
        let line = line_start(text, closing);
        let on_own_line = text[line..closing].trim().is_empty();
        let at = if on_own_line { line } else { closing };
        let newline = line_ending(text);
        let mut inserted = if on_own_line {
            String::new()
        } else {
            newline.to_string()
        };
        for (name, cell) in additions {
            inserted.push_str(&format!("  <data name=\"{}\" xml:space=\"preserve\">{newline}    <value>{}</value>{newline}", xml_escape(name, true), xml_escape(&cell.value, false)));
            if let Some(comment) = &cell.comment {
                inserted.push_str(&format!(
                    "    <comment>{}</comment>{newline}",
                    xml_escape(comment, false)
                ));
            }
            inserted.push_str(&format!("  </data>{newline}"));
        }
        edits.push(Edit {
            range: at..at,
            replacement: inserted,
        });
    }
    edits.sort_by(|a, b| {
        b.range
            .start
            .cmp(&a.range.start)
            .then_with(|| b.range.end.cmp(&a.range.end))
    });
    let mut result = text.clone();
    let mut limit = text.len();
    for edit in edits {
        ensure!(edit.range.end <= limit, "overlapping resource edits");
        limit = edit.range.start;
        result.replace_range(edit.range, &edit.replacement);
    }
    parse_entries(&result).context("edited resource XML is invalid")?;
    Ok(result)
}

fn line_start(text: &str, offset: usize) -> usize {
    text[..offset].rfind('\n').map_or(0, |index| index + 1)
}

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
    }
    start..end
}

fn line_ending(text: &str) -> &'static str {
    if text.contains("\r\n") { "\r\n" } else { "\n" }
}

fn attribute_value_range(text: &str, range: Range<usize>) -> Result<Range<usize>> {
    let attribute = &text[range.clone()];
    let (_, after_equal) = attribute
        .split_once('=')
        .context("resource name attribute has no value")?;
    let quote = after_equal
        .find(['\'', '"'])
        .context("resource name attribute has no quote")?;
    let start = range.start + attribute.find('=').unwrap() + 1 + quote + 1;
    let marker = text.as_bytes()[start - 1];
    let end = text[start..range.end]
        .bytes()
        .position(|byte| byte == marker)
        .context("resource name attribute has no closing quote")?
        + start;
    Ok(start..end)
}

fn edit_element_text(
    text: &str,
    node: roxmltree::Node<'_, '_>,
    value: &str,
    edits: &mut Vec<Edit>,
) -> Result<()> {
    ensure!(
        !node.children().any(|child| child.is_element()),
        "resource text contains nested XML"
    );
    let replacement = xml_escape(value, false);
    if let Some(child) = node.children().find(|child| child.is_text()) {
        edits.push(Edit {
            range: child.range(),
            replacement,
        });
    } else if text[node.range()].ends_with("/>") {
        edits.push(Edit {
            range: node.range(),
            replacement: format!(
                "<{}>{}</{}>",
                node.tag_name().name(),
                replacement,
                node.tag_name().name()
            ),
        });
    } else {
        let closing = text[node.range()]
            .rfind("</")
            .context("resource element has no closing tag")?
            + node.range().start;
        edits.push(Edit {
            range: closing..closing,
            replacement,
        });
    }
    Ok(())
}

fn xml_escape(value: &str, attribute: bool) -> String {
    let mut escaped = value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    if attribute {
        escaped = escaped.replace('"', "&quot;").replace('\'', "&apos;");
    }
    escaped
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
        std::fs::write(
            &french,
            format!("{header}  <data name=\"Hello\"><value>Bonjour</value></data>\n</root>\n"),
        )
        .unwrap();
        let mut group = ResourceGroup::load(&french).unwrap();
        assert_eq!(group.files.len(), 2);
        assert_eq!(group.files[0].culture, None);
        group.rows[0].name = "Welcome".into();
        group.rows[0].cells[1].as_mut().unwrap().value = "Salut & bonjour".into();
        group.add_row();
        let added = group.rows.last_mut().unwrap();
        added.name = "Bye".into();
        added.cells[0].as_mut().unwrap().value = "Goodbye".into();
        added.cells[1] = Some(ResourceCell {
            value: "Au revoir".into(),
            comment: None,
        });
        let saved = group.save().unwrap();
        assert!(saved.rows.iter().any(|row| row.name == "Welcome"));
        let neutral_text = std::fs::read_to_string(&neutral).unwrap();
        let french_text = std::fs::read_to_string(&french).unwrap();
        assert!(neutral_text.contains("<!-- keep this header -->"));
        assert!(neutral_text.contains("type=\"System.Byte[]\""));
        assert!(neutral_text.contains("<comment>Greeting</comment>"));
        assert!(neutral_text.contains("name=\"Welcome\""));
        assert!(french_text.contains("Salut &amp; bonjour"));
        assert!(french_text.contains("name=\"Bye\""));
    }

    #[test]
    fn changed_file_blocks_save_before_any_write() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("Strings.resx");
        std::fs::write(
            &file,
            "<root>\n  <data name=\"A\"><value>x</value></data>\n</root>\n",
        )
        .unwrap();
        let mut group = ResourceGroup::load(&file).unwrap();
        group.rows[0].cells[0].as_mut().unwrap().value = "y".into();
        std::fs::write(
            &file,
            "<root>\n  <data name=\"A\"><value>external</value></data>\n</root>\n",
        )
        .unwrap();
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
        group.add_row();
        group.rows[1].name = "B".into();
        group.rows[1].cells[0].as_mut().unwrap().value = "y".into();
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
            assert!(text.contains("name=\"Keep\""));
            assert!(!text.contains("name=\"Remove\""));
        }
    }
}
