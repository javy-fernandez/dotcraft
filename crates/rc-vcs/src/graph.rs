//! Lanes of the log graph, computed from each commit's parents (in log order, children
//! before parents). Free lanes are reused, so the graph stays narrow.

use crate::log::Commit;

/// Which half of a row an edge is drawn in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Half {
    /// From the row's top edge (column `from`) to its middle (column `to`).
    Upper,
    /// From the row's middle (column `from`) to its bottom edge (column `to`).
    Lower,
}

/// One line segment of a row.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Edge {
    pub half: Half,
    pub from: usize,
    pub to: usize,
    /// Stable colour index of the lane the segment belongs to.
    pub color: usize,
}

/// What to draw in the row of one commit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphRow {
    /// Column of the commit's dot.
    pub column: usize,
    pub color: usize,
    pub edges: Vec<Edge>,
    /// Columns used by this row (for the graph's width).
    pub width: usize,
}

/// Incremental lane assignment: feed commits page by page in log order.
#[derive(Clone, Debug, Default)]
pub struct Graph {
    /// Per column: the commit hash the lane is waiting for, with its colour.
    lanes: Vec<Option<(String, usize)>>,
    next_color: usize,
}

impl Graph {
    pub fn new() -> Graph {
        Graph::default()
    }

    /// The rows of `commits`, continuing from the previous call.
    pub fn rows(&mut self, commits: &[Commit]) -> Vec<GraphRow> {
        commits.iter().map(|c| self.row(&c.hash, &c.parents)).collect()
    }

    /// The row of one commit given its parents.
    pub fn row(&mut self, hash: &str, parents: &[String]) -> GraphRow {
        let mut edges = Vec::new();
        let matching: Vec<usize> = self.lanes.iter().enumerate().filter(|(_, l)| l.as_ref().is_some_and(|(h, _)| h == hash)).map(|(i, _)| i).collect();
        let (column, color) = match matching.first() {
            Some(&i) => (i, self.lanes[i].as_ref().expect("lane").1),
            None => {
                let color = self.new_color();
                let i = self.free_lane();
                (i, color)
            }
        };
        // Upper half: lanes passing through, and lanes merging into this commit.
        for (i, lane) in self.lanes.iter().enumerate() {
            if let Some((h, c)) = lane {
                let to = if h == hash { column } else { i };
                edges.push(Edge { half: Half::Upper, from: i, to, color: *c });
            }
        }
        for &i in &matching {
            self.lanes[i] = None;
        }
        // Lower half: the first parent continues in this column, others join or open lanes.
        let mut parent_cols = Vec::new();
        let mut joins = Vec::new();
        for (n, p) in parents.iter().enumerate() {
            let awaited = self.lanes.iter().position(|l| l.as_ref().is_some_and(|(h, _)| h == p));
            if n == 0
                && let Some(j) = awaited
                && j > column
            {
                // A lane further right waits for the same parent: it joins this one, which
                // keeps the graph left-aligned.
                let c = self.lanes[j].take().expect("lane").1;
                joins.push(Edge { half: Half::Lower, from: j, to: column, color: c });
                self.set_lane(column, p.clone(), color);
                parent_cols.push((column, color));
            } else if let Some(i) = awaited {
                parent_cols.push((i, self.lanes[i].as_ref().expect("lane").1));
            } else if n == 0 {
                self.set_lane(column, p.clone(), color);
                parent_cols.push((column, color));
            } else {
                let c = self.new_color();
                let i = self.free_lane();
                self.set_lane(i, p.clone(), c);
                parent_cols.push((i, c));
            }
        }
        for (i, lane) in self.lanes.iter().enumerate() {
            if let Some((_, c)) = lane
                && !parent_cols.iter().any(|(pc, _)| *pc == i)
            {
                edges.push(Edge { half: Half::Lower, from: i, to: i, color: *c });
            }
        }
        for (i, c) in parent_cols {
            edges.push(Edge { half: Half::Lower, from: column, to: i, color: if i == column { color } else { c } });
        }
        edges.extend(joins);
        while self.lanes.last().is_some_and(Option::is_none) {
            self.lanes.pop();
        }
        let width = edges.iter().map(|e| e.from.max(e.to) + 1).max().unwrap_or(0).max(column + 1);
        GraphRow { column, color, edges, width }
    }

    fn new_color(&mut self) -> usize {
        self.next_color += 1;
        self.next_color - 1
    }

    fn free_lane(&mut self) -> usize {
        match self.lanes.iter().position(Option::is_none) {
            Some(i) => i,
            None => {
                self.lanes.push(None);
                self.lanes.len() - 1
            }
        }
    }

    fn set_lane(&mut self, i: usize, hash: String, color: usize) {
        if i >= self.lanes.len() {
            self.lanes.resize(i + 1, None);
        }
        self.lanes[i] = Some((hash, color));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn branch_and_merge() {
        // m merges b into a; a and b both come from r.
        let mut g = Graph::new();
        let m = g.row("m", &p(&["a", "b"]));
        assert_eq!(m.column, 0);
        assert!(m.edges.contains(&Edge { half: Half::Lower, from: 0, to: 1, color: 1 }));
        let a = g.row("a", &p(&["r"]));
        assert_eq!(a.column, 0);
        assert!(a.edges.iter().any(|e| e.half == Half::Upper && e.from == 1 && e.to == 1));
        let b = g.row("b", &p(&["r"]));
        assert_eq!(b.column, 1);
        // b's parent r is already awaited in lane 0: b's lane joins it.
        assert!(b.edges.contains(&Edge { half: Half::Lower, from: 1, to: 0, color: 0 }));
        let r = g.row("r", &[]);
        assert_eq!(r.column, 0);
        assert_eq!(r.width, 1);
        assert!(r.edges.iter().all(|e| e.half == Half::Upper));
    }
}
