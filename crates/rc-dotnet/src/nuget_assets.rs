//! Resolved NuGet package dependencies from a project's restore assets.

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssetsGraph {
    pub project: PathBuf,
    pub framework: String,
    pub packages: Vec<PackageNode>,
    pub edges: Vec<PackageEdge>,
}

/// Reads the graph produced by the most recent successful `dotnet restore`.
pub fn load(project: &Path) -> anyhow::Result<AssetsGraph> {
    let assets = project
        .parent()
        .ok_or_else(|| anyhow::anyhow!("project has no parent directory"))?
        .join("obj/project.assets.json");
    let len = std::fs::metadata(&assets)?.len();
    anyhow::ensure!(
        len <= MAX_ASSETS_BYTES,
        "NuGet assets exceed the {MAX_ASSETS_BYTES}-byte limit"
    );
    let text = std::fs::read_to_string(&assets)?;
    anyhow::ensure!(
        text.len() as u64 <= MAX_ASSETS_BYTES,
        "NuGet assets exceed the {MAX_ASSETS_BYTES}-byte limit"
    );
    parse(project, &text)
}

/// Parses one target framework from NuGet's restore assets. Project and framework references
/// are excluded; edges connect resolved package versions only.
pub fn parse(project: &Path, json: &str) -> anyhow::Result<AssetsGraph> {
    anyhow::ensure!(
        json.len() as u64 <= MAX_ASSETS_BYTES,
        "NuGet assets exceed the {MAX_ASSETS_BYTES}-byte limit"
    );
    let root: serde_json::Value = serde_json::from_str(json)?;
    let targets = root
        .get("targets")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| anyhow::anyhow!("NuGet assets have no targets"))?;
    let frameworks = root
        .pointer("/project/frameworks")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| anyhow::anyhow!("NuGet assets have no project frameworks"))?;
    let framework = frameworks
        .keys()
        .find(|name| targets.contains_key(*name))
        .ok_or_else(|| anyhow::anyhow!("NuGet assets have no matching target framework"))?
        .clone();
    let target = targets
        .get(&framework)
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| anyhow::anyhow!("NuGet target is invalid"))?;
    let direct = frameworks
        .get(&framework)
        .and_then(|f| f.get("dependencies"))
        .and_then(serde_json::Value::as_object);
    let mut packages = Vec::new();
    let mut dependencies = Vec::new();
    let mut by_id = HashMap::<String, usize>::new();
    for (key, value) in target {
        if value.get("type").and_then(serde_json::Value::as_str) != Some("package") {
            continue;
        }
        anyhow::ensure!(
            packages.len() < MAX_PACKAGES,
            "NuGet graph exceeds the {MAX_PACKAGES}-package limit"
        );
        let (id, version) = key
            .rsplit_once('/')
            .ok_or_else(|| anyhow::anyhow!("invalid NuGet package key: {key}"))?;
        anyhow::ensure!(
            !id.is_empty() && !version.is_empty(),
            "invalid NuGet package key: {key}"
        );
        let index = packages.len();
        let normalized = id.to_ascii_lowercase();
        anyhow::ensure!(
            by_id.insert(normalized, index).is_none(),
            "NuGet target resolves multiple versions of {id}"
        );
        let is_direct =
            direct.is_some_and(|deps| deps.keys().any(|name| name.eq_ignore_ascii_case(id)));
        packages.push(PackageNode {
            id: id.into(),
            version: version.into(),
            direct: is_direct,
        });
        dependencies.push(
            value
                .get("dependencies")
                .and_then(serde_json::Value::as_object)
                .map(|deps| deps.keys().cloned().collect::<Vec<_>>())
                .unwrap_or_default(),
        );
    }
    let mut edges = Vec::new();
    for (from, deps) in dependencies.into_iter().enumerate() {
        for name in deps {
            if let Some(to) = by_id.get(&name.to_ascii_lowercase()) {
                anyhow::ensure!(
                    edges.len() < MAX_EDGES,
                    "NuGet graph exceeds the {MAX_EDGES}-edge limit"
                );
                edges.push(PackageEdge { from, to: *to });
            }
        }
    }
    Ok(AssetsGraph {
        project: project.to_path_buf(),
        framework,
        packages,
        edges,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_direct_and_resolved_transitive_dependencies() {
        let json = r#"{"targets":{"net8.0":{"A/1.0.0":{"type":"package","dependencies":{"B":"[2.0.0, )"}},"B/2.1.0":{"type":"package"},"Project/1.0.0":{"type":"project"}}},"project":{"frameworks":{"net8.0":{"dependencies":{"A":{"version":"[1.0.0, )"}}}}}}"#;
        let graph = parse(Path::new("Shop.csproj"), json).unwrap();
        assert_eq!(graph.framework, "net8.0");
        assert_eq!(graph.packages.len(), 2);
        assert!(graph.packages.iter().any(|p| p.id == "A" && p.direct));
        assert!(
            graph
                .packages
                .iter()
                .any(|p| p.id == "B" && p.version == "2.1.0" && !p.direct)
        );
        assert_eq!(graph.edges, [PackageEdge { from: 0, to: 1 }]);
    }

    #[test]
    fn rejects_malformed_or_oversized_assets() {
        assert!(parse(Path::new("Shop.csproj"), "{}").is_err());
        assert!(
            parse(
                Path::new("Shop.csproj"),
                &" ".repeat(MAX_ASSETS_BYTES as usize + 1)
            )
            .is_err()
        );
        let ambiguous = r#"{"targets":{"net8.0":{"A/1.0.0":{"type":"package"},"A/2.0.0":{"type":"package"}}},"project":{"frameworks":{"net8.0":{}}}}"#;
        assert!(parse(Path::new("Shop.csproj"), ambiguous).is_err());
    }
}
