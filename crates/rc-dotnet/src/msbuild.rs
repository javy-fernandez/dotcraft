//! Evaluated MSBuild properties through `dotnet msbuild <proj> -getProperty:A -getProperty:B`
//! (SDK 8+): one raw value for a single property, JSON for several.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::sdk::Dotnet;

/// Properties read by [`evaluate`].
pub const PROJECT_PROPERTIES: &[&str] = &[
    "TargetFramework",
    "TargetFrameworks",
    "OutputType",
    "TargetPath",
    "AssemblyName",
    "RootNamespace",
    "ProjectDir",
    "OutDir",
    "IsTestProject",
    "UsingMicrosoftNETSdkWeb",
    "RunCommand",
    "RunArguments",
    "RunWorkingDirectory",
];

/// The evaluated view of a project for one configuration and target framework.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProjectProperties {
    /// The framework these values are for (the first of `TargetFrameworks` when multi-targeting).
    pub target_framework: String,
    /// Every framework of the project (one element when not multi-targeting).
    pub target_frameworks: Vec<String>,
    pub output_type: String,
    /// The built assembly (`bin/Debug/net8.0/App.dll`).
    pub target_path: PathBuf,
    pub assembly_name: String,
    pub root_namespace: String,
    pub project_dir: PathBuf,
    pub out_dir: PathBuf,
    pub is_test_project: bool,
    pub is_web: bool,
    /// Every property returned, by name.
    pub raw: BTreeMap<String, String>,
}

impl ProjectProperties {
    pub fn is_executable(&self) -> bool {
        self.output_type.eq_ignore_ascii_case("Exe") || self.output_type.eq_ignore_ascii_case("WinExe")
    }

    /// Builds the struct from `-getProperty` values (pure, for tests).
    pub fn from_values(values: BTreeMap<String, String>) -> ProjectProperties {
        let get = |k: &str| values.get(k).map(|s| s.trim().to_string()).unwrap_or_default();
        let truthy = |k: &str| get(k).eq_ignore_ascii_case("true");
        let mut tfs: Vec<String> = get("TargetFrameworks").split(';').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect();
        let tf = get("TargetFramework");
        if tfs.is_empty() && !tf.is_empty() {
            tfs.push(tf.clone());
        }
        ProjectProperties {
            target_framework: if tf.is_empty() { tfs.first().cloned().unwrap_or_default() } else { tf },
            target_frameworks: tfs,
            output_type: get("OutputType"),
            target_path: PathBuf::from(get("TargetPath")),
            assembly_name: get("AssemblyName"),
            root_namespace: get("RootNamespace"),
            project_dir: PathBuf::from(get("ProjectDir")),
            out_dir: PathBuf::from(get("OutDir")),
            is_test_project: truthy("IsTestProject"),
            is_web: truthy("UsingMicrosoftNETSdkWeb"),
            raw: values,
        }
    }
}

/// Reads `names` from `project` with optional global properties (`Configuration`, …).
pub async fn get_properties(dotnet: &Dotnet, project: &Path, names: &[&str], global: &[(&str, &str)]) -> anyhow::Result<BTreeMap<String, String>> {
    anyhow::ensure!(!names.is_empty(), "no properties requested");
    let mut cmd = dotnet.command();
    cmd.arg("msbuild").arg(project).arg("-nologo");
    for n in names {
        cmd.arg(format!("-getProperty:{n}"));
    }
    for (k, v) in global {
        cmd.arg(format!("-p:{k}={v}"));
    }
    if let Some(dir) = project.parent() {
        cmd.current_dir(dir);
    }
    cmd.kill_on_drop(true);
    let out = cmd.output().await?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    if !out.status.success() {
        let diags = crate::diagnostics::parse_all(&stdout);
        let msg = diags.first().map(|d| d.message.clone()).unwrap_or_else(|| format!("{}{}", stdout.trim(), String::from_utf8_lossy(&out.stderr).trim()));
        anyhow::bail!("dotnet msbuild -getProperty failed for {}: {msg}", project.display());
    }
    parse_get_property(names, &stdout)
}

/// Evaluates the usual project properties for `configuration`; when the project
/// multi-targets, evaluates again for `target_framework` (or the first framework) so that
/// `TargetPath` and friends are the inner build's values.
pub async fn evaluate(dotnet: &Dotnet, project: &Path, configuration: &str, target_framework: Option<&str>) -> anyhow::Result<ProjectProperties> {
    let mut global = vec![("Configuration", configuration)];
    if let Some(tf) = target_framework {
        global.push(("TargetFramework", tf));
    }
    let mut props = ProjectProperties::from_values(get_properties(dotnet, project, PROJECT_PROPERTIES, &global).await?);
    if target_framework.is_none() && props.raw.get("TargetFramework").is_none_or(|v| v.trim().is_empty()) && !props.target_framework.is_empty() {
        let tf = props.target_framework.clone();
        let all = props.target_frameworks.clone();
        global.push(("TargetFramework", &tf));
        props = ProjectProperties::from_values(get_properties(dotnet, project, PROJECT_PROPERTIES, &global).await?);
        props.target_frameworks = all;
    }
    Ok(props)
}

/// Parses the output of `-getProperty`: the raw value for one name, `{"Properties":{…}}` for more.
pub fn parse_get_property(names: &[&str], stdout: &str) -> anyhow::Result<BTreeMap<String, String>> {
    let mut map = BTreeMap::new();
    if names.len() == 1 {
        map.insert(names[0].to_string(), stdout.trim_end_matches(['\r', '\n']).to_string());
        return Ok(map);
    }
    let start = stdout.find('{').ok_or_else(|| anyhow::anyhow!("no JSON in msbuild output: {}", stdout.trim()))?;
    let v: serde_json::Value = serde_json::from_str(&stdout[start..])?;
    let props = v.get("Properties").and_then(|p| p.as_object()).ok_or_else(|| anyhow::anyhow!("no Properties in msbuild output"))?;
    for (k, v) in props {
        map.insert(k.clone(), v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string()));
    }
    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_single_and_multiple_properties() {
        let one = parse_get_property(&["TargetPath"], "/p/bin/Debug/net8.0/App.dll\n").unwrap();
        assert_eq!(one["TargetPath"], "/p/bin/Debug/net8.0/App.dll");

        let json = r#"{
  "Properties": {
    "TargetFramework": "",
    "TargetFrameworks": "net8.0;net10.0",
    "OutputType": "Exe",
    "TargetPath": "/p/bin/Debug/App.dll",
    "IsTestProject": "",
    "UsingMicrosoftNETSdkWeb": "true"
  }
}"#;
        let many = parse_get_property(&["TargetFramework", "TargetFrameworks"], json).unwrap();
        let p = ProjectProperties::from_values(many);
        assert_eq!(p.target_frameworks, vec!["net8.0", "net10.0"]);
        assert_eq!(p.target_framework, "net8.0");
        assert!(p.is_executable() && p.is_web && !p.is_test_project);
    }

    #[test]
    fn single_framework_fills_the_list() {
        let mut m = BTreeMap::new();
        m.insert("TargetFramework".to_string(), "net8.0".to_string());
        let p = ProjectProperties::from_values(m);
        assert_eq!(p.target_frameworks, vec!["net8.0"]);
    }
}
