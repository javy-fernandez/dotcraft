//! Hierarchy tool window: Call Hierarchy (⌃⌥H, callers or callees of a method) and Type
//! Hierarchy (⌃H, base types or derived types), as in Rider. The tree loads lazily: each node
//! asks the C# server for its children when it is expanded.

use serde_json::Value;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HierarchyKind {
    Calls,
    Types,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    /// Callers (calls) or base types (types).
    Up,
    /// Callees (calls) or derived types (types).
    Down,
}

impl HierarchyKind {
    pub fn children_method(self, dir: Direction) -> &'static str {
        match (self, dir) {
            (HierarchyKind::Calls, Direction::Up) => "callHierarchy/incomingCalls",
            (HierarchyKind::Calls, Direction::Down) => "callHierarchy/outgoingCalls",
            (HierarchyKind::Types, Direction::Up) => "typeHierarchy/supertypes",
            (HierarchyKind::Types, Direction::Down) => "typeHierarchy/subtypes",
        }
    }

    pub fn prepare_method(self) -> &'static str {
        match self {
            HierarchyKind::Calls => "textDocument/prepareCallHierarchy",
            HierarchyKind::Types => "textDocument/prepareTypeHierarchy",
        }
    }
}

#[derive(Clone, Debug)]
pub struct HNode {
    /// The server's item, sent back to ask for the children.
    pub item: Value,
    pub name: String,
    pub detail: String,
    pub kind: u32,
    pub path: Option<PathBuf>,
    pub line: usize,
    pub col: usize,
    pub depth: usize,
    /// `None` until loaded.
    pub children: Option<Vec<usize>>,
    pub expanded: bool,
}

impl HNode {
    pub fn from_item(item: Value, depth: usize) -> HNode {
        let s = |k: &str| item.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let path = item.get("uri").and_then(Value::as_str).and_then(|u| u.parse::<rc_lsp::lsp_types::Uri>().ok()).and_then(|u| rc_lsp::uri_to_path(&u));
        let pos = item.get("selectionRange").or_else(|| item.get("range")).and_then(|r| r.get("start"));
        let num = |k: &str| pos.and_then(|p| p.get(k)).and_then(Value::as_u64).unwrap_or(0) as usize;
        HNode { name: s("name"), detail: s("detail"), kind: item.get("kind").and_then(Value::as_u64).unwrap_or(0) as u32, path, line: num("line"), col: num("character"), depth, children: None, expanded: false, item }
    }
}

/// The tree shown in the window.
#[derive(Clone, Debug)]
pub struct Hierarchy {
    pub kind: HierarchyKind,
    pub direction: Direction,
    pub nodes: Vec<HNode>,
    pub roots: Vec<usize>,
    pub selected: Option<usize>,
    /// Bumped on each new tree, so late answers for an old one are dropped.
    pub generation: u64,
}

impl Hierarchy {
    pub fn new(kind: HierarchyKind, items: Vec<Value>, generation: u64) -> Self {
        let direction = if kind == HierarchyKind::Calls { Direction::Up } else { Direction::Down };
        let nodes: Vec<HNode> = items.into_iter().map(|i| HNode::from_item(i, 0)).collect();
        let roots = (0..nodes.len()).collect();
        Hierarchy { kind, direction, nodes, roots, selected: Some(0), generation }
    }

    /// Children of a call hierarchy answer: `from` (incoming) or `to` (outgoing) items.
    pub fn set_children(&mut self, parent: usize, items: Vec<Value>) {
        let Some(depth) = self.nodes.get(parent).map(|n| n.depth + 1) else { return };
        let mut ids = Vec::new();
        for it in items {
            let item = it.get("from").or_else(|| it.get("to")).cloned().unwrap_or(it);
            self.nodes.push(HNode::from_item(item, depth));
            ids.push(self.nodes.len() - 1);
        }
        self.nodes[parent].children = Some(ids);
    }

    /// Switching callers/callees (or base/derived types) rebuilds the tree from the roots.
    pub fn set_direction(&mut self, dir: Direction) {
        if self.direction == dir {
            return;
        }
        self.direction = dir;
        let roots: Vec<HNode> = self.roots.iter().map(|&r| self.nodes[r].clone()).map(|mut n| {
            n.children = None;
            n.expanded = false;
            n
        }).collect();
        self.nodes = roots;
        self.roots = (0..self.nodes.len()).collect();
        self.selected = Some(0);
        self.generation += 1;
    }

    /// Visible rows: depth-first through the expanded nodes.
    pub fn rows(&self) -> Vec<usize> {
        fn walk(h: &Hierarchy, id: usize, out: &mut Vec<usize>) {
            out.push(id);
            if h.nodes[id].expanded
                && let Some(children) = &h.nodes[id].children
            {
                for &c in children {
                    walk(h, c, out);
                }
            }
        }
        let mut out = Vec::new();
        for &r in &self.roots {
            walk(self, r, &mut out);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn item(name: &str, line: u64) -> Value {
        json!({ "name": name, "kind": 6, "detail": "Shop.Core", "uri": "file:///tmp/A.cs", "range": {}, "selectionRange": { "start": { "line": line, "character": 4 }, "end": { "line": line, "character": 9 } } })
    }

    #[test]
    fn builds_the_tree_from_server_items() {
        let mut h = Hierarchy::new(HierarchyKind::Calls, vec![item("Total", 3)], 1);
        assert_eq!(h.nodes[0].line, 3);
        assert_eq!(h.nodes[0].path.as_deref(), Some(std::path::Path::new("/tmp/A.cs")));
        h.set_children(0, vec![json!({ "from": item("Checkout", 10), "fromRanges": [] }), json!({ "from": item("Print", 20), "fromRanges": [] })]);
        assert_eq!(h.rows(), vec![0], "collapsed");
        h.nodes[0].expanded = true;
        assert_eq!(h.rows(), vec![0, 1, 2]);
        assert_eq!(h.nodes[2].name, "Print");
        assert_eq!(h.nodes[2].depth, 1);
        h.set_direction(Direction::Down);
        assert_eq!(h.rows(), vec![0]);
        assert!(h.nodes[0].children.is_none());
    }
}
