//! Static project-to-package graph from direct NuGet references and CPM previews.

use std::collections::HashSet;
use std::path::PathBuf;

use crate::cpm::ProjectCpm;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackageNode {
    pub id: String,
    /// Declared or centrally selected version; `None` when it cannot be resolved statically.
    pub version: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VersionSource {
    Project,
    Override,
    Central(PathBuf),
    Unresolved,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackageEdge {
    pub project: usize,
    pub package: usize,
    pub source: VersionSource,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NugetGraph {
    pub projects: Vec<PathBuf>,
    pub packages: Vec<PackageNode>,
    pub edges: Vec<PackageEdge>,
}

impl NugetGraph {
    /// Builds only direct references; no restore assets or transitive closure are read.
    pub fn from_projects(projects: &[ProjectCpm]) -> anyhow::Result<Self> {
        anyhow::ensure!(projects.len() <= 512, "NuGet graph exceeds 512 projects");
        let mut graph = NugetGraph { projects: projects.iter().map(|p| p.project.clone()).collect(), ..Default::default() };
        let mut seen = HashSet::new();
        for (project, cpm) in projects.iter().enumerate() {
            anyhow::ensure!(cpm.references.len() + cpm.globals.len() <= 10000, "NuGet graph has too many references in a project");
            for reference in &cpm.references {
                let central = cpm.global(&reference.id).or_else(|| cpm.central_version(&reference.id));
                let source = if let Some(global) = cpm.global(&reference.id) {
                    VersionSource::Central(global.file.clone())
                } else if cpm.enabled && reference.version_override.is_some() {
                    VersionSource::Override
                } else if reference.version.is_some() {
                    VersionSource::Project
                } else if cpm.enabled {
                    central.map(|v| VersionSource::Central(v.file.clone())).unwrap_or(VersionSource::Unresolved)
                } else {
                    VersionSource::Unresolved
                };
                let version = cpm.effective_version(&reference.id).filter(|v| !v.is_empty());
                graph.add_edge(project, &reference.id, version, source, &mut seen)?;
            }
            if cpm.enabled {
                for global in &cpm.globals {
                    graph.add_edge(project, &global.id, Some(global.version.clone()), VersionSource::Central(global.file.clone()), &mut seen)?;
                }
            }
        }
        Ok(graph)
    }

    fn add_edge(&mut self, project: usize, id: &str, version: Option<String>, source: VersionSource, seen: &mut HashSet<(usize, usize)>) -> anyhow::Result<()> {
        anyhow::ensure!(id.len() <= 256 && !id.is_empty(), "NuGet package ID is invalid or oversized");
        anyhow::ensure!(version.as_ref().is_none_or(|v| v.len() <= 256), "NuGet package version is oversized");
        let package = match self.packages.iter().position(|p| p.id.eq_ignore_ascii_case(id) && p.version == version) {
            Some(index) => index,
            None => {
                anyhow::ensure!(self.packages.len() < 10000, "NuGet graph exceeds 10000 package nodes");
                self.packages.push(PackageNode { id: id.to_string(), version });
                self.packages.len() - 1
            }
        };
        if seen.insert((project, package)) {
            anyhow::ensure!(self.edges.len() < 10000, "NuGet graph exceeds 10000 edges");
            self.edges.push(PackageEdge { project, package, source });
        }
        Ok(())
    }

    pub fn packages_for(&self, project: usize) -> impl Iterator<Item = (&PackageNode, &VersionSource)> {
        self.edges.iter().filter(move |edge| edge.project == project).map(|edge| (&self.packages[edge.package], &edge.source))
    }
}
