//! Resolved NuGet package dependencies from a project's restore assets
//! (`project.assets.json`, written by `dotnet restore` in the project's intermediate folder).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

const MAX_ASSETS_BYTES: u64 = 16 * 1024 * 1024;
const MAX_PACKAGES: usize = 2_000;
const MAX_EDGES: usize = 10_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackageNode {
    pub id: String,
    pub version: String,
    pub direct: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackageEdge {
    pub from: usize,
    pub to: usize,
}

/// The packages resolved for one target framework.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameworkGraph {
    /// As the project names it (`net8.0`, `net8.0-windows`).
    pub framework: String,
    pub packages: Vec<PackageNode>,
    pub edges: Vec<PackageEdge>,
}

/// Every target framework of a project.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssetsGraph {
    pub project: PathBuf,
    pub frameworks: Vec<FrameworkGraph>,
}

/// Where restore writes `project.assets.json`: `MSBuildProjectExtensionsPath` when the project
/// or its `Directory.Build.props` sets it, else `BaseIntermediateOutputPath`, else `obj/`.
/// Read statically (no MSBuild): conditions are ignored and only the usual macros expand.
pub fn assets_path(project: &Path) -> PathBuf {
    let dir = project.parent().unwrap_or(Path::new("."));
    let name = project.file_stem().and_then(|s| s.to_str()).unwrap_or_default();
    // Directory.Build.props is imported before the SDK computes the defaults, so it wins over
    // the project body (where only non-SDK projects can still change them).
    let mut files = Vec::new();
    if let Some(props) = dir.ancestors().map(|d| d.join("Directory.Build.props")).find(|p| p.is_file()) {
        files.push(props);
    }
    files.push(project.to_path_buf());
    let read = |prop: &str, base: Option<&str>| {
        files.iter().find_map(|file| {
            let text = std::fs::read_to_string(file).ok()?;
            let doc = roxmltree::Document::parse(text.strip_prefix('\u{feff}').unwrap_or(&text)).ok()?;
            let value = doc.descendants().filter(|n| n.has_tag_name(prop)).filter_map(|n| n.text()).map(str::trim).rfind(|v| !v.is_empty())?;
            let this = file.parent().unwrap_or(dir);
            expand(value, dir, this, name, base)
        })
    };
    let base = read("BaseIntermediateOutputPath", None);
    let extensions = read("MSBuildProjectExtensionsPath", base.as_deref()).or(base);
    let folder = match extensions {
        Some(path) => dir.join(path.replace('\\', "/")),
        None => dir.join("obj"),
    };
    folder.join("project.assets.json")
}

/// Expands the macros a path property usually holds; `None` when another one is left.
fn expand(value: &str, project_dir: &Path, this_dir: &Path, name: &str, base: Option<&str>) -> Option<String> {
    let slash = |p: &Path| format!("{}/", p.display());
    let mut out = value
        .replace("$(MSBuildProjectDirectory)", &project_dir.display().to_string())
        .replace("$(MSBuildThisFileDirectory)", &slash(this_dir))
        .replace("$(MSBuildProjectName)", name);
    if let Some(base) = base {
        out = out.replace("$(BaseIntermediateOutputPath)", base);
    }
    (!out.contains("$(")).then_some(out)
}

/// Reads the graph of the most recent successful `dotnet restore`; `None` when the project has
/// not been restored yet.
pub fn load(project: &Path) -> anyhow::Result<Option<AssetsGraph>> {
    let assets = assets_path(project);
    let len = match std::fs::metadata(&assets) {
        Ok(m) => m.len(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    anyhow::ensure!(len <= MAX_ASSETS_BYTES, "NuGet assets exceed the {MAX_ASSETS_BYTES}-byte limit");
    let text = std::fs::read_to_string(&assets)?;
    parse(project, text.strip_prefix('\u{feff}').unwrap_or(&text)).map(Some)
}

/// Parses every target framework of NuGet's restore assets (runtime-specific targets such as
/// `net8.0/win-x64` are skipped). Project and framework references are excluded; edges connect
/// resolved package versions only.
pub fn parse(project: &Path, json: &str) -> anyhow::Result<AssetsGraph> {
    anyhow::ensure!(json.len() as u64 <= MAX_ASSETS_BYTES, "NuGet assets exceed the {MAX_ASSETS_BYTES}-byte limit");
    let root: serde_json::Value = serde_json::from_str(json)?;
    let targets = root.get("targets").and_then(serde_json::Value::as_object).ok_or_else(|| anyhow::anyhow!("NuGet assets have no targets"))?;
    let declared = root.pointer("/project/frameworks").and_then(serde_json::Value::as_object).ok_or_else(|| anyhow::anyhow!("NuGet assets have no project frameworks"))?;
    let mut frameworks = Vec::new();
    // Declared frameworks first (their key is the target's; `targetAlias` is the project's name
    // for it, e.g. `net8.0-windows` for `net8.0-windows7.0`), then any other target.
    let mut names: Vec<&String> = declared.keys().filter(|k| targets.contains_key(*k)).collect();
    names.extend(targets.keys().filter(|k| !k.contains('/') && !declared.contains_key(*k)));
    for name in names {
        let target = targets.get(name).and_then(serde_json::Value::as_object).ok_or_else(|| anyhow::anyhow!("NuGet target {name} is invalid"))?;
        let info = declared.get(name);
        let alias = info.and_then(|f| f.get("targetAlias")).and_then(serde_json::Value::as_str).filter(|a| !a.is_empty());
        let direct = info.and_then(|f| f.get("dependencies")).and_then(serde_json::Value::as_object);
        let (packages, edges) = framework_graph(target, direct)?;
        frameworks.push(FrameworkGraph { framework: alias.unwrap_or(name).to_string(), packages, edges });
    }
    anyhow::ensure!(!frameworks.is_empty(), "NuGet assets have no matching target framework");
    Ok(AssetsGraph { project: project.to_path_buf(), frameworks })
}

type Target = serde_json::Map<String, serde_json::Value>;

fn framework_graph(target: &Target, direct: Option<&Target>) -> anyhow::Result<(Vec<PackageNode>, Vec<PackageEdge>)> {
    let mut packages = Vec::new();
    let mut dependencies = Vec::new();
    let mut by_id = HashMap::<String, usize>::new();
    for (key, value) in target {
        if value.get("type").and_then(serde_json::Value::as_str) != Some("package") {
            continue;
        }
        anyhow::ensure!(packages.len() < MAX_PACKAGES, "NuGet graph exceeds the {MAX_PACKAGES}-package limit");
        let (id, version) = key.rsplit_once('/').ok_or_else(|| anyhow::anyhow!("invalid NuGet package key: {key}"))?;
        anyhow::ensure!(!id.is_empty() && !version.is_empty(), "invalid NuGet package key: {key}");
        let index = packages.len();
        anyhow::ensure!(by_id.insert(id.to_ascii_lowercase(), index).is_none(), "NuGet target resolves multiple versions of {id}");
        let is_direct = direct.is_some_and(|deps| deps.keys().any(|name| name.eq_ignore_ascii_case(id)));
        packages.push(PackageNode { id: id.into(), version: version.into(), direct: is_direct });
        dependencies.push(value.get("dependencies").and_then(serde_json::Value::as_object).map(|deps| deps.keys().cloned().collect::<Vec<_>>()).unwrap_or_default());
    }
    let mut edges = Vec::new();
    for (from, deps) in dependencies.into_iter().enumerate() {
        for name in deps {
            if let Some(to) = by_id.get(&name.to_ascii_lowercase()) {
                anyhow::ensure!(edges.len() < MAX_EDGES, "NuGet graph exceeds the {MAX_EDGES}-edge limit");
                edges.push(PackageEdge { from, to: *to });
            }
        }
    }
    Ok((packages, edges))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_direct_and_resolved_transitive_dependencies() {
        let json = r#"{"targets":{"net8.0":{"A/1.0.0":{"type":"package","dependencies":{"B":"[2.0.0, )"}},"B/2.1.0":{"type":"package"},"Project/1.0.0":{"type":"project"}}},"project":{"frameworks":{"net8.0":{"dependencies":{"A":{"version":"[1.0.0, )"}}}}}}"#;
        let graph = parse(Path::new("Shop.csproj"), json).unwrap();
        let [net8] = graph.frameworks.as_slice() else { panic!("{graph:?}") };
        assert_eq!(net8.framework, "net8.0");
        assert_eq!(net8.packages.len(), 2);
        assert!(net8.packages.iter().any(|p| p.id == "A" && p.direct));
        assert!(net8.packages.iter().any(|p| p.id == "B" && p.version == "2.1.0" && !p.direct));
        assert_eq!(net8.edges, [PackageEdge { from: 0, to: 1 }]);
    }

    #[test]
    fn every_target_framework_is_parsed_under_the_project_alias() {
        let json = r#"{
          "targets": {
            "net48": {"Old/1.0.0": {"type": "package"}},
            "net8.0-windows7.0": {"New/2.0.0": {"type": "package", "dependencies": {"Dep": "1.0.0"}}, "Dep/1.0.0": {"type": "package"}},
            "net8.0-windows7.0/win-x64": {"New/2.0.0": {"type": "package"}}
          },
          "project": {"frameworks": {
            "net8.0-windows7.0": {"targetAlias": "net8.0-windows", "dependencies": {"New": {}}},
            "net48": {"targetAlias": "net48", "dependencies": {"Old": {}}}
          }}
        }"#;
        let graph = parse(Path::new("App.csproj"), json).unwrap();
        let names: Vec<_> = graph.frameworks.iter().map(|f| f.framework.as_str()).collect();
        assert_eq!(names, ["net48", "net8.0-windows"]);
        let windows = &graph.frameworks[1];
        assert!(windows.packages.iter().any(|p| p.id == "New" && p.direct));
        assert!(windows.packages.iter().any(|p| p.id == "Dep" && !p.direct));
        assert_eq!(windows.edges.len(), 1);
        assert_eq!(graph.frameworks[0].packages, [PackageNode { id: "Old".into(), version: "1.0.0".into(), direct: true }]);
    }

    #[test]
    fn rejects_malformed_or_oversized_assets() {
        assert!(parse(Path::new("Shop.csproj"), "{}").is_err());
        assert!(parse(Path::new("Shop.csproj"), &" ".repeat(MAX_ASSETS_BYTES as usize + 1)).is_err());
        let ambiguous = r#"{"targets":{"net8.0":{"A/1.0.0":{"type":"package"},"A/2.0.0":{"type":"package"}}},"project":{"frameworks":{"net8.0":{}}}}"#;
        assert!(parse(Path::new("Shop.csproj"), ambiguous).is_err());
    }

    const ASSETS: &str = r#"{"targets":{"net8.0":{"A/1.0.0":{"type":"package"}}},"project":{"frameworks":{"net8.0":{"dependencies":{"A":{}}}}}}"#;

    fn dir() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let root = dunce::canonicalize(tmp.path()).unwrap();
        (tmp, root)
    }

    #[test]
    fn assets_default_to_obj_and_unrestored_projects_have_none() {
        let (_tmp, root) = dir();
        let project = root.join("App/App.csproj");
        std::fs::create_dir_all(project.parent().unwrap()).unwrap();
        std::fs::write(&project, "<Project Sdk=\"Microsoft.NET.Sdk\" />").unwrap();
        assert_eq!(assets_path(&project), root.join("App/obj/project.assets.json"));
        assert_eq!(load(&project).unwrap(), None);
        std::fs::create_dir_all(root.join("App/obj")).unwrap();
        std::fs::write(root.join("App/obj/project.assets.json"), format!("\u{feff}{ASSETS}")).unwrap();
        assert_eq!(load(&project).unwrap().unwrap().frameworks[0].packages[0].id, "A");
    }

    #[test]
    fn assets_follow_the_intermediate_output_path() {
        let (_tmp, root) = dir();
        let project = root.join("src/App/App.csproj");
        std::fs::create_dir_all(project.parent().unwrap()).unwrap();
        std::fs::write(&project, "<Project Sdk=\"Microsoft.NET.Sdk\" />").unwrap();
        // BaseIntermediateOutputPath from Directory.Build.props, relative to the project.
        std::fs::write(root.join("Directory.Build.props"), r"<Project><PropertyGroup><BaseIntermediateOutputPath>$(MSBuildThisFileDirectory)artifacts\obj\$(MSBuildProjectName)\</BaseIntermediateOutputPath></PropertyGroup></Project>").unwrap();
        let expected = root.join("artifacts/obj/App/project.assets.json");
        assert_eq!(assets_path(&project), expected);
        std::fs::create_dir_all(expected.parent().unwrap()).unwrap();
        std::fs::write(&expected, ASSETS).unwrap();
        assert!(load(&project).unwrap().is_some());
        // MSBuildProjectExtensionsPath, when set, wins and may build on the base path.
        std::fs::write(&project, "<Project Sdk=\"Microsoft.NET.Sdk\"><PropertyGroup><MSBuildProjectExtensionsPath>$(BaseIntermediateOutputPath)ext/</MSBuildProjectExtensionsPath></PropertyGroup></Project>").unwrap();
        assert_eq!(assets_path(&project), root.join("artifacts/obj/App/ext/project.assets.json"));
        // A relative path is taken from the project directory.
        std::fs::write(root.join("Directory.Build.props"), "<Project><PropertyGroup><BaseIntermediateOutputPath>build/</BaseIntermediateOutputPath></PropertyGroup></Project>").unwrap();
        std::fs::write(&project, "<Project Sdk=\"Microsoft.NET.Sdk\" />").unwrap();
        assert_eq!(assets_path(&project), root.join("src/App/build/project.assets.json"));
        // Macros it cannot expand fall back to obj/.
        std::fs::write(root.join("Directory.Build.props"), "<Project><PropertyGroup><BaseIntermediateOutputPath>$(ArtifactsPath)/obj/</BaseIntermediateOutputPath></PropertyGroup></Project>").unwrap();
        assert_eq!(assets_path(&project), root.join("src/App/obj/project.assets.json"));
    }
}
