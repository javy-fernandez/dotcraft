//! A small, formatting-preserving editor for common SDK-style project properties.

use std::path::Path;

use crate::cpm::{TextEdit, apply_edits, element_text_edit, group_span, insert_property, line_span};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProjectPropertiesEditor {
    pub target_frameworks: String,
    pub nullable: String,
    pub language_version: String,
    pub implicit_usings: String,
    pub warnings_as_errors: String,
    pub package_id: String,
    pub version: String,
    pub authors: String,
    pub description: String,
    pub license_expression: String,
}

/// The unconditional `<name>` elements of the unconditional `<PropertyGroup>`s directly under
/// `<Project>`, in document order. MSBuild keeps the last one, so that is the one shown and edited.
fn occurrences<'a, 'input: 'a>(doc: &'a roxmltree::Document<'input>, name: &'a str) -> impl Iterator<Item = roxmltree::Node<'a, 'input>> + 'a {
    doc.root_element()
        .children()
        .filter(|group| group.has_tag_name("PropertyGroup") && group.attribute("Condition").is_none())
        .flat_map(|group| group.children())
        .filter(move |node| node.has_tag_name(name) && node.attribute("Condition").is_none())
}

impl ProjectPropertiesEditor {
    pub fn load(path: &Path) -> anyhow::Result<(Self, String)> {
        let source = std::fs::read_to_string(path)?;
        let model = Self::parse(&source)?;
        Ok((model, source))
    }

    pub fn parse(source: &str) -> anyhow::Result<Self> {
        let doc = roxmltree::Document::parse(source)?;
        anyhow::ensure!(doc.root_element().tag_name().name() == "Project", "file is not an MSBuild project");
        let value = |name: &str| occurrences(&doc, name).last().and_then(|node| node.text()).unwrap_or("").trim().to_string();
        let frameworks = value("TargetFrameworks");
        Ok(Self {
            target_frameworks: if frameworks.is_empty() { value("TargetFramework") } else { frameworks },
            nullable: value("Nullable"),
            language_version: value("LangVersion"),
            implicit_usings: value("ImplicitUsings"),
            warnings_as_errors: value("TreatWarningsAsErrors"),
            package_id: value("PackageId"),
            version: value("Version"),
            authors: value("Authors"),
            description: value("Description"),
            license_expression: value("PackageLicenseExpression"),
        })
    }

    /// Writes only fields changed by the form. A concurrent disk edit is rejected instead of
    /// silently replacing it. All other XML, including comments and whitespace, remains intact.
    pub fn save(&self, path: &Path, original: &Self, expected_source: &str) -> anyhow::Result<String> {
        let current = std::fs::read_to_string(path)?;
        anyhow::ensure!(current == expected_source, "the project file changed on disk; reopen Properties before saving");
        let updated = self.apply(&current, original)?;
        if updated == current {
            return Ok(updated);
        }
        let doc = roxmltree::Document::parse(&updated)?;
        anyhow::ensure!(doc.root_element().tag_name().name() == "Project", "edited project XML is invalid");
        rc_core::paths::write_atomic(path, updated.as_bytes())?;
        Ok(updated)
    }

    pub fn apply(&self, source: &str, original: &Self) -> anyhow::Result<String> {
        let mut output = source.to_string();
        if self.target_frameworks != original.target_frameworks {
            let frameworks = self.target_frameworks.trim();
            if frameworks.is_empty() {
                output = remove(&output, "TargetFramework")?;
                output = remove(&output, "TargetFrameworks")?;
            } else {
                let (keep, drop) = if frameworks.contains(';') { ("TargetFrameworks", "TargetFramework") } else { ("TargetFramework", "TargetFrameworks") };
                output = set(&output, keep, frameworks)?;
                output = remove(&output, drop)?;
            }
        }
        for (name, value, previous) in [
            ("Nullable", &self.nullable, &original.nullable),
            ("LangVersion", &self.language_version, &original.language_version),
            ("ImplicitUsings", &self.implicit_usings, &original.implicit_usings),
            ("TreatWarningsAsErrors", &self.warnings_as_errors, &original.warnings_as_errors),
            ("PackageId", &self.package_id, &original.package_id),
            ("Version", &self.version, &original.version),
            ("Authors", &self.authors, &original.authors),
            ("Description", &self.description, &original.description),
            ("PackageLicenseExpression", &self.license_expression, &original.license_expression),
        ] {
            if value != previous {
                let value = value.trim();
                output = if value.is_empty() { remove(&output, name)? } else { set(&output, name, value)? };
            }
        }
        if self.license_expression != original.license_expression && !self.license_expression.trim().is_empty() {
            output = remove(&output, "PackageLicenseFile")?;
        }
        Ok(output)
    }
}

/// Sets `name` to `value` in the occurrence [`ProjectPropertiesEditor::parse`] shows, or adds it
/// to the first unconditional `<PropertyGroup>` when the project does not define it.
fn set(source: &str, name: &str, value: &str) -> anyhow::Result<String> {
    let doc = roxmltree::Document::parse(source)?;
    let root = doc.root_element();
    let range = root.range();
    if source[range.clone()].trim_end().ends_with("/>") {
        let close = source[..range.end].rfind("/>").unwrap_or(range.end - 2);
        let mut expanded = source.to_string();
        expanded.replace_range(close..range.end, "></Project>");
        return set(&expanded, name, value);
    }
    let edit = match occurrences(&doc, name).last() {
        Some(node) if node.text().unwrap_or("").trim() == value => return Ok(source.to_string()),
        Some(node) => element_text_edit(source, node, value),
        None => insert_property(&doc, source, name, value)?,
    };
    Ok(apply_edits(source, &[edit]))
}

/// Removes every occurrence of `name` that the editor reads, with the line it leaves blank, and
/// a `<PropertyGroup>` left with nothing else in it.
fn remove(source: &str, name: &str) -> anyhow::Result<String> {
    let doc = roxmltree::Document::parse(source)?;
    let mut edits = Vec::new();
    for group in doc.root_element().children().filter(|group| group.has_tag_name("PropertyGroup") && group.attribute("Condition").is_none()) {
        let targets: Vec<_> = group.children().filter(|node| node.has_tag_name(name) && node.attribute("Condition").is_none()).collect();
        if targets.is_empty() {
            continue;
        }
        let emptied = group.children().all(|node| targets.contains(&node) || (node.is_text() && node.text().unwrap_or("").trim().is_empty()));
        if emptied {
            edits.push(TextEdit { range: group_span(source, group.range()), text: String::new() });
        } else {
            edits.extend(targets.into_iter().map(|node| TextEdit { range: line_span(source, node.range()), text: String::new() }));
        }
    }
    Ok(apply_edits(source, &edits))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_only_selected_properties_and_preserves_comments_and_line_endings() {
        let original = "<Project Sdk=\"Microsoft.NET.Sdk\">\r\n  <!-- keep this -->\r\n  <PropertyGroup>\r\n    <TargetFramework>net8.0</TargetFramework>\r\n    <Nullable>enable</Nullable>\r\n    <Description>Old</Description> <!-- metadata note -->\r\n  </PropertyGroup>\r\n  <ItemGroup><PackageReference Include=\"Example\" Version=\"1.0\" /></ItemGroup>\r\n</Project>\r\n";
        let baseline = ProjectPropertiesEditor::parse(original).unwrap();
        let mut edited = baseline.clone();
        edited.target_frameworks = "net8.0;net10.0".into();
        edited.nullable = "disable".into();
        edited.description.clear();
        edited.authors = "DotCraft & Friends".into();
        let result = edited.apply(original, &baseline).unwrap();
        assert!(result.contains("<!-- keep this -->\r\n"));
        assert!(result.contains("<PackageReference Include=\"Example\" Version=\"1.0\" />"));
        assert!(result.contains("<TargetFrameworks>net8.0;net10.0</TargetFrameworks>"));
        assert!(!result.contains("<TargetFramework>"));
        assert!(result.contains("<Nullable>disable</Nullable>"));
        assert!(!result.contains("<Description>Old</Description>"));
        assert!(result.contains("<!-- metadata note -->"));
        assert!(result.contains("<Authors>DotCraft &amp; Friends</Authors>"));
        assert!(!result.replace("\r\n", "").contains('\n'), "only CRLF line endings: {result:?}");
        assert_eq!(ProjectPropertiesEditor::parse(&result).unwrap().authors, "DotCraft & Friends");
    }

    #[test]
    fn rejects_disk_changes_and_keeps_other_xml_on_save() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("App.csproj");
        std::fs::write(&path, "<Project><PropertyGroup><TargetFramework>net8.0</TargetFramework></PropertyGroup></Project>").unwrap();
        let (baseline, source) = ProjectPropertiesEditor::load(&path).unwrap();
        let mut edited = baseline.clone();
        edited.implicit_usings = "enable".into();
        std::fs::write(&path, format!("{source}\n<!-- external -->")).unwrap();
        assert!(edited.save(&path, &baseline, &source).is_err());
        std::fs::write(&path, &source).unwrap();
        edited.save(&path, &baseline, &source).unwrap();
        assert_eq!(ProjectPropertiesEditor::load(&path).unwrap().0.implicit_usings, "enable");
        let names: Vec<_> = std::fs::read_dir(dir.path()).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(names, ["App.csproj"], "no temporary file is left behind");
    }

    #[test]
    fn save_keeps_the_byte_order_mark_and_crlf() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("App.csproj");
        let source = "\u{feff}<Project Sdk=\"Microsoft.NET.Sdk\">\r\n  <PropertyGroup>\r\n    <TargetFramework>net8.0</TargetFramework>\r\n  </PropertyGroup>\r\n</Project>\r\n";
        std::fs::write(&path, source).unwrap();
        let (baseline, loaded) = ProjectPropertiesEditor::load(&path).unwrap();
        let mut edited = baseline.clone();
        edited.nullable = "enable".into();
        edited.save(&path, &baseline, &loaded).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "\u{feff}<Project Sdk=\"Microsoft.NET.Sdk\">\r\n  <PropertyGroup>\r\n    <TargetFramework>net8.0</TargetFramework>\r\n    <Nullable>enable</Nullable>\r\n  </PropertyGroup>\r\n</Project>\r\n"
        );
    }

    #[test]
    fn adds_properties_to_an_empty_project_without_reformatting_the_root() {
        let source = "<Project Sdk=\"Microsoft.NET.Sdk\"/>";
        let baseline = ProjectPropertiesEditor::parse(source).unwrap();
        let mut edited = baseline.clone();
        edited.target_frameworks = "net10.0".into();
        let result = edited.apply(source, &baseline).unwrap();
        assert!(result.starts_with("<Project Sdk=\"Microsoft.NET.Sdk\">"));
        assert_eq!(ProjectPropertiesEditor::parse(&result).unwrap().target_frameworks, "net10.0");
    }

    #[test]
    fn round_trips_compiler_and_package_metadata_fields_without_touching_conditionals() {
        let source = "<Project><PropertyGroup><Nullable Condition=\"'$(Configuration)' == 'Debug'\">disable</Nullable></PropertyGroup></Project>";
        let baseline = ProjectPropertiesEditor::parse(source).unwrap();
        assert!(baseline.nullable.is_empty());
        let edited = ProjectPropertiesEditor {
            target_frameworks: "net10.0".into(),
            nullable: "enable".into(),
            language_version: "latest".into(),
            implicit_usings: "enable".into(),
            warnings_as_errors: "true".into(),
            package_id: "DotCraft.Sample".into(),
            version: "1.2.3".into(),
            authors: "DotCraft".into(),
            description: "Sample & tools".into(),
            license_expression: "MIT".into(),
        };
        let result = edited.apply(source, &baseline).unwrap();
        assert!(result.contains("<Nullable Condition=\"'$(Configuration)' == 'Debug'\">disable</Nullable>"));
        assert_eq!(ProjectPropertiesEditor::parse(&result).unwrap(), edited);
    }

    #[test]
    fn edits_the_occurrence_it_shows_when_a_property_repeats() {
        let source = "<Project>\n  <PropertyGroup>\n    <Nullable>disable</Nullable>\n  </PropertyGroup>\n  <Target Name=\"X\">\n    <PropertyGroup>\n      <Nullable>annotations</Nullable>\n    </PropertyGroup>\n  </Target>\n  <PropertyGroup Condition=\"'$(Configuration)' == 'Debug'\">\n    <Nullable>warnings</Nullable>\n  </PropertyGroup>\n  <PropertyGroup>\n    <Nullable>enable</Nullable>\n  </PropertyGroup>\n</Project>\n";
        let baseline = ProjectPropertiesEditor::parse(source).unwrap();
        // MSBuild keeps the last unconditional definition directly under <Project>.
        assert_eq!(baseline.nullable, "enable");
        let mut edited = baseline.clone();
        edited.nullable = "warnings".into();
        let result = edited.apply(source, &baseline).unwrap();
        assert_eq!(result, source.replace("<Nullable>enable</Nullable>", "<Nullable>warnings</Nullable>"));
        assert_eq!(ProjectPropertiesEditor::parse(&result).unwrap().nullable, "warnings");
    }

    #[test]
    fn adds_a_missing_property_to_the_project_not_to_a_target() {
        let source = "<Project>\n  <Target Name=\"X\">\n    <PropertyGroup>\n      <Version>9.9.9</Version>\n    </PropertyGroup>\n  </Target>\n  <PropertyGroup>\n    <TargetFramework>net8.0</TargetFramework>\n  </PropertyGroup>\n</Project>\n";
        let baseline = ProjectPropertiesEditor::parse(source).unwrap();
        assert!(baseline.version.is_empty());
        let mut edited = baseline.clone();
        edited.version = "1.0.0".into();
        let result = edited.apply(source, &baseline).unwrap();
        assert!(result.contains("<Version>9.9.9</Version>"));
        assert!(result.contains("    <TargetFramework>net8.0</TargetFramework>\n    <Version>1.0.0</Version>\n"));
    }

    #[test]
    fn clearing_a_property_removes_its_line_and_a_group_left_empty() {
        let source = "<Project Sdk=\"Microsoft.NET.Sdk\">\r\n  <PropertyGroup>\r\n    <TargetFramework>net8.0</TargetFramework>\r\n    <Nullable>enable</Nullable>\r\n  </PropertyGroup>\r\n  <PropertyGroup>\r\n    <Description>Old</Description>\r\n  </PropertyGroup>\r\n  <ItemGroup />\r\n</Project>\r\n";
        let baseline = ProjectPropertiesEditor::parse(source).unwrap();
        let mut edited = baseline.clone();
        edited.nullable.clear();
        edited.description.clear();
        let result = edited.apply(source, &baseline).unwrap();
        assert_eq!(result, "<Project Sdk=\"Microsoft.NET.Sdk\">\r\n  <PropertyGroup>\r\n    <TargetFramework>net8.0</TargetFramework>\r\n  </PropertyGroup>\r\n  <ItemGroup />\r\n</Project>\r\n");
    }

    #[test]
    fn clearing_keeps_a_group_that_still_holds_a_comment() {
        let source = "<Project>\n  <PropertyGroup>\n    <!-- packaging -->\n    <Authors>Me</Authors>\n  </PropertyGroup>\n</Project>\n";
        let baseline = ProjectPropertiesEditor::parse(source).unwrap();
        let mut edited = baseline.clone();
        edited.authors.clear();
        assert_eq!(edited.apply(source, &baseline).unwrap(), "<Project>\n  <PropertyGroup>\n    <!-- packaging -->\n  </PropertyGroup>\n</Project>\n");
    }
}
