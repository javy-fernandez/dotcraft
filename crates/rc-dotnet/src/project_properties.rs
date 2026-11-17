//! A small, formatting-preserving editor for common SDK-style project properties.

use std::path::Path;

use crate::cpm::{TextEdit, apply_edits, set_central_property};

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

impl ProjectPropertiesEditor {
    pub fn load(path: &Path) -> anyhow::Result<(Self, String)> {
        let source = std::fs::read_to_string(path)?;
        let model = Self::parse(&source)?;
        Ok((model, source))
    }

    pub fn parse(source: &str) -> anyhow::Result<Self> {
        let doc = roxmltree::Document::parse(source)?;
        anyhow::ensure!(
            doc.root_element().tag_name().name() == "Project",
            "file is not an MSBuild project"
        );
        let value = |name: &str| {
            doc.root_element()
                .children()
                .filter(|group| {
                    group.has_tag_name("PropertyGroup") && group.attribute("Condition").is_none()
                })
                .flat_map(|group| group.children())
                .filter(|node| node.has_tag_name(name) && node.attribute("Condition").is_none())
                .filter_map(|node| node.text())
                .last()
                .unwrap_or("")
                .trim()
                .to_string()
        };
        let frameworks = value("TargetFrameworks");
        Ok(Self {
            target_frameworks: if frameworks.is_empty() {
                value("TargetFramework")
            } else {
                frameworks
            },
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
    pub fn save(
        &self,
        path: &Path,
        original: &Self,
        expected_source: &str,
    ) -> anyhow::Result<String> {
        let current = std::fs::read_to_string(path)?;
        anyhow::ensure!(
            current == expected_source,
            "the project file changed on disk; reopen Properties before saving"
        );
        let updated = self.apply(&current, original)?;
        if updated == current {
            return Ok(updated);
        }
        let doc = roxmltree::Document::parse(&updated)?;
        anyhow::ensure!(
            doc.root_element().tag_name().name() == "Project",
            "edited project XML is invalid"
        );
        let parent = path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("project has no parent directory"))?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
        use std::io::Write;
        temporary.write_all(updated.as_bytes())?;
        temporary
            .as_file()
            .set_permissions(std::fs::metadata(path)?.permissions())?;
        temporary.persist(path).map_err(|error| error.error)?;
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
                let many = frameworks.contains(';');
                output = set(
                    &output,
                    if many {
                        "TargetFrameworks"
                    } else {
                        "TargetFramework"
                    },
                    frameworks,
                )?;
                output = remove(
                    &output,
                    if many {
                        "TargetFramework"
                    } else {
                        "TargetFrameworks"
                    },
                )?;
            }
        }
        for (name, value, previous) in [
            ("Nullable", &self.nullable, &original.nullable),
            (
                "LangVersion",
                &self.language_version,
                &original.language_version,
            ),
            (
                "ImplicitUsings",
                &self.implicit_usings,
                &original.implicit_usings,
            ),
            (
                "TreatWarningsAsErrors",
                &self.warnings_as_errors,
                &original.warnings_as_errors,
            ),
            ("PackageId", &self.package_id, &original.package_id),
            ("Version", &self.version, &original.version),
            ("Authors", &self.authors, &original.authors),
            ("Description", &self.description, &original.description),
            (
                "PackageLicenseExpression",
                &self.license_expression,
                &original.license_expression,
            ),
        ] {
            if value != previous {
                let value = value.trim();
                output = if value.is_empty() {
                    remove(&output, name)?
                } else {
                    set(&output, name, value)?
                };
            }
        }
        if self.license_expression != original.license_expression
            && !self.license_expression.trim().is_empty()
        {
            output = remove(&output, "PackageLicenseFile")?;
        }
        Ok(output)
    }
}

fn set(source: &str, name: &str, value: &str) -> anyhow::Result<String> {
    let doc = roxmltree::Document::parse(source)?;
    let root = doc.root_element();
    let range = root.range();
    if source[range.clone()].trim_end().ends_with("/>") {
        let close = range.end - 2;
        let mut expanded = source.to_string();
        expanded.replace_range(close..range.end, "></Project>");
        return set(&expanded, name, value);
    }
    let edits = set_central_property(source, name, value)?;
    Ok(apply_edits(source, &edits))
}

fn remove(source: &str, name: &str) -> anyhow::Result<String> {
    let doc = roxmltree::Document::parse(source)?;
    let edits = doc
        .root_element()
        .children()
        .filter(|group| {
            group.has_tag_name("PropertyGroup") && group.attribute("Condition").is_none()
        })
        .flat_map(|group| group.children())
        .filter(|node| node.has_tag_name(name) && node.attribute("Condition").is_none())
        .map(|node| TextEdit {
            range: node.range(),
            text: String::new(),
        })
        .collect::<Vec<_>>();
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
        assert_eq!(
            ProjectPropertiesEditor::parse(&result).unwrap().authors,
            "DotCraft & Friends"
        );
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
        assert_eq!(
            ProjectPropertiesEditor::load(&path)
                .unwrap()
                .0
                .implicit_usings,
            "enable"
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
        assert_eq!(
            ProjectPropertiesEditor::parse(&result)
                .unwrap()
                .target_frameworks,
            "net10.0"
        );
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
        assert!(
            result.contains(
                "<Nullable Condition=\"'$(Configuration)' == 'Debug'\">disable</Nullable>"
            )
        );
        assert_eq!(ProjectPropertiesEditor::parse(&result).unwrap(), edited);
    }
}
