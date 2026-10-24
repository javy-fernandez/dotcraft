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
            Some(u) if u.contains("://") => u.to_string(),
            Some(u) => format!("{}/{}", base.trim_end_matches('/'), u.trim_start_matches('/')),
            None => base,
        })
    }
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
}
