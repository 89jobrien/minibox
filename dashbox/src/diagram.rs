// dashbox/src/diagram.rs — composable node/edge primitives

/// The semantic kind of a node — used for icon selection and filtering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    Branch,
    Job,
    Command,
    Artifact,
}

impl NodeKind {
    pub fn label(self) -> &'static str {
        match self {
            NodeKind::Branch => "branch",
            NodeKind::Job => "ci job",
            NodeKind::Command => "command",
            NodeKind::Artifact => "artifact",
        }
    }

    #[allow(dead_code)]
    pub fn icon(self) -> &'static str {
        match self {
            NodeKind::Branch => "⎇",
            NodeKind::Job => "⚙",
            NodeKind::Command => "$",
            NodeKind::Artifact => "◈",
        }
    }
}

/// Visual style of a directed edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeStyle {
    /// Automatic / triggered: `──────►`
    Solid,
    /// Tag / artifact release: `─ ─ ─ ►`
    Dashed,
    /// Manual gate (workflow_dispatch): `·  ·  ·►`
    Manual,
}

/// Live CI status overlaid on a node at render time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NodeStatus {
    #[default]
    Unknown,
    Passing,
    Failing,
    Running,
}

/// A single node in a diagram.  Nodes are defined independently of layout.
pub struct Node {
    pub id: usize,
    pub label: &'static str,
    pub detail: &'static str,
    pub kind: NodeKind,
}

/// A directed edge between two nodes.  Edges are defined independently of layout.
pub struct Edge {
    pub from: usize,
    pub to: usize,
    pub label: Option<&'static str>,
    pub style: EdgeStyle,
}

/// A diagram composes pre-defined nodes and edges into a 2-D grid for rendering.
///
/// `layout[row]` = ordered list of node IDs appearing left-to-right in that row.
/// Nodes and edges that appear in the layout must already exist in `nodes`/`edges`.
pub struct Diagram {
    #[allow(dead_code)]
    pub name: &'static str,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    pub layout: Vec<Vec<usize>>,
}

#[derive(Debug, Clone, Copy)]
pub enum NavDir {
    Up,
    Down,
    Left,
    Right,
}

impl Diagram {
    pub fn node(&self, id: usize) -> Option<&Node> {
        self.nodes.iter().find(|n| n.id == id)
    }

    pub fn edge_between(&self, from: usize, to: usize) -> Option<&Edge> {
        self.edges.iter().find(|e| e.from == from && e.to == to)
    }

    pub fn outgoing(&self, from: usize) -> Vec<&Edge> {
        self.edges.iter().filter(|e| e.from == from).collect()
    }

    pub fn position_of(&self, id: usize) -> Option<(usize, usize)> {
        for (r, row) in self.layout.iter().enumerate() {
            for (c, &nid) in row.iter().enumerate() {
                if nid == id {
                    return Some((r, c));
                }
            }
        }
        None
    }

    pub fn first_node(&self) -> Option<usize> {
        self.layout.iter().flatten().copied().next()
    }

    pub fn navigate(&self, id: usize, dir: NavDir) -> Option<usize> {
        let (row, col) = self.position_of(id)?;
        match dir {
            NavDir::Left if col > 0 => self.layout[row].get(col - 1).copied(),
            NavDir::Right => self.layout[row].get(col + 1).copied(),
            NavDir::Up if row > 0 => {
                let c = col.min(self.layout[row - 1].len().saturating_sub(1));
                self.layout[row - 1].get(c).copied()
            }
            NavDir::Down if row + 1 < self.layout.len() => {
                let c = col.min(self.layout[row + 1].len().saturating_sub(1));
                self.layout[row + 1].get(c).copied()
            }
            _ => None,
        }
    }

    pub fn next_node(&self, id: usize) -> usize {
        let all: Vec<usize> = self.layout.iter().flatten().copied().collect();
        let pos = all.iter().position(|&n| n == id).unwrap_or(0);
        all[(pos + 1) % all.len()]
    }

    pub fn prev_node(&self, id: usize) -> usize {
        let all: Vec<usize> = self.layout.iter().flatten().copied().collect();
        let pos = all.iter().position(|&n| n == id).unwrap_or(0);
        all[(pos + all.len() - 1) % all.len()]
    }

    /// Returns true if any node in `prev_row` has an edge to any node in `next_row`.
    pub fn has_cross_row_edge(&self, prev_row: &[usize], next_row: &[usize]) -> bool {
        prev_row
            .iter()
            .any(|&f| next_row.iter().any(|&t| self.edge_between(f, t).is_some()))
    }
}
