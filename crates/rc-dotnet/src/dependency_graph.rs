//! Project and NuGet references represented as a navigable graph.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::ProjectInfo;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DependencyNode {
    Project { path: PathBuf, name: String, in_solution: bool },
    Package { name: String, version: String },
}

#[derive(Clone, Debug, Default)]
pub struct DependencyGraph {
    pub nodes: Vec<DependencyNode>,
    /// Directed references from a project to another project or a package.
    pub edges: Vec<(usize, usize)>,
}

impl DependencyGraph {
    /// Load project files on a background task; callers should not run this on the UI thread.
    pub fn load(paths: &[PathBuf]) -> Self {
        let projects = paths.iter().filter_map(|path| ProjectInfo::load(path).ok()).collect();
        Self::from_projects(projects)
    }

    pub fn from_projects(projects: Vec<ProjectInfo>) -> Self {
        let mut graph = Self::default();
        let mut project_ids = BTreeMap::<PathBuf, usize>::new();
        let mut package_ids = BTreeMap::<(String, String), usize>::new();
        for project in &projects {
            project_ids.insert(project.path.clone(), graph.nodes.len());
            graph.nodes.push(DependencyNode::Project {
                path: project.path.clone(),
                name: project.name.clone(),
                in_solution: true,
            });
        }
        for project in &projects {
            let source = project_ids[&project.path];
            for reference in &project.project_refs {
                let target = *project_ids.entry(reference.clone()).or_insert_with(|| {
                    let id = graph.nodes.len();
                    graph.nodes.push(DependencyNode::Project {
                        path: reference.clone(),
                        name: display_name(reference),
                        in_solution: false,
                    });
                    id
                });
                graph.edges.push((source, target));
            }
            for package in &project.packages {
                let key = (package.name.clone(), package.version.clone());
                let target = *package_ids.entry(key.clone()).or_insert_with(|| {
                    let id = graph.nodes.len();
                    graph.nodes.push(DependencyNode::Package { name: key.0, version: key.1 });
                    id
                });
                graph.edges.push((source, target));
            }
        }
        graph
    }
}

fn display_name(path: &Path) -> String {
    path.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graph_links_solution_projects_and_deduplicates_shared_packages() {
        let root = Path::new("/work");
        let app = ProjectInfo::parse(
            r#"<Project Sdk="Microsoft.NET.Sdk"><ItemGroup><ProjectReference Include="../Lib/Lib.csproj"/><PackageReference Include="Newtonsoft.Json" Version="13.0.3"/></ItemGroup></Project>"#,
            &root.join("App/App.csproj"),
        ).unwrap();
        let lib = ProjectInfo::parse(
            r#"<Project Sdk="Microsoft.NET.Sdk"><ItemGroup><PackageReference Include="Newtonsoft.Json" Version="13.0.3"/></ItemGroup></Project>"#,
            &root.join("Lib/Lib.csproj"),
        ).unwrap();
        let graph = DependencyGraph::from_projects(vec![app, lib]);
        assert_eq!(graph.nodes.len(), 3);
        assert_eq!(graph.edges.len(), 3);
        assert!(graph.edges.contains(&(0, 1)));
        assert!(graph.edges.contains(&(0, 2)));
        assert!(graph.edges.contains(&(1, 2)));
    }
}
