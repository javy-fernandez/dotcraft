//! Loads the solution's reference graph away from the UI thread.

use std::path::PathBuf;

use rc_core::Runtime;
use rc_core::inbox::{Inbox, Waker};
use rc_dotnet::dependency_graph::DependencyGraph;

pub struct DependencyDiagram {
    paths: Vec<PathBuf>,
    generation: u64,
    inbox: Inbox<(u64, DependencyGraph)>,
    pub graph: Option<DependencyGraph>,
    pub loading: bool,
}

impl DependencyDiagram {
    pub fn new(waker: Waker) -> Self {
        Self { paths: Vec::new(), generation: 0, inbox: Inbox::new(waker), graph: None, loading: false }
    }

    pub fn request(&mut self, mut paths: Vec<PathBuf>) {
        paths.sort();
        paths.dedup();
        if self.paths == paths && (self.loading || self.graph.is_some()) {
            return;
        }
        self.paths = paths.clone();
        self.generation = self.generation.wrapping_add(1);
        self.graph = None;
        self.loading = !paths.is_empty();
        if !self.loading {
            return;
        }
        let out = self.inbox.sender();
        let generation = self.generation;
        Runtime::handle().spawn_blocking(move || {
            let graph = DependencyGraph::load(&paths);
            out.send((generation, graph));
        });
    }

    pub fn refresh(&mut self) {
        let paths = std::mem::take(&mut self.paths);
        self.request(paths);
    }

    pub fn update(&mut self) {
        for (generation, graph) in self.inbox.drain() {
            if generation == self.generation {
                self.graph = Some(graph);
                self.loading = false;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_refresh_result_does_not_replace_the_latest_graph() {
        let mut diagram = DependencyDiagram::new(Waker::noop());
        diagram.generation = 2;
        diagram.loading = true;
        diagram.inbox.sender().send((1, DependencyGraph::default()));
        diagram.update();
        assert!(diagram.graph.is_none());
        assert!(diagram.loading);
    }
}
