//! Folder publish profiles stored in `Properties/PublishProfiles/*.pubxml`.

use std::path::{Path, PathBuf};

use crate::build::{BuildAction, BuildRequest};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublishProfile {
    pub name: String,
    pub configuration: String,
    pub publish_dir: String,
    pub runtime_identifier: String,
    pub self_contained: bool,
    pub single_file: bool,
    pub ready_to_run: bool,
    pub trimmed: bool,
}

impl Default for PublishProfile {
    fn default() -> Self {
        Self {
            name: String::new(),
            configuration: "Release".into(),
            publish_dir: "bin/Release/publish/".into(),
            runtime_identifier: String::new(),
            self_contained: false,
            single_file: false,
            ready_to_run: false,
            trimmed: false,
        }
    }
}

impl PublishProfile {
    pub fn path_for(project: &Path, name: &str) -> anyhow::Result<PathBuf> {
        anyhow::ensure!(
            !name.is_empty()
                && name
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric()
                        || matches!(character, '-' | '_')),
            "publish profile names may contain only ASCII letters, digits, '-' and '_'"
        );
        let parent = project
            .parent()
            .ok_or_else(|| anyhow::anyhow!("project has no parent directory"))?;
        Ok(parent
            .join("Properties/PublishProfiles")
            .join(format!("{name}.pubxml")))
    }

    pub fn list(project: &Path) -> anyhow::Result<Vec<String>> {
        let dir = project
            .parent()
            .ok_or_else(|| anyhow::anyhow!("project has no parent directory"))?
            .join("Properties/PublishProfiles");
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        let mut names = entries
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let path = entry.path();
                (path
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("pubxml"))
                    && path.is_file())
                .then(|| {
                    path.file_stem()
                        .map(|stem| stem.to_string_lossy().into_owned())
                })
                .flatten()
            })
            .filter(|name| Self::path_for(project, name).is_ok())
            .collect::<Vec<_>>();
        names.sort_unstable();
        Ok(names)
    }

    pub fn load(project: &Path, name: &str) -> anyhow::Result<Self> {
        let path = Self::path_for(project, name)?;
        let text = std::fs::read_to_string(path)?;
        let doc = roxmltree::Document::parse(&text)?;
        anyhow::ensure!(
            doc.root_element().tag_name().name() == "Project",
            "publish profile must have a Project root"
        );
        let property = |name: &str| {
            doc.descendants()
                .filter(|node| node.is_element() && node.tag_name().name() == name)
                .filter_map(|node| node.text())
                .last()
                .unwrap_or("")
                .trim()
                .to_string()
        };
        let boolean = |name: &str| property(name).eq_ignore_ascii_case("true");
        let configuration = property("Configuration");
        let publish_dir = property("PublishDir");
        Ok(Self {
            name: name.into(),
            configuration: if configuration.is_empty() {
                "Release".into()
            } else {
                configuration
            },
            publish_dir: if publish_dir.is_empty() {
                property("PublishUrl")
            } else {
                publish_dir
            },
            runtime_identifier: property("RuntimeIdentifier"),
            self_contained: boolean("SelfContained"),
            single_file: boolean("PublishSingleFile"),
            ready_to_run: boolean("PublishReadyToRun"),
            trimmed: boolean("PublishTrimmed"),
        })
    }

    pub fn save(&self, project: &Path) -> anyhow::Result<PathBuf> {
        let path = Self::path_for(project, &self.name)?;
        anyhow::ensure!(
            !self.configuration.trim().is_empty(),
            "configuration is empty"
        );
        anyhow::ensure!(
            !self.publish_dir.trim().is_empty(),
            "publish folder is empty"
        );
        anyhow::ensure!(
            !self.trimmed || self.self_contained,
            "trimming requires a self-contained deployment"
        );
        anyhow::ensure!(
            !self.single_file || !self.runtime_identifier.trim().is_empty(),
            "single-file publish requires a runtime identifier"
        );
        let values = [
            ("WebPublishMethod", "FileSystem".to_string()),
            ("Configuration", self.configuration.trim().to_string()),
            ("PublishDir", self.publish_dir.trim().to_string()),
            (
                "RuntimeIdentifier",
                self.runtime_identifier.trim().to_string(),
            ),
            ("SelfContained", self.self_contained.to_string()),
            ("PublishSingleFile", self.single_file.to_string()),
            ("PublishReadyToRun", self.ready_to_run.to_string()),
            ("PublishTrimmed", self.trimmed.to_string()),
        ];
        let original = match std::fs::read_to_string(&path) {
            Ok(text) => Some(text),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        let text = match original.as_deref() {
            Some(text) => update_xml(text, &values)?,
            None => {
                let mut text = String::from("<Project>\n  <PropertyGroup>\n");
                for (key, value) in &values {
                    if !value.is_empty() {
                        text.push_str(&format!("    <{key}>{}</{key}>\n", escape_xml(value)));
                    }
                }
                text.push_str("  </PropertyGroup>\n</Project>\n");
                text
            }
        };
        let parent = path.parent().unwrap();
        std::fs::create_dir_all(parent)?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
        use std::io::Write;
        temporary.write_all(text.as_bytes())?;
        if let Ok(metadata) = std::fs::metadata(&path) {
            temporary
                .as_file()
                .set_permissions(metadata.permissions())?;
        }
        temporary.persist(&path).map_err(|error| error.error)?;
        Ok(path)
    }

    pub fn request(&self, project: &Path) -> anyhow::Result<BuildRequest> {
        let path = Self::path_for(project, &self.name)?;
        anyhow::ensure!(path.is_file(), "publish profile has not been saved");
        let mut request = BuildRequest::new(project, BuildAction::Publish);
        request.configuration = self.configuration.clone();
        request
            .properties
            .push(("PublishProfile".into(), self.name.clone()));
        Ok(request)
    }
}

fn update_xml(original: &str, values: &[(&str, String)]) -> anyhow::Result<String> {
    let document = roxmltree::Document::parse(original)?;
    let root = document.root_element();
    anyhow::ensure!(
        root.tag_name().name() == "Project",
        "publish profile must have a Project root"
    );
    let group = root
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == "PropertyGroup")
        .last()
        .ok_or_else(|| anyhow::anyhow!("publish profile has no PropertyGroup"))?;
    let mut edits = Vec::<(usize, usize, String)>::new();
    for (key, value) in values {
        let existing = root
            .descendants()
            .filter(|node| node.is_element() && node.tag_name().name() == *key)
            .last();
        if let Some(node) = existing {
            edits.push((
                node.range().start,
                node.range().end,
                if value.is_empty() {
                    String::new()
                } else {
                    format!("<{key}>{}</{key}>", escape_xml(value))
                },
            ));
        } else if !value.is_empty() {
            let close = &original[group.range()];
            let insertion = group.range().start
                + close
                    .rfind("</")
                    .ok_or_else(|| anyhow::anyhow!("invalid PropertyGroup"))?;
            edits.push((
                insertion,
                insertion,
                format!("    <{key}>{}</{key}>\n  ", escape_xml(value)),
            ));
        }
    }
    edits.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| right.1.cmp(&left.1)));
    let mut output = original.to_string();
    for (start, end, replacement) in edits {
        output.replace_range(start..end, &replacement);
    }
    roxmltree::Document::parse(&output)?;
    Ok(output)
}

fn escape_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saves_and_loads_folder_publish_profile() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("App.csproj");
        let profile = PublishProfile {
            name: "linux-x64".into(),
            publish_dir: "out/release/".into(),
            runtime_identifier: "linux-x64".into(),
            self_contained: true,
            single_file: true,
            ready_to_run: true,
            trimmed: true,
            ..Default::default()
        };
        profile.save(&project).unwrap();
        assert_eq!(PublishProfile::list(&project).unwrap(), vec!["linux-x64"]);
        assert_eq!(
            PublishProfile::load(&project, "linux-x64").unwrap(),
            profile
        );
        let request = profile.request(&project).unwrap();
        assert_eq!(request.args()[0], "publish");
        assert!(
            request
                .args()
                .contains(&"-p:PublishProfile=linux-x64".into())
        );
        assert_eq!(request.configuration, "Release");
    }

    #[test]
    fn edits_existing_profile_without_removing_unrelated_xml() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("App.csproj");
        let path = PublishProfile::path_for(&project, "Folder").unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "<Project>\n  <PropertyGroup>\n    <PublishDir>old/</PublishDir>\n    <CustomSetting>keep</CustomSetting>\n  </PropertyGroup>\n</Project>\n").unwrap();
        let profile = PublishProfile {
            name: "Folder".into(),
            publish_dir: "new/&amp/".into(),
            ..Default::default()
        };
        profile.save(&project).unwrap();
        let text = std::fs::read_to_string(path).unwrap();
        assert!(text.contains("<CustomSetting>keep</CustomSetting>"));
        assert!(text.contains("<PublishDir>new/&amp;amp/</PublishDir>"));
        assert_eq!(
            PublishProfile::load(&project, "Folder")
                .unwrap()
                .publish_dir,
            "new/&amp/"
        );
    }
}
