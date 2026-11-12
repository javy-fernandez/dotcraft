//! `Properties/launchSettings.json`: the launch profiles of a project, in file order.

use std::path::{Path, PathBuf};

use serde::Deserialize;

/// One profile of `launchSettings.json`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LaunchProfile {
    pub name: String,
    /// `Project`, `Executable`, `IISExpress`, `Docker`…
    pub command_name: String,
    pub command_line_args: Option<String>,
    /// In file order.
    pub environment_variables: Vec<(String, String)>,
    /// `;`-separated URLs (ASP.NET).
    pub application_url: Option<String>,
    pub launch_browser: bool,
    pub launch_url: Option<String>,
    pub working_directory: Option<String>,
    pub executable_path: Option<String>,
    pub dotnet_run_messages: bool,
}

impl LaunchProfile {
    /// Profiles that can be started by running the project (`commandName: Project`).
    pub fn is_project(&self) -> bool {
        self.command_name.eq_ignore_ascii_case("Project")
    }

    /// The URLs of `applicationUrl`.
    pub fn urls(&self) -> Vec<String> {
        self.application_url.as_deref().map(split_urls).unwrap_or_default()
    }

    /// The page to open in a browser (`applicationUrl` + `launchUrl`) when `launchBrowser`.
    pub fn browser_url(&self) -> Option<String> {
        if !self.launch_browser {
            return None;
        }
        let base = self.urls().into_iter().next()?;
        Some(match self.launch_url.as_deref().filter(|u| !u.is_empty()) {
            Some(u) if has_uri_scheme(u) => u.to_string(),
            Some(u) => format!("{}/{}", base.trim_end_matches('/'), u.trim_start_matches('/')),
            None => base,
        })
    }
}

fn has_uri_scheme(url: &str) -> bool {
    url.split_once("://").is_some_and(|(scheme, _)| {
        scheme.chars().next().is_some_and(|first| first.is_ascii_alphabetic())
            && scheme.chars().all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '+' | '-' | '.'))
    })
}

/// Splits `https://localhost:7001;http://localhost:5001`.
pub fn split_urls(s: &str) -> Vec<String> {
    s.split(';').map(str::trim).filter(|u| !u.is_empty()).map(str::to_string).collect()
}

/// The parsed file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LaunchSettings {
    pub profiles: Vec<LaunchProfile>,
}

impl LaunchSettings {
    /// `Properties/launchSettings.json` next to the project file or in the project folder.
    pub fn path_for(project: &Path) -> PathBuf {
        let dir = if project.is_dir() { project } else { project.parent().unwrap_or(Path::new(".")) };
        dir.join("Properties").join("launchSettings.json")
    }

    /// Loads the settings of a project; `Ok(None)` when it has none.
    pub fn load(project: &Path) -> anyhow::Result<Option<LaunchSettings>> {
        let path = Self::path_for(project);
        match std::fs::read_to_string(&path) {
            Ok(text) => Ok(Some(Self::parse(&text)?)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Update one profile while retaining file-level and profile-level fields this UI does
    /// not know about. `None` removes the named profile. A new profile uses `original = None`.
    pub fn write_profile(project: &Path, original: Option<&str>, profile: Option<&LaunchProfile>) -> anyhow::Result<()> {
        anyhow::ensure!(original.is_some() || profile.is_some(), "no launch profile selected");
        let path = Self::path_for(project);
        let mut document: serde_json::Value = match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(text.trim_start_matches('\u{feff}'))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => serde_json::json!({ "profiles": {} }),
            Err(error) => return Err(error.into()),
        };
        let root = document.as_object_mut().ok_or_else(|| anyhow::anyhow!("launchSettings.json must contain an object"))?;
        let profiles = root.entry("profiles").or_insert_with(|| serde_json::json!({})).as_object_mut()
            .ok_or_else(|| anyhow::anyhow!("profiles must contain an object"))?;
        if let Some(old) = original {
            anyhow::ensure!(profiles.contains_key(old), "launch profile '{old}' no longer exists");
        }
        if let Some(profile) = profile {
            let name = profile.name.trim();
            anyhow::ensure!(!name.is_empty(), "launch profile name is empty");
            anyhow::ensure!(original == Some(name) || !profiles.contains_key(name), "launch profile '{name}' already exists");
            let mut entry = if original == Some(name) {
                profiles.get(name).cloned()
            } else {
                original.and_then(|old| profiles.shift_remove(old))
            }.unwrap_or_else(|| serde_json::json!({}));
            let fields = entry.as_object_mut().ok_or_else(|| anyhow::anyhow!("launch profile must contain an object"))?;
            fields.insert("commandName".into(), serde_json::Value::String(profile.command_name.clone()));
            set_optional(fields, "commandLineArgs", profile.command_line_args.as_deref());
            set_optional(fields, "applicationUrl", profile.application_url.as_deref());
            fields.insert("launchBrowser".into(), serde_json::Value::Bool(profile.launch_browser));
            set_optional(fields, "launchUrl", profile.launch_url.as_deref());
            set_optional(fields, "workingDirectory", profile.working_directory.as_deref());
            set_optional(fields, "executablePath", profile.executable_path.as_deref());
            let env = profile.environment_variables.iter().map(|(key, value)| (key.clone(), serde_json::Value::String(value.clone()))).collect();
            fields.insert("environmentVariables".into(), serde_json::Value::Object(env));
            profiles.insert(name.into(), entry);
        } else if let Some(old) = original {
            profiles.shift_remove(old);
        }
        let parent = path.parent().ok_or_else(|| anyhow::anyhow!("launchSettings.json has no parent directory"))?;
        std::fs::create_dir_all(parent)?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
        serde_json::to_writer_pretty(&mut temporary, &document)?;
        use std::io::Write;
        temporary.write_all(b"\n")?;
        if let Ok(metadata) = std::fs::metadata(&path) {
            temporary.as_file().set_permissions(metadata.permissions())?;
        }
        temporary.persist(path).map_err(|error| error.error)?;
        Ok(())
    }

    pub fn parse(text: &str) -> anyhow::Result<LaunchSettings> {
        let text = text.trim_start_matches('\u{feff}');
        let file: FileDe = serde_json::from_str(text)?;
        Ok(LaunchSettings {
            profiles: file
                .profiles
                .0
                .into_iter()
                .map(|(name, p)| LaunchProfile {
                    name,
                    command_name: p.command_name.unwrap_or_default(),
                    command_line_args: p.command_line_args,
                    environment_variables: p.environment_variables.0.into_iter().map(|(k, v)| (k, json_to_string(v))).collect(),
                    application_url: p.application_url,
                    launch_browser: p.launch_browser.unwrap_or(false),
                    launch_url: p.launch_url,
                    working_directory: p.working_directory,
                    executable_path: p.executable_path,
                    dotnet_run_messages: p.dotnet_run_messages.unwrap_or(false),
                })
                .collect(),
        })
    }

    pub fn profile(&self, name: &str) -> Option<&LaunchProfile> {
        self.profiles.iter().find(|p| p.name == name)
    }

    /// What `dotnet run` picks: the first `Project` profile.
    pub fn default_profile(&self) -> Option<&LaunchProfile> {
        self.profiles.iter().find(|p| p.is_project())
    }
}

fn set_optional(fields: &mut serde_json::Map<String, serde_json::Value>, name: &str, value: Option<&str>) {
    if let Some(value) = value.filter(|value| !value.is_empty()) {
        fields.insert(name.into(), serde_json::Value::String(value.into()));
    } else {
        fields.shift_remove(name);
    }
}

fn json_to_string(v: serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s,
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    }
}

#[derive(Deserialize)]
struct FileDe {
    #[serde(default)]
    profiles: Ordered<ProfileDe>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProfileDe {
    command_name: Option<String>,
    command_line_args: Option<String>,
    #[serde(default)]
    environment_variables: Ordered<serde_json::Value>,
    application_url: Option<String>,
    launch_browser: Option<bool>,
    launch_url: Option<String>,
    working_directory: Option<String>,
    executable_path: Option<String>,
    dotnet_run_messages: Option<bool>,
}

/// A JSON object kept in file order.
struct Ordered<T>(Vec<(String, T)>);

impl<T> Default for Ordered<T> {
    fn default() -> Self {
        Ordered(Vec::new())
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Ordered<T> {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V<T>(std::marker::PhantomData<T>);
        impl<'de, T: Deserialize<'de>> serde::de::Visitor<'de> for V<T> {
            type Value = Ordered<T>;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("an object")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut out = Vec::new();
                while let Some((k, v)) = map.next_entry::<String, T>()? {
                    out.push((k, v));
                }
                Ok(Ordered(out))
            }
            fn visit_unit<E>(self) -> Result<Self::Value, E> {
                Ok(Ordered(Vec::new()))
            }
        }
        d.deserialize_any(V(std::marker::PhantomData))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\u{feff}{
  \"$schema\": \"http://json.schemastore.org/launchsettings.json\",
  \"iisSettings\": { \"iisExpress\": { \"applicationUrl\": \"http://localhost:42877\", \"sslPort\": 0 } },
  \"profiles\": {
    \"https\": {
      \"commandName\": \"Project\",
      \"dotnetRunMessages\": true,
      \"launchBrowser\": true,
      \"launchUrl\": \"swagger\",
      \"applicationUrl\": \"https://localhost:7001;http://localhost:5112\",
      \"commandLineArgs\": \"--seed \\\"demo data\\\"\",
      \"workingDirectory\": \"$(ProjectDir)\",
      \"environmentVariables\": { \"ASPNETCORE_ENVIRONMENT\": \"Development\", \"B\": \"2\", \"A\": \"1\" }
    },
    \"IIS Express\": { \"commandName\": \"IISExpress\", \"launchBrowser\": true },
    \"tool\": { \"commandName\": \"Executable\", \"executablePath\": \"/usr/bin/env\", \"environmentVariables\": null }
  }
}";

    #[test]
    fn parses_profiles_in_order() {
        let s = LaunchSettings::parse(SAMPLE).unwrap();
        let names: Vec<_> = s.profiles.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["https", "IIS Express", "tool"]);
        let p = s.default_profile().unwrap();
        assert_eq!(p.name, "https");
        assert!(p.dotnet_run_messages && p.launch_browser);
        assert_eq!(p.command_line_args.as_deref(), Some("--seed \"demo data\""));
        assert_eq!(p.working_directory.as_deref(), Some("$(ProjectDir)"));
        let keys: Vec<_> = p.environment_variables.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, vec!["ASPNETCORE_ENVIRONMENT", "B", "A"]);
        assert_eq!(p.urls(), vec!["https://localhost:7001", "http://localhost:5112"]);
        assert_eq!(p.browser_url().as_deref(), Some("https://localhost:7001/swagger"));
        assert!(!s.profile("IIS Express").unwrap().is_project());
        assert_eq!(s.profile("tool").unwrap().executable_path.as_deref(), Some("/usr/bin/env"));
    }

    #[test]
    fn loads_the_fixture_and_handles_missing_files() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/Shop/src/Shop.Api/Shop.Api.csproj");
        let s = LaunchSettings::load(&fixture).unwrap().unwrap();
        assert_eq!(s.default_profile().unwrap().application_url.as_deref(), Some("http://localhost:5112"));
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(LaunchSettings::load(&dir.path().join("X.csproj")).unwrap(), None);
    }

    #[test]
    fn edits_profiles_without_dropping_other_settings() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("App.csproj");
        let path = LaunchSettings::path_for(&project);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, r#"{"$schema":"example","profiles":{"first":{"commandName":"Project","customFlag":9},"second":{"commandName":"Project"}},"iisSettings":{"windowsAuthentication":false}}"#).unwrap();
        let profile = LaunchProfile {
            name: "renamed".into(),
            command_name: "Project".into(),
            application_url: Some("http://localhost:5100".into()),
            environment_variables: vec![("MODE".into(), "Development".into())],
            launch_browser: true,
            command_line_args: Some("--sample".into()),
            ..Default::default()
        };
        LaunchSettings::write_profile(&project, Some("first"), Some(&profile)).unwrap();
        let document: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(document["$schema"], "example");
        assert_eq!(document["iisSettings"]["windowsAuthentication"], false);
        assert_eq!(document["profiles"]["renamed"]["customFlag"], 9);
        assert_eq!(document["profiles"]["renamed"]["environmentVariables"]["MODE"], "Development");
        assert!(document["profiles"].get("first").is_none());
        assert!(document["profiles"].get("second").is_some());
        assert_eq!(LaunchSettings::load(&project).unwrap().unwrap().profiles.iter().map(|profile| profile.name.as_str()).collect::<Vec<_>>(), vec!["second", "renamed"]);
        let mut created = profile.clone();
        created.name = "third".into();
        LaunchSettings::write_profile(&project, None, Some(&created)).unwrap();
        LaunchSettings::write_profile(&project, Some("third"), None).unwrap();
        assert!(LaunchSettings::load(&project).unwrap().unwrap().profile("third").is_none());
    }
}
