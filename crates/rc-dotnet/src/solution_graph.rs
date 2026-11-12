//! Bounded solution project graph and Graphviz DOT export.

use std::path::PathBuf;

use crate::{ProjectInfo, Solution};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphNode {
    pub name: String,
    pub path: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphEdge {
    /// Index of the project that declares the reference.
    pub from: usize,
    /// Index of the referenced project.
    pub to: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SolutionGraph {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    /// References outside the loaded solution, including projects excluded by a `.slnf`.
    pub external_references: Vec<(usize, PathBuf)>,
}

impl SolutionGraph {
    /// Reads only projects in `solution`; each project file is limited to 1 MiB.
    pub fn load(solution: &Solution) -> anyhow::Result<Self> {
        let projects = solution.projects();
        anyhow::ensure!(projects.len() <= 512, "Solution graph exceeds 512 projects");
        let mut graph = SolutionGraph::default();
        for project in &projects {
            anyhow::ensure!(project.name.len() <= 256, "Solution graph project name exceeds 256 bytes");
            graph.nodes.push(GraphNode {
                name: project.name.clone(),
                path: dunce::canonicalize(&project.path)?,
            });
        }
        for (from, node) in graph.nodes.iter().enumerate() {
            let bytes = crate::http_file::read_bounded_file(&node.path, 1024 * 1024)?;
            let text = String::from_utf8(bytes)?;
            let info = ProjectInfo::parse(text.trim_start_matches('\u{feff}'), &node.path)?;
            for reference in info.project_refs {
                let path = dunce::canonicalize(&reference).unwrap_or(reference);
                if let Some(to) = graph.nodes.iter().position(|n| n.path == path) {
                    graph.edges.push(GraphEdge { from, to });
                } else {
                    graph.external_references.push((from, path));
                }
                anyhow::ensure!(graph.edges.len() + graph.external_references.len() <= 10000, "Solution graph exceeds 10000 references");
            }
        }
        graph.edges.sort_by_key(|e| (e.from, e.to));
        graph.edges.dedup();
        graph.external_references.sort();
        graph.external_references.dedup();
        Ok(graph)
    }

    /// Returns the first project-index cycle, with its starting index repeated at the end.
    pub fn cycle(&self) -> Option<Vec<usize>> {
        let mut state = vec![0u8; self.nodes.len()];
        let mut stack = Vec::new();
        for start in 0..self.nodes.len() {
            if state[start] == 0 {
                if let Some(cycle) = cycle_from(self, start, &mut state, &mut stack) {
                    return Some(cycle);
                }
            }
        }
        None
    }

    /// Graphviz DOT text with project names as labels and references as directed edges.
    pub fn to_dot(&self) -> String {
        let mut out = String::from("digraph DotCraft {\n");
        for (index, node) in self.nodes.iter().enumerate() {
            out.push_str(&format!("  n{index} [label=\"{}\"];\n", escape_dot(&node.name)));
        }
        for edge in &self.edges {
            out.push_str(&format!("  n{} -> n{};\n", edge.from, edge.to));
        }
        out.push_str("}\n");
        out
    }
}

fn cycle_from(graph: &SolutionGraph, node: usize, state: &mut [u8], stack: &mut Vec<usize>) -> Option<Vec<usize>> {
    state[node] = 1;
    stack.push(node);
    for edge in graph.edges.iter().filter(|e| e.from == node) {
        if edge.to >= state.len() { continue; }
        match state[edge.to] {
            0 => if let Some(cycle) = cycle_from(graph, edge.to, state, stack) { return Some(cycle) },
            1 => {
                let start = stack.iter().position(|n| *n == edge.to)?;
                let mut cycle = stack[start..].to_vec();
                cycle.push(edge.to);
                return Some(cycle);
            }
            _ => {}
        }
    }
    stack.pop();
    state[node] = 2;
    None
}

fn escape_dot(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if c.is_control() => out.push(' '),
            _ => out.push(c),
        }
    }
    out
}
