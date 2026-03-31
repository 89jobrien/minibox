# Mermaid Diagram Engine Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace hand-coded Rust diagram constructors in dashbox with a Mermaid parser that produces the same navigable `OwnedDiagram` domain model, while also supporting user-defined `.mmd` files from `~/.mbx/diagrams/`.

**Architecture:** Add `OwnedDiagram`/`OwnedNode`/`OwnedEdge` types with owned `String` fields (replacing static-str `Diagram`/`Node`/`Edge`); add a `mermaid.rs` parser that converts `&str` → `OwnedDiagram`; add a `source.rs` loader for embedded statics and filesystem files; convert all 6 built-in diagrams to `.mmd` files; update `DiagramsTab` to hold `Vec<OwnedDiagram>` replacing the 6 named fields and hardcoded `View` enum.

**Tech Stack:** Rust 2024, ratatui 0.29, no new dependencies.

---

## File Map

| Action | Path | Responsibility |
|--------|------|----------------|
| Create | `crates/dashbox/src/diagram/mod.rs` | `OwnedNode`, `OwnedEdge`, `OwnedDiagram`, `NodeKind`, `EdgeStyle`, `NavDir`, navigation methods |
| Create | `crates/dashbox/src/diagram/mermaid.rs` | `parse(&str) -> Result<OwnedDiagram, MermaidError>` |
| Create | `crates/dashbox/src/diagram/source.rs` | `DiagramSource` enum, `load_user_diagrams()` |
| Create | `crates/dashbox/src/diagrams/ci_flow.mmd` | CI Flow Mermaid source |
| Create | `crates/dashbox/src/diagrams/dev_loop.mmd` | Dev Loop Mermaid source |
| Create | `crates/dashbox/src/diagrams/container_lifecycle.mmd` | Container Lifecycle Mermaid source |
| Create | `crates/dashbox/src/diagrams/image_pull.mmd` | Image Pull Mermaid source |
| Create | `crates/dashbox/src/diagrams/adapter_suite.mmd` | Adapter Suite Mermaid source |
| Create | `crates/dashbox/src/diagrams/workspace_deps.mmd` | Workspace Deps Mermaid source |
| Delete | `crates/dashbox/src/diagram.rs` | Replaced by `diagram/mod.rs` |
| Replace | `crates/dashbox/src/diagrams.rs` | Replaced by `built_in_diagrams()` function + `pub use` |
| Modify | `crates/dashbox/src/tabs/diagrams.rs` | Use `Vec<OwnedDiagram>`, remove `View` enum, remove 6 named fields |
| Modify | `crates/dashbox/src/app.rs` | Pass user diagrams into `DiagramsTab::new()` |
| Modify | `crates/dashbox/src/main.rs` | No change needed (App::new() handles it) |

---

## Task 1: OwnedDiagram types + navigation

**Files:**
- Create: `crates/dashbox/src/diagram/mod.rs`
- Delete: `crates/dashbox/src/diagram.rs` (after copy)

- [ ] **Step 1: Create `crates/dashbox/src/diagram/` directory and `mod.rs`**

Create `crates/dashbox/src/diagram/mod.rs` with this complete content:

```rust
// dashbox/src/diagram/mod.rs

pub mod mermaid;
pub mod source;

/// The semantic kind of a node — used for icon selection and color coding.
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

/// A single node in a diagram. Uses owned strings for compatibility with
/// both embedded static diagrams and file-loaded user diagrams.
#[derive(Debug, Clone)]
pub struct OwnedNode {
    pub id: usize,
    pub label: String,
    pub detail: String,
    pub kind: NodeKind,
}

/// A directed edge between two nodes.
#[derive(Debug, Clone)]
pub struct OwnedEdge {
    pub from: usize,
    pub to: usize,
    pub label: Option<String>,
    pub style: EdgeStyle,
}

/// A navigable diagram. `layout[row]` = ordered node IDs left-to-right in that row.
#[derive(Debug, Clone)]
pub struct OwnedDiagram {
    pub name: String,
    pub nodes: Vec<OwnedNode>,
    pub edges: Vec<OwnedEdge>,
    pub layout: Vec<Vec<usize>>,
}

#[derive(Debug, Clone, Copy)]
pub enum NavDir {
    Up,
    Down,
    Left,
    Right,
}

impl OwnedDiagram {
    pub fn node(&self, id: usize) -> Option<&OwnedNode> {
        self.nodes.iter().find(|n| n.id == id)
    }

    pub fn edge_between(&self, from: usize, to: usize) -> Option<&OwnedEdge> {
        self.edges.iter().find(|e| e.from == from && e.to == to)
    }

    pub fn outgoing(&self, from: usize) -> Vec<&OwnedEdge> {
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
```

- [ ] **Step 2: Add empty stub files for submodules (so it compiles)**

Create `crates/dashbox/src/diagram/mermaid.rs`:
```rust
// stub — implemented in Task 2
```

Create `crates/dashbox/src/diagram/source.rs`:
```rust
// stub — implemented in Task 3
```

- [ ] **Step 3: Update `main.rs` to use new module path**

In `crates/dashbox/src/main.rs`, the `mod diagram;` line currently refers to `diagram.rs`. Since we're replacing it with `diagram/mod.rs`, the module declaration stays the same — Rust resolves either automatically. Delete `crates/dashbox/src/diagram.rs` now.

- [ ] **Step 4: Update all imports of old types throughout the codebase**

In `crates/dashbox/src/tabs/diagrams.rs`, the import line:
```rust
use crate::diagram::{Diagram, EdgeStyle, NavDir, NodeKind, NodeStatus};
```
Change to:
```rust
use crate::diagram::{EdgeStyle, NavDir, NodeKind, NodeStatus, OwnedDiagram};
```

In `crates/dashbox/src/diagrams.rs`, the import:
```rust
use crate::diagram::{Diagram, Edge, EdgeStyle, Node, NodeKind};
```
Change to:
```rust
use crate::diagram::{EdgeStyle, NodeKind, OwnedDiagram, OwnedEdge, OwnedNode};
```

- [ ] **Step 5: Verify it compiles (errors expected in diagrams.rs/tabs/diagrams.rs — that's fine)**

```bash
cargo check -p dashbox 2>&1 | head -40
```

Expected: errors about `Diagram`, `Node`, `Edge` not found — that's correct, we haven't migrated those files yet.

- [ ] **Step 6: Commit**

```bash
git add crates/dashbox/src/diagram/
git rm crates/dashbox/src/diagram.rs
git commit -m "feat(dashbox): add OwnedDiagram types with owned String fields"
```

---

## Task 2: Mermaid parser

**Files:**
- Modify: `crates/dashbox/src/diagram/mermaid.rs`

- [ ] **Step 1: Write failing tests first**

Replace `crates/dashbox/src/diagram/mermaid.rs` with:

```rust
// dashbox/src/diagram/mermaid.rs

use crate::diagram::{EdgeStyle, NodeKind, OwnedDiagram, OwnedEdge, OwnedNode};

#[derive(Debug)]
pub struct MermaidError {
    pub line: usize,
    pub message: String,
}

impl std::fmt::Display for MermaidError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "mermaid parse error at line {}: {}", self.line, self.message)
    }
}

/// Parse a Mermaid flowchart/graph source into an OwnedDiagram.
///
/// Supported syntax:
///   graph LR / flowchart LR         — declaration (direction ignored)
///   A[label]                        → NodeKind::Command
///   A([label])                      → NodeKind::Branch
///   A{label}                        → NodeKind::Job
///   A>label<                        → NodeKind::Artifact
///   A --> B                         → EdgeStyle::Solid
///   A -->|text| B                   → EdgeStyle::Solid with label
///   A -.-> B                        → EdgeStyle::Dashed
///   A -.->|text| B                  → EdgeStyle::Dashed with label
///   A ==> B                         → EdgeStyle::Manual
///   A ==>|text| B                   → EdgeStyle::Manual with label
///   %% detail: text                 — detail for most recently declared node
///   %% kind: branch|job|command|artifact  — override inferred kind
///   %% layout: A B C                — assign node IDs to a layout row
///   %% name: Human Title            — diagram name (defaults to "Diagram")
///
/// Parse errors produce a single-node diagram showing the error message.
pub fn parse(src: &str) -> OwnedDiagram {
    match try_parse(src) {
        Ok(d) => d,
        Err(e) => error_diagram(e),
    }
}

fn error_diagram(e: MermaidError) -> OwnedDiagram {
    OwnedDiagram {
        name: "Parse Error".to_string(),
        nodes: vec![OwnedNode {
            id: 0,
            label: "error".to_string(),
            detail: e.to_string(),
            kind: NodeKind::Job,
        }],
        edges: vec![],
        layout: vec![vec![0]],
    }
}

fn try_parse(src: &str) -> Result<OwnedDiagram, MermaidError> {
    let mut name = "Diagram".to_string();
    let mut nodes: Vec<OwnedNode> = Vec::new();
    let mut edges: Vec<OwnedEdge> = Vec::new();
    // Maps string ID (e.g. "A") to numeric usize id
    let mut id_map: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    // layout rows specified via %% layout: ...
    let mut layout_hints: Vec<Vec<String>> = Vec::new();
    // pending detail/kind for the last declared node string ID
    let mut last_node_str_id: Option<String> = None;

    for (line_idx, raw_line) in src.lines().enumerate() {
        let line = raw_line.trim();
        let lineno = line_idx + 1;

        if line.is_empty() {
            continue;
        }

        // ── Comment / metadata lines ──────────────────────────────────────
        if let Some(rest) = line.strip_prefix("%%") {
            let meta = rest.trim();
            if let Some(val) = meta.strip_prefix("name:") {
                name = val.trim().to_string();
            } else if let Some(val) = meta.strip_prefix("detail:") {
                let detail = val.trim().to_string();
                if let Some(ref sid) = last_node_str_id {
                    if let Some(&nid) = id_map.get(sid) {
                        if let Some(node) = nodes.iter_mut().find(|n| n.id == nid) {
                            node.detail = detail;
                        }
                    }
                }
            } else if let Some(val) = meta.strip_prefix("kind:") {
                let kind = parse_kind(val.trim());
                if let Some(ref sid) = last_node_str_id {
                    if let Some(&nid) = id_map.get(sid) {
                        if let Some(node) = nodes.iter_mut().find(|n| n.id == nid) {
                            node.kind = kind;
                        }
                    }
                }
            } else if let Some(val) = meta.strip_prefix("layout:") {
                let row: Vec<String> = val.split_whitespace().map(|s| s.to_string()).collect();
                if !row.is_empty() {
                    layout_hints.push(row);
                }
            }
            continue;
        }

        // ── Graph declaration ─────────────────────────────────────────────
        if line.starts_with("graph ") || line.starts_with("flowchart ") {
            continue;
        }

        // ── Edge line detection ───────────────────────────────────────────
        // Edges contain --> or -.-> or ==>
        if line.contains("-->") || line.contains("-.-") || line.contains("==>") {
            if let Some((from_sid, to_sid, label, style)) = parse_edge_line(line, lineno)? {
                let from_id = get_or_insert_node(&mut nodes, &mut id_map, &from_sid, None);
                let to_id = get_or_insert_node(&mut nodes, &mut id_map, &to_sid, None);
                edges.push(OwnedEdge {
                    from: from_id,
                    to: to_id,
                    label,
                    style,
                });
            }
            continue;
        }

        // ── Node declaration ──────────────────────────────────────────────
        if let Some((sid, label, kind)) = parse_node_line(line) {
            let nid = get_or_insert_node(&mut nodes, &mut id_map, &sid, Some((label, kind)));
            // Overwrite label/kind if node was auto-created by an earlier edge
            if let Some(node) = nodes.iter_mut().find(|n| n.id == nid) {
                // Only update if this is an explicit declaration (has a shape specifier)
                // parse_node_line only returns Some for lines with shape syntax
                if !node.label.is_empty() {
                    // already set — re-apply in case edge auto-created it blank
                }
            }
            last_node_str_id = Some(sid);
            continue;
        }
    }

    // ── Layout resolution ─────────────────────────────────────────────────
    let layout = if !layout_hints.is_empty() {
        layout_hints
            .into_iter()
            .map(|row| {
                row.into_iter()
                    .filter_map(|sid| id_map.get(&sid).copied())
                    .collect::<Vec<_>>()
            })
            .filter(|row: &Vec<usize>| !row.is_empty())
            .collect()
    } else {
        bfs_layout(&nodes, &edges)
    };

    if nodes.is_empty() {
        return Err(MermaidError {
            line: 0,
            message: "no nodes found".to_string(),
        });
    }

    Ok(OwnedDiagram { name, nodes, edges, layout })
}

/// Get numeric ID for a string node ID, inserting a blank node if not seen before.
fn get_or_insert_node(
    nodes: &mut Vec<OwnedNode>,
    id_map: &mut std::collections::HashMap<String, usize>,
    sid: &str,
    label_kind: Option<(String, NodeKind)>,
) -> usize {
    if let Some(&existing) = id_map.get(sid) {
        // If we have label info, update the node (edge auto-creates blank nodes)
        if let Some((label, kind)) = label_kind {
            if let Some(node) = nodes.iter_mut().find(|n| n.id == existing) {
                if node.label.is_empty() {
                    node.label = label;
                    node.kind = kind;
                }
            }
        }
        return existing;
    }
    let nid = nodes.len();
    let (label, kind) = label_kind.unwrap_or_else(|| (sid.to_string(), NodeKind::Command));
    nodes.push(OwnedNode {
        id: nid,
        label,
        detail: String::new(),
        kind,
    });
    id_map.insert(sid.to_string(), nid);
    nid
}

/// Parse a node declaration line. Returns (string_id, label, kind) or None.
///
/// Supported shapes:
///   A[label]     → Command
///   A([label])   → Branch
///   A{label}     → Job
///   A>label<     → Artifact
fn parse_node_line(line: &str) -> Option<(String, String, NodeKind)> {
    // Find the first shape-opening character
    let bracket_pos = line.find(|c| matches!(c, '[' | '{' | '>' | '('));
    let bracket_pos = bracket_pos?;

    let sid = line[..bracket_pos].trim().to_string();
    if sid.is_empty() || sid.contains(' ') {
        return None;
    }

    let rest = &line[bracket_pos..];

    if let Some(inner) = rest.strip_prefix("([").and_then(|s| s.strip_suffix("])")) {
        return Some((sid, inner.to_string(), NodeKind::Branch));
    }
    if let Some(inner) = rest.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        return Some((sid, inner.to_string(), NodeKind::Command));
    }
    if let Some(inner) = rest.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
        return Some((sid, inner.to_string(), NodeKind::Job));
    }
    if let Some(inner) = rest.strip_prefix('>').and_then(|s| s.strip_suffix('<')) {
        return Some((sid, inner.to_string(), NodeKind::Artifact));
    }

    None
}

/// Parse an edge line. Returns (from_sid, to_sid, label, style) or None if unparseable.
fn parse_edge_line(
    line: &str,
    lineno: usize,
) -> Result<Option<(String, String, Option<String>, EdgeStyle)>, MermaidError> {
    // Detect style
    let style = if line.contains("==>") {
        EdgeStyle::Manual
    } else if line.contains("-.-") {
        EdgeStyle::Dashed
    } else {
        EdgeStyle::Solid
    };

    let arrow = match style {
        EdgeStyle::Manual => "==>",
        EdgeStyle::Dashed => "-.->",
        EdgeStyle::Solid => "-->",
    };

    let arrow_pos = match line.find(arrow) {
        Some(p) => p,
        None => {
            return Err(MermaidError {
                line: lineno,
                message: format!("could not find arrow in edge line: {line}"),
            });
        }
    };

    let from_sid = line[..arrow_pos].trim().to_string();
    if from_sid.is_empty() {
        return Ok(None);
    }

    let after_arrow = &line[arrow_pos + arrow.len()..];

    // Optional label: |text|
    let (label, to_part) = if after_arrow.trim_start().starts_with('|') {
        let rest = after_arrow.trim_start().strip_prefix('|').unwrap();
        if let Some(pipe_end) = rest.find('|') {
            let lbl = rest[..pipe_end].to_string();
            let to = rest[pipe_end + 1..].trim().to_string();
            (Some(lbl), to)
        } else {
            (None, after_arrow.trim().to_string())
        }
    } else {
        (None, after_arrow.trim().to_string())
    };

    let to_sid = to_part.trim().to_string();
    if to_sid.is_empty() {
        return Ok(None);
    }

    Ok(Some((from_sid, to_sid, label, style)))
}

fn parse_kind(s: &str) -> NodeKind {
    match s.to_lowercase().as_str() {
        "branch" => NodeKind::Branch,
        "job" => NodeKind::Job,
        "artifact" => NodeKind::Artifact,
        _ => NodeKind::Command,
    }
}

/// Assign layout rows by BFS from root nodes (nodes with no incoming edges).
fn bfs_layout(nodes: &[OwnedNode], edges: &[OwnedEdge]) -> Vec<Vec<usize>> {
    use std::collections::{HashMap, VecDeque};

    let all_ids: Vec<usize> = nodes.iter().map(|n| n.id).collect();
    let has_incoming: std::collections::HashSet<usize> =
        edges.iter().map(|e| e.to).collect();

    let roots: Vec<usize> = all_ids
        .iter()
        .copied()
        .filter(|id| !has_incoming.contains(id))
        .collect();

    let mut depth: HashMap<usize, usize> = HashMap::new();
    let mut queue: VecDeque<(usize, usize)> = roots.iter().map(|&id| (id, 0)).collect();

    while let Some((id, d)) = queue.pop_front() {
        depth.entry(id).or_insert(d);
        for e in edges.iter().filter(|e| e.from == id) {
            if !depth.contains_key(&e.to) {
                queue.push_back((e.to, d + 1));
            }
        }
    }

    // Nodes not reachable from roots get depth 0
    for id in &all_ids {
        depth.entry(*id).or_insert(0);
    }

    let max_depth = depth.values().copied().max().unwrap_or(0);
    let mut rows: Vec<Vec<usize>> = vec![Vec::new(); max_depth + 1];
    for id in &all_ids {
        rows[depth[id]].push(*id);
    }
    rows.retain(|r| !r.is_empty());
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIMPLE: &str = r#"
graph LR
  A([main])
  %% detail: Active R&D branch
  B([next])
  %% detail: Auto-promoted from main

  A -->|auto| B
%% name: Test Flow
%% layout: A B
"#;

    #[test]
    fn test_parse_node_count() {
        let d = parse(SIMPLE);
        assert_eq!(d.nodes.len(), 2, "expected 2 nodes");
    }

    #[test]
    fn test_parse_node_labels() {
        let d = parse(SIMPLE);
        assert_eq!(d.nodes[0].label, "main");
        assert_eq!(d.nodes[1].label, "next");
    }

    #[test]
    fn test_parse_node_kind_branch() {
        let d = parse(SIMPLE);
        assert_eq!(d.nodes[0].kind, NodeKind::Branch);
        assert_eq!(d.nodes[1].kind, NodeKind::Branch);
    }

    #[test]
    fn test_parse_detail() {
        let d = parse(SIMPLE);
        assert_eq!(d.nodes[0].detail, "Active R&D branch");
        assert_eq!(d.nodes[1].detail, "Auto-promoted from main");
    }

    #[test]
    fn test_parse_edge() {
        let d = parse(SIMPLE);
        assert_eq!(d.edges.len(), 1);
        assert_eq!(d.edges[0].from, 0);
        assert_eq!(d.edges[0].to, 1);
        assert_eq!(d.edges[0].label.as_deref(), Some("auto"));
        assert_eq!(d.edges[0].style, EdgeStyle::Solid);
    }

    #[test]
    fn test_parse_name() {
        let d = parse(SIMPLE);
        assert_eq!(d.name, "Test Flow");
    }

    #[test]
    fn test_parse_layout_hints() {
        let d = parse(SIMPLE);
        assert_eq!(d.layout, vec![vec![0usize, 1usize]]);
    }

    #[test]
    fn test_parse_edge_styles() {
        let src = r#"
graph LR
  A[cmd]
  B[cmd2]
  C[cmd3]
  A -.-> B
  A ==> C
"#;
        let d = parse(src);
        assert_eq!(d.edges[0].style, EdgeStyle::Dashed);
        assert_eq!(d.edges[1].style, EdgeStyle::Manual);
    }

    #[test]
    fn test_parse_node_shapes() {
        let src = r#"
flowchart LR
  A[box]
  B([rounded])
  C{diamond}
  D>flag<
"#;
        let d = parse(src);
        assert_eq!(d.nodes[0].kind, NodeKind::Command);
        assert_eq!(d.nodes[1].kind, NodeKind::Branch);
        assert_eq!(d.nodes[2].kind, NodeKind::Job);
        assert_eq!(d.nodes[3].kind, NodeKind::Artifact);
    }

    #[test]
    fn test_bfs_layout_fallback() {
        let src = r#"
graph LR
  A[start]
  B[middle]
  C[end]
  A --> B
  B --> C
"#;
        let d = parse(src);
        // BFS: A at depth 0, B at depth 1, C at depth 2
        assert_eq!(d.layout.len(), 3);
        assert_eq!(d.layout[0], vec![0]);
        assert_eq!(d.layout[1], vec![1]);
        assert_eq!(d.layout[2], vec![2]);
    }

    #[test]
    fn test_kind_override() {
        let src = r#"
graph LR
  A[mynode]
  %% kind: artifact
"#;
        let d = parse(src);
        assert_eq!(d.nodes[0].kind, NodeKind::Artifact);
    }

    #[test]
    fn test_empty_source_is_error_diagram() {
        let d = parse("");
        assert_eq!(d.name, "Parse Error");
        assert_eq!(d.nodes.len(), 1);
        assert!(d.nodes[0].detail.contains("no nodes found"));
    }

    #[test]
    fn test_navigation_works_on_parsed_diagram() {
        let d = parse(SIMPLE);
        // navigate right from node 0 should reach node 1
        let next = d.navigate(0, crate::diagram::NavDir::Right);
        assert_eq!(next, Some(1));
    }
}
```

- [ ] **Step 2: Run the tests (expect failures since parser is implemented above — they should pass)**

```bash
cargo test -p dashbox diagram::mermaid 2>&1
```

Expected: all tests pass (implementation and tests are written together in this step).

- [ ] **Step 3: Commit**

```bash
git add crates/dashbox/src/diagram/mermaid.rs
git commit -m "feat(dashbox): add Mermaid parser producing OwnedDiagram"
```

---

## Task 3: DiagramSource and built_in_diagrams loader

**Files:**
- Modify: `crates/dashbox/src/diagram/source.rs`
- Create: `crates/dashbox/src/diagrams/` directory (6 `.mmd` files)
- Replace: `crates/dashbox/src/diagrams.rs`

- [ ] **Step 1: Create the 6 `.mmd` files**

Note: `include_str!` paths are relative to the source file. Since `source.rs` is at
`crates/dashbox/src/diagram/source.rs`, the path to `crates/dashbox/src/diagrams/ci_flow.mmd`
is `"../diagrams/ci_flow.mmd"`.

Create `crates/dashbox/src/diagrams/ci_flow.mmd`:
```
graph LR
  feature([feature/*])
  %% detail: Short-lived branches targeting main. PRs trigger fmt+clippy+check gates. Auto-deleted on merge.
  main([main])
  %% detail: Active R&D. CI gates: cargo check + fmt --check + clippy -D warnings. Direct push or PR merge.
  next([next])
  %% detail: Auto-promoted from main on green CI via phased-deployment.yml. Extra gates: nextest + cargo audit + deny + machete.
  stable([stable])
  %% detail: Manual promotion from next via workflow_dispatch. Extra gates: cargo geiger + release build.
  vtag>v* tag<
  %% detail: Versioned release cut from stable. Triggers release.yml: cross-compiled musl binaries + GitHub Release.

  feature -->|PR| main
  main -->|auto| next
  next ==>|manual| stable
  stable -.->|tag| vtag

%% name: CI Flow
%% layout: feature main next stable vtag
```

Create `crates/dashbox/src/diagrams/dev_loop.mmd`:
```
graph LR
  edit[edit]
  %% detail: Write Rust. Use bacon for fast watch-mode check. Shared CARGO_TARGET_DIR at ~/.mbx/cache/target/ across worktrees.
  chk[cargo chk]
  %% detail: Fast type-check pass, no codegen. Catches most errors in <1s. Run via bacon or directly.
  precommit[pre-commit]
  %% detail: cargo xtask pre-commit: fmt --check + clippy -D warnings + release build. macOS-safe (no Linux-only crates).
  commit[commit]
  %% detail: git commit. SSH-signed via 1Password agent. AI-generated message: just commit-msg. Hooks run obfsck secrets audit.
  push[git push]
  %% detail: just sync-check fetches+rebases onto origin/main first. Then pushes to remote, triggering GitHub Actions.
  ci{CI (GHA)}
  %% detail: ci.yml: check+fmt+clippy on all branches. nextest+audit+deny+machete on next+stable. geiger on stable only.
  next([next])
  %% detail: phased-deployment.yml auto-promotes main→next after green CI. Triggers the full nextest + audit gate suite.

  edit --> chk
  chk --> precommit
  precommit --> commit
  commit --> push
  push -->|triggers| ci
  ci -.->|promotes| next

%% name: Dev Loop
%% layout: edit chk precommit commit
%% layout: push ci next
```

Create `crates/dashbox/src/diagrams/container_lifecycle.mmd`:
```
graph LR
  runreq[run req]
  %% detail: CLI sends RunContainer JSON over Unix socket at /run/minibox/miniboxd.sock. Protocol: JSON-over-newline.
  auth{auth}
  %% detail: SO_PEERCRED on Unix socket. Kernel provides client UID/PID. Only UID 0 (root) permitted. Logged for audit trail.
  imgcache{img cache}
  %% detail: Check /var/lib/minibox/images/ for cached layers. If missing, pulls from Docker Hub with anonymous token auth.
  overlay[overlay]
  %% detail: mount overlay: lowerdir=layers (read-only), upperdir=container_rw, workdir=container_work. Requires CLONE_NEWNS + root.
  clone[clone()]
  %% detail: clone(2) with CLONE_NEWPID | CLONE_NEWNS | CLONE_NEWUTS | CLONE_NEWIPC | CLONE_NEWNET. Parent spawns reaper task for child PID.
  pivotroot[pivot_root]
  %% detail: Child: MS_PRIVATE propagation, bind-mount rootfs, pivot_root to container FS, unmount old root.
  exec[exec]
  %% detail: execve() with explicit envp (not execvp). Closes extra FDs via close_range(). PID 1 inside container namespace.

  runreq --> auth
  auth --> imgcache
  imgcache --> overlay
  overlay --> clone
  clone --> pivotroot
  pivotroot --> exec

%% name: Container Lifecycle
%% layout: runreq auth imgcache overlay
%% layout: clone pivotroot exec
```

Create `crates/dashbox/src/diagrams/image_pull.mmd`:
```
graph LR
  imageref[ImageRef]
  %% detail: Parse [REGISTRY/]NAMESPACE/NAME[:TAG]. Routes to correct registry adapter. Default: docker.io/library.
  tokenauth{token auth}
  %% detail: Docker Hub: POST /token with scope repository:pull. Returns short-lived JWT. Anonymous auth, no login required.
  manifest{manifest}
  %% detail: GET /v2/{name}/manifests/{ref}. Max size: 10MB. Parses OCI image manifest JSON for layer digest list.
  layers{layers}
  %% detail: GET /v2/{name}/blobs/{digest} per layer. Max: 1GB/layer, 5GB total. Streamed to disk.
  verify{verify}
  %% detail: SHA256 digest of downloaded blob compared against manifest entry. Reject on mismatch.
  untar[untar]
  %% detail: Extract tar layer. Security checks: reject path traversal (..), absolute symlinks, device nodes. Strip setuid bits.
  cached>cached<
  %% detail: Layers written to /var/lib/minibox/images/{name}/{digest}/. Ready for overlay mount.

  imageref --> tokenauth
  tokenauth --> manifest
  manifest --> layers
  layers --> verify
  verify --> untar
  untar --> cached

%% name: Image Pull
%% layout: imageref tokenauth manifest layers
%% layout: verify untar cached
```

Create `crates/dashbox/src/diagrams/adapter_suite.mmd`:
```
graph LR
  adapter[ADAPTER]
  %% detail: MINIBOX_ADAPTER env var selects the adapter suite at daemon startup. Wired in miniboxd/src/main.rs.
  native{native}
  %% detail: Linux namespaces + cgroups v2 + overlay FS. Requires root. Default adapter. Full isolation.
  gke{gke}
  %% detail: Unprivileged: proot + copy FS + no-op limiter. No root required. For GKE/restricted environments.
  colima{colima}
  %% detail: macOS via limactl + nerdctl inside Colima VM. Routed through ColimaRuntime adapter. Requires Colima running.
  vz{vz}
  %% detail: macOS Virtualization.framework: boots Alpine Linux VM, forwards commands via vsock. Requires --features vz + VM image.

  adapter -->|native| native
  adapter -->|gke| gke
  adapter -->|colima| colima
  adapter -.->|vz| vz

%% name: Adapter Suite
%% layout: adapter
%% layout: native gke colima vz
```

Create `crates/dashbox/src/diagrams/workspace_deps.mmd`:
```
graph LR
  miniboxd>miniboxd<
  %% detail: Unified daemon binary. Unconditional: daemonbox. Linux: mbx+minibox-core+nix. macOS: macbox. Windows: winbox.
  miniboxcli>minibox-cli<
  %% detail: Platform-agnostic CLI binary. Deps: minibox-core + minibox-client. No direct mbx dependency.
  mbxbench>mbx-bench<
  %% detail: Benchmark harness binary. Deps: minibox-core + mbx.
  dockerboxd>dockerboxd<
  %% detail: Docker API shim (axum HTTP over Unix socket). Translates Docker API to minibox protocol. Deps: minibox-client + minibox-core.
  macbox{macbox}
  %% detail: macOS daemon: Colima + vz adapters. Deps: daemonbox + minibox-core + mbx. Optional: objc2 + Virtualization.framework.
  winbox{winbox}
  %% detail: Windows daemon stub (HCS planned). Deps: daemonbox + mbx.
  daemonbox{daemonbox}
  %% detail: Platform-agnostic handler/state/server. Deps: minibox-core + mbx. Unix: nix for SO_PEERCRED auth.
  mbxclient{mbx-client}
  %% detail: Unix socket client: DaemonClient + DaemonResponseStream. Deps: minibox-core only.
  mbx{mbx}
  %% detail: Linux container primitives: namespaces, cgroups v2, overlay FS, image pull, adapters. Deps: minibox-core + minibox-macros + nix.
  mbxcore{mbx-core}
  %% detail: Cross-platform types: protocol, domain traits, image management, preflight. Deps: minibox-macros. Unix: nix.
  mbxmacros[mbx-macros]
  %% detail: Proc-macro crate: as_any! and adapt! macros used by mbx and minibox-core. True leaf — no workspace deps.

  miniboxd --> daemonbox
  miniboxd -->|linux| mbxcore
  miniboxd -->|linux| mbx
  miniboxd -.->|macos| macbox
  miniboxd -.->|windows| winbox
  miniboxcli --> mbxcore
  miniboxcli --> mbxclient
  mbxbench --> mbxcore
  mbxbench --> mbx
  dockerboxd --> mbxclient
  dockerboxd --> mbxcore
  macbox --> daemonbox
  macbox --> mbxcore
  macbox --> mbx
  winbox --> daemonbox
  winbox --> mbx
  daemonbox --> mbxcore
  daemonbox --> mbx
  mbxclient --> mbxcore
  mbx --> mbxcore
  mbx --> mbxmacros
  mbxcore --> mbxmacros

%% name: Workspace Deps
%% layout: miniboxd miniboxcli mbxbench dockerboxd
%% layout: macbox winbox daemonbox mbxclient
%% layout: mbx mbxcore
%% layout: mbxmacros
```

- [ ] **Step 2: Implement `source.rs`**

Replace `crates/dashbox/src/diagram/source.rs` with:

```rust
// dashbox/src/diagram/source.rs

use std::path::PathBuf;

use crate::diagram::OwnedDiagram;
use crate::diagram::mermaid;

pub enum DiagramSource {
    /// Built-in diagram embedded as a `&'static str` via `include_str!`.
    Embedded { name: &'static str, src: &'static str },
    /// User-defined diagram loaded from a `.mmd` file on disk.
    File(PathBuf),
}

impl DiagramSource {
    /// Parse/load into an OwnedDiagram. Infallible — errors produce an error-node diagram.
    pub fn load(&self) -> OwnedDiagram {
        match self {
            DiagramSource::Embedded { src, .. } => mermaid::parse(src),
            DiagramSource::File(path) => match std::fs::read_to_string(path) {
                Ok(src) => mermaid::parse(&src),
                Err(e) => mermaid::parse(&format!("%%error loading file: {e}")),
            },
        }
    }
}

/// All built-in diagrams as embedded Mermaid sources.
pub fn built_in_diagrams() -> Vec<OwnedDiagram> {
    let sources: &[(&str, &str)] = &[
        ("CI Flow",             include_str!("../diagrams/ci_flow.mmd")),
        ("Dev Loop",            include_str!("../diagrams/dev_loop.mmd")),
        ("Container Lifecycle", include_str!("../diagrams/container_lifecycle.mmd")),
        ("Image Pull",          include_str!("../diagrams/image_pull.mmd")),
        ("Adapter Suite",       include_str!("../diagrams/adapter_suite.mmd")),
        ("Workspace Deps",      include_str!("../diagrams/workspace_deps.mmd")),
    ];
    sources
        .iter()
        .map(|(_name, src)| DiagramSource::Embedded { name: _name, src }.load())
        .collect()
}

/// Load all `.mmd` files from `~/.mbx/diagrams/`, sorted by filename.
/// Files that fail to parse produce single error-node diagrams (never panics).
pub fn load_user_diagrams() -> Vec<OwnedDiagram> {
    let dir = match dirs::home_dir() {
        Some(h) => h.join(".mbx").join("diagrams"),
        None => return Vec::new(),
    };
    if !dir.exists() {
        return Vec::new();
    }
    let mut paths: Vec<PathBuf> = match std::fs::read_dir(&dir) {
        Ok(entries) => entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().map_or(false, |ext| ext == "mmd"))
            .collect(),
        Err(_) => return Vec::new(),
    };
    paths.sort();
    paths
        .into_iter()
        .map(|p| DiagramSource::File(p).load())
        .collect()
}
```

- [ ] **Step 3: Replace `diagrams.rs` with a thin wrapper**

Replace the entire content of `crates/dashbox/src/diagrams.rs` with:

```rust
// dashbox/src/diagrams.rs — re-exports source loader for built-in diagrams
pub use crate::diagram::source::built_in_diagrams;
```

- [ ] **Step 4: Verify it compiles**

```bash
cargo check -p dashbox 2>&1 | head -40
```

Expected: errors only in `tabs/diagrams.rs` (old `Diagram`/`View` references) — not in the new files.

- [ ] **Step 5: Commit**

```bash
git add crates/dashbox/src/diagram/source.rs crates/dashbox/src/diagrams/ crates/dashbox/src/diagrams.rs
git commit -m "feat(dashbox): add DiagramSource, built-in .mmd files, user diagram loader"
```

---

## Task 4: Migrate DiagramsTab to Vec<OwnedDiagram>

**Files:**
- Modify: `crates/dashbox/src/tabs/diagrams.rs`

This is the largest migration task. We replace the 6 named `Diagram` fields and the `View` enum
with `Vec<OwnedDiagram>` and an `active: usize` index.

- [ ] **Step 1: Replace the entire `tabs/diagrams.rs`**

Replace the full content of `crates/dashbox/src/tabs/diagrams.rs` with:

```rust
// dashbox/src/tabs/diagrams.rs

use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use super::{TabAction, TabRenderer};
use crate::data::CachedSource;
use crate::data::ci::CiSource;
use crate::diagram::{EdgeStyle, NavDir, NodeKind, NodeStatus, OwnedDiagram};

// ── Layout constants ─────────────────────────────────────────────────────────

/// Inner label width for each node box.
const NODE_INNER: usize = 10;
/// Total box width: ┌ + (NODE_INNER+2)×─ + ┐
#[allow(dead_code)]
const BOX_W: usize = NODE_INNER + 4;
/// Width of the connector string between two nodes in the same row.
const CONN_W: usize = 9;

// ── Tab state ────────────────────────────────────────────────────────────────

pub struct DiagramsTab {
    diagrams: Vec<OwnedDiagram>,
    active: usize,
    selected: usize,
    ci_source: CachedSource<CiSource>,
}

impl DiagramsTab {
    pub fn new(extra: Vec<OwnedDiagram>) -> Self {
        let mut diagrams = crate::diagrams::built_in_diagrams();
        diagrams.extend(extra);
        let first = diagrams.first().and_then(|d| d.first_node()).unwrap_or(0);
        Self {
            diagrams,
            active: 0,
            selected: first,
            ci_source: CachedSource::new(CiSource::new(), 30),
        }
    }

    fn active_diagram(&self) -> &OwnedDiagram {
        &self.diagrams[self.active]
    }

    fn go_to_index(&mut self, idx: usize) {
        self.active = idx;
        self.selected = self.active_diagram().first_node().unwrap_or(0);
    }

    fn next_diagram(&mut self) {
        let next = (self.active + 1) % self.diagrams.len();
        self.go_to_index(next);
    }

    fn prev_diagram(&mut self) {
        let n = self.diagrams.len();
        let prev = (self.active + n - 1) % n;
        self.go_to_index(prev);
    }

    /// Live CI status overlay — only populated for the first diagram (CI Flow).
    fn build_statuses(&mut self) -> HashMap<usize, NodeStatus> {
        // CI status only applies to the CI Flow diagram (index 0, nodes main=1,next=2,stable=3)
        if self.active != 0 {
            return HashMap::new();
        }
        self.ci_source.ensure_fresh();
        let ci_data = match self.ci_source.get() {
            Some(Ok(d)) => d,
            _ => return HashMap::new(),
        };
        let mut branch_status: HashMap<&str, NodeStatus> = HashMap::new();
        for run in &ci_data.runs {
            let status = if run.status == "completed" {
                match run.conclusion.as_str() {
                    "success" => NodeStatus::Passing,
                    "failure" => NodeStatus::Failing,
                    _ => NodeStatus::Unknown,
                }
            } else if run.status == "in_progress" {
                NodeStatus::Running
            } else {
                NodeStatus::Unknown
            };
            branch_status.entry(run.head_branch.as_str()).or_insert(status);
        }
        // In ci_flow.mmd: main=1, next=2, stable=3 (declaration order)
        [("main", 1usize), ("next", 2), ("stable", 3)]
            .iter()
            .filter_map(|&(branch, id)| branch_status.get(branch).map(|&s| (id, s)))
            .collect()
    }
}

// ── Render helpers ────────────────────────────────────────────────────────────

fn kind_color(kind: NodeKind) -> Color {
    match kind {
        NodeKind::Branch => Color::Cyan,
        NodeKind::Job => Color::Yellow,
        NodeKind::Command => Color::Green,
        NodeKind::Artifact => Color::Magenta,
    }
}

fn status_color(status: NodeStatus, kind: NodeKind) -> Color {
    match status {
        NodeStatus::Unknown => kind_color(kind),
        NodeStatus::Passing => Color::Green,
        NodeStatus::Failing => Color::Red,
        NodeStatus::Running => Color::Yellow,
    }
}

/// Center `text` within `width` display columns, truncating if too long.
fn center_label(text: &str, width: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len().min(width);
    let label: String = chars[..len].iter().collect();
    let pad = width - len;
    let left = pad / 2;
    let right = pad - left;
    format!("{}{}{}", " ".repeat(left), label, " ".repeat(right))
}

/// Build a connector string of exactly `width` display columns.
fn connector_str(edge: &crate::diagram::OwnedEdge, width: usize) -> String {
    let fill = match edge.style {
        EdgeStyle::Solid => '─',
        EdgeStyle::Dashed => '-',
        EdgeStyle::Manual => '·',
    };
    let lbl = edge.label.as_deref().unwrap_or("");
    let lbl_len = lbl.chars().count();
    let fill_total = width.saturating_sub(1 + lbl_len);
    let left = fill_total / 2;
    let right = fill_total - left;
    let mut s = String::new();
    for _ in 0..left { s.push(fill); }
    s.push_str(lbl);
    for _ in 0..right { s.push(fill); }
    s.push('►');
    s
}

fn connector_color(style: EdgeStyle) -> Color {
    match style {
        EdgeStyle::Solid => Color::DarkGray,
        EdgeStyle::Dashed => Color::DarkGray,
        EdgeStyle::Manual => Color::Yellow,
    }
}

/// Render one row of nodes as three `Line`s: top borders, labels, bottom borders.
fn render_row_lines(
    row: &[usize],
    diagram: &OwnedDiagram,
    selected: usize,
    statuses: &HashMap<usize, NodeStatus>,
    is_continuation: bool,
) -> [Line<'static>; 3] {
    let mut tops: Vec<Span<'static>> = Vec::new();
    let mut mids: Vec<Span<'static>> = Vec::new();
    let mut bots: Vec<Span<'static>> = Vec::new();

    tops.push(Span::raw("  "));
    mids.push(if is_continuation {
        Span::styled("↳ ", Style::default().fg(Color::DarkGray))
    } else {
        Span::raw("  ")
    });
    bots.push(Span::raw("  "));

    let gap: String = " ".repeat(CONN_W);

    for (i, &nid) in row.iter().enumerate() {
        if i > 0 {
            let prev = row[i - 1];
            if let Some(edge) = diagram.edge_between(prev, nid) {
                let conn = connector_str(edge, CONN_W);
                let color = connector_color(edge.style);
                tops.push(Span::raw(gap.clone()));
                mids.push(Span::styled(conn, Style::default().fg(color)));
                bots.push(Span::raw(gap.clone()));
            } else {
                tops.push(Span::raw(gap.clone()));
                mids.push(Span::raw(gap.clone()));
                bots.push(Span::raw(gap.clone()));
            }
        }

        let node = match diagram.node(nid) {
            Some(n) => n,
            None => continue,
        };
        let status = statuses.get(&nid).copied().unwrap_or_default();
        let color = status_color(status, node.kind);

        let style = if nid == selected {
            Style::default()
                .fg(Color::Black)
                .bg(color)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(color)
        };

        let label = center_label(&node.label, NODE_INNER);
        let top = format!("┌{:─<w$}┐", "", w = NODE_INNER + 2);
        let mid = format!("│ {label} │");
        let bot = format!("└{:─<w$}┘", "", w = NODE_INNER + 2);

        tops.push(Span::styled(top, style));
        mids.push(Span::styled(mid, style));
        bots.push(Span::styled(bot, style));
    }

    [Line::from(tops), Line::from(mids), Line::from(bots)]
}

fn diagram_lines(
    diagram: &OwnedDiagram,
    selected: usize,
    statuses: &HashMap<usize, NodeStatus>,
) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();

    for (row_idx, row) in diagram.layout.iter().enumerate() {
        if row_idx > 0 {
            let prev = &diagram.layout[row_idx - 1];
            let is_cont = diagram.has_cross_row_edge(prev, row);
            lines.push(Line::raw(""));
            let [t, m, b] = render_row_lines(row, diagram, selected, statuses, is_cont);
            lines.extend([t, m, b]);
        } else {
            let [t, m, b] = render_row_lines(row, diagram, selected, statuses, false);
            lines.extend([t, m, b]);
        }
    }

    lines
}

fn detail_lines(
    diagram: &OwnedDiagram,
    selected: usize,
    statuses: &HashMap<usize, NodeStatus>,
) -> Vec<Line<'static>> {
    let node = match diagram.node(selected) {
        Some(n) => n,
        None => return vec![],
    };
    let status = statuses.get(&selected).copied().unwrap_or_default();

    let (status_str, status_color) = match status {
        NodeStatus::Unknown => ("", Color::DarkGray),
        NodeStatus::Passing => (" ✓ passing", Color::Green),
        NodeStatus::Failing => (" ✗ failing", Color::Red),
        NodeStatus::Running => (" ⏳ running", Color::Yellow),
    };
    let kind_col = kind_color(node.kind);

    let mut lines: Vec<Line<'static>> = Vec::new();

    lines.push(Line::from(Span::styled(
        node.label.clone(),
        Style::default().fg(kind_col).add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(vec![
        Span::styled(node.kind.label().to_string(), Style::default().fg(Color::DarkGray)),
        Span::styled(status_str.to_string(), Style::default().fg(status_color)),
    ]));
    lines.push(Line::raw(""));
    lines.push(Line::from(Span::raw(node.detail.clone())));
    lines.push(Line::raw(""));

    let outgoing = diagram.outgoing(selected);
    if !outgoing.is_empty() {
        lines.push(Line::from(Span::styled(
            "edges".to_string(),
            Style::default().fg(Color::DarkGray),
        )));
        for edge in outgoing {
            let target = diagram.node(edge.to).map(|n| n.label.as_str()).unwrap_or("?");
            let arrow = match edge.style {
                EdgeStyle::Solid => "──►",
                EdgeStyle::Dashed => "─ ►",
                EdgeStyle::Manual => "··►",
            };
            let text = match &edge.label {
                Some(lbl) => format!(" {arrow} {target}  ({lbl})"),
                None => format!(" {arrow} {target}"),
            };
            lines.push(Line::from(Span::styled(
                text,
                Style::default().fg(Color::Gray),
            )));
        }
    }

    lines
}

fn legend_line() -> Line<'static> {
    Line::from(vec![
        Span::styled("─────────► ", Style::default().fg(Color::DarkGray)),
        Span::styled("auto  ", Style::default().fg(Color::DarkGray)),
        Span::styled("─--------► ", Style::default().fg(Color::DarkGray)),
        Span::styled("release  ", Style::default().fg(Color::DarkGray)),
        Span::styled("·········► ", Style::default().fg(Color::Yellow)),
        Span::styled("manual  ", Style::default().fg(Color::DarkGray)),
        Span::styled("⎇ branch  ", Style::default().fg(Color::Cyan)),
        Span::styled("⚙ job  ", Style::default().fg(Color::Yellow)),
        Span::styled("$ cmd  ", Style::default().fg(Color::Green)),
        Span::styled("◈ artifact", Style::default().fg(Color::Magenta)),
    ])
}

// ── TabRenderer ──────────────────────────────────────────────────────────────

impl TabRenderer for DiagramsTab {
    fn render(&mut self, frame: &mut Frame, area: Rect) {
        let statuses = self.build_statuses();
        let diagram = self.active_diagram();
        let selected = self.selected;
        let active = self.active;
        let total = self.diagrams.len();

        let chunks = Layout::horizontal([Constraint::Percentage(70), Constraint::Percentage(30)])
            .split(area);

        // ── Left: diagram canvas ──────────────────────────────────────────
        let title = format!(" {} ({}/{})  d/D:cycle ", diagram.name, active + 1, total);
        let left_block = Block::default()
            .borders(Borders::ALL)
            .title(title)
            .title_style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD));
        let inner_left = left_block.inner(chunks[0]);
        frame.render_widget(left_block, chunks[0]);

        let mut all_lines: Vec<Line<'static>> = Vec::new();
        all_lines.push(Line::raw(""));
        all_lines.extend(diagram_lines(diagram, selected, &statuses));
        all_lines.push(Line::raw(""));
        all_lines.push(legend_line());

        frame.render_widget(Paragraph::new(all_lines), inner_left);

        // ── Right: node detail ────────────────────────────────────────────
        let right_block = Block::default()
            .borders(Borders::ALL)
            .title(" Node ")
            .title_style(Style::default().fg(Color::DarkGray));
        let inner_right = right_block.inner(chunks[1]);
        frame.render_widget(right_block, chunks[1]);

        frame.render_widget(
            Paragraph::new(detail_lines(diagram, selected, &statuses)).wrap(Wrap { trim: true }),
            inner_right,
        );
    }

    fn handle_key(&mut self, key: KeyEvent) -> TabAction {
        let nav_result = {
            let diagram = self.active_diagram();
            match key.code {
                KeyCode::Left | KeyCode::Char('h') => diagram.navigate(self.selected, NavDir::Left),
                KeyCode::Right | KeyCode::Char('l') => diagram.navigate(self.selected, NavDir::Right),
                KeyCode::Up | KeyCode::Char('k') => diagram.navigate(self.selected, NavDir::Up),
                KeyCode::Down | KeyCode::Char('j') => diagram.navigate(self.selected, NavDir::Down),
                KeyCode::Tab => Some(diagram.next_node(self.selected)),
                KeyCode::BackTab => Some(diagram.prev_node(self.selected)),
                _ => None,
            }
        };

        match key.code {
            KeyCode::Char('d') => self.next_diagram(),
            KeyCode::Char('D') => self.prev_diagram(),
            _ => {
                if let Some(nid) = nav_result {
                    self.selected = nid;
                }
            }
        }

        TabAction::None
    }

    fn refresh(&mut self) {
        self.ci_source.refresh();
    }

    fn status_keys(&self) -> &'static str {
        "h/l:move  j/k:row  Tab:next  d/D:diagram"
    }
}
```

- [ ] **Step 2: Update `app.rs` to pass user diagrams**

In `crates/dashbox/src/app.rs`, change the `DiagramsTab::new()` call:

Find:
```rust
Box::new(DiagramsTab::new()),
```

Replace with:
```rust
Box::new(DiagramsTab::new(crate::diagram::source::load_user_diagrams())),
```

Also add the import at the top of `app.rs` if not already present — the `use crate::tabs::diagrams::DiagramsTab;` line is already there; that's sufficient.

- [ ] **Step 3: Full build check**

```bash
cargo check -p dashbox 2>&1
```

Expected: clean (zero errors).

- [ ] **Step 4: Run all dashbox tests**

```bash
cargo test -p dashbox 2>&1
```

Expected: all tests pass, including the mermaid parser tests from Task 2.

- [ ] **Step 5: Clippy**

```bash
cargo clippy -p dashbox -- -D warnings 2>&1
```

Fix any warnings before proceeding.

- [ ] **Step 6: Commit**

```bash
git add crates/dashbox/src/tabs/diagrams.rs crates/dashbox/src/app.rs
git commit -m "feat(dashbox): migrate DiagramsTab to Vec<OwnedDiagram> from Mermaid sources"
```

---

## Task 5: Delete dead code and verify end-to-end

**Files:**
- The old static-str `Diagram`/`Node`/`Edge` types were in `diagram.rs` (already deleted in Task 1).
- The old `diagrams.rs` functions (`ci_flow()`, etc.) are replaced in Task 3.
- Any remaining dead imports.

- [ ] **Step 1: Confirm no references to old types**

```bash
cargo check -p dashbox 2>&1
```

Look for any remaining references to the old `Diagram`, `Node`, `Edge` (non-`Owned`) types.

- [ ] **Step 2: Run full test suite**

```bash
cargo test -p dashbox 2>&1
```

Expected output includes lines like:
```
test diagram::mermaid::tests::test_parse_node_count ... ok
test diagram::mermaid::tests::test_parse_detail ... ok
test diagram::mermaid::tests::test_navigation_works_on_parsed_diagram ... ok
```

- [ ] **Step 3: Build the binary**

```bash
cargo build -p dashbox 2>&1
```

Expected: builds cleanly.

- [ ] **Step 4: Smoke-test the binary (manual)**

```bash
./target/debug/dashbox
```

Press `7` to go to the Diagrams tab. Verify:
- Title bar shows `CI Flow (1/6)` (or similar count including user diagrams if any exist)
- Nodes render with boxes
- Arrow-key navigation moves the selection highlight
- Detail panel updates when moving to a different node
- `d` key cycles to next diagram, `D` goes back
- Navigation works on all 6 built-in diagrams

- [ ] **Step 5: Final commit**

```bash
git add -A
git commit -m "feat(dashbox): mermaid diagram engine complete — parser + .mmd sources + user file loader"
```

---

## Self-Review

### Spec coverage check

| Spec requirement | Covered by |
|---|---|
| Mermaid is source of truth | Task 3 (6 `.mmd` files) |
| `Diagram` model stays navigable | Task 1 (`OwnedDiagram` with identical nav methods) |
| Embedded statics (built-ins) | Task 3 (`include_str!` + `built_in_diagrams()`) |
| File loading (`~/.mbx/diagrams/`) | Task 3 (`load_user_diagrams()`) |
| Navigation preserved | Task 4 (same key handlers, `OwnedDiagram` nav methods) |
| No new runtime deps | All tasks (hand-rolled parser) |
| `%% detail:` comment convention | Task 2 (parser) |
| `%% layout:` hints + BFS fallback | Task 2 (parser) |
| `%% kind:` override | Task 2 (parser) |
| `%% name:` override | Task 2 (parser) |
| Node shapes → `NodeKind` mapping | Task 2 (parser) |
| Edge styles (`-->`, `-.->`, `==>`) | Task 2 (parser) |
| Parse errors → error-node diagram | Task 2 (`error_diagram()`) |
| `View` enum removed | Task 4 |
| Unit tests for parser | Task 2 (12 tests) |
| File loader test | Not explicitly — covered by `load_user_diagrams()` returning empty vec gracefully |

### Placeholder scan
None found — all steps contain actual code.

### Type consistency
- `OwnedDiagram`, `OwnedNode`, `OwnedEdge` — defined in Task 1, used consistently in Tasks 2–4.
- `mermaid::parse()` returns `OwnedDiagram` — used in `DiagramSource::load()` in Task 3.
- `DiagramsTab::new(extra: Vec<OwnedDiagram>)` — defined in Task 4, called in Task 4 (`app.rs`).
- CI status node IDs (`main=1, next=2, stable=3`) — hardcoded in `build_statuses()`. These match declaration order in `ci_flow.mmd` (feature=0, main=1, next=2, stable=3, vtag=4). ✓
