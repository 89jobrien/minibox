// dashbox/src/diagram/mermaid.rs

use crate::diagram::{EdgeStyle, NodeKind, OwnedDiagram, OwnedEdge, OwnedNode};

#[derive(Debug)]
pub struct MermaidError {
    pub line: usize,
    pub message: String,
}

impl std::fmt::Display for MermaidError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "mermaid parse error at line {}: {}",
            self.line, self.message
        )
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
            get_or_insert_node(&mut nodes, &mut id_map, &sid, Some((label.clone(), kind)));
            // Update label/kind on the node (may have been auto-created by an edge)
            if let Some(&nid) = id_map.get(&sid) {
                if let Some(node) = nodes.iter_mut().find(|n| n.id == nid) {
                    node.label = label;
                    node.kind = kind;
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

    Ok(OwnedDiagram {
        name,
        nodes,
        edges,
        layout,
    })
}

/// Get numeric ID for a string node ID, inserting a blank node if not seen before.
fn get_or_insert_node(
    nodes: &mut Vec<OwnedNode>,
    id_map: &mut std::collections::HashMap<String, usize>,
    sid: &str,
    label_kind: Option<(String, NodeKind)>,
) -> usize {
    if let Some(&existing) = id_map.get(sid) {
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
    let bracket_pos = line.find(|c: char| "[{>(".contains(c))?;

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

type EdgeParseResult = Result<Option<(String, String, Option<String>, EdgeStyle)>, MermaidError>;

/// Parse an edge line. Returns (from_sid, to_sid, label, style) or None if unparseable.
fn parse_edge_line(line: &str, lineno: usize) -> EdgeParseResult {
    // Detect style — check most specific patterns first
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
    let has_incoming: std::collections::HashSet<usize> = edges.iter().map(|e| e.to).collect();

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
