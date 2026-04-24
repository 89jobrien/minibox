---
title: Mermaid Diagram Engine for Dashbox
status: draft
date: 2026-03-30
---

# Mermaid Diagram Engine — Design Spec

## Problem

Dashbox's diagrams are hand-coded Rust in `diagrams.rs`. Adding or updating a diagram requires
editing Rust source, recompiling, and knowing the `Node`/`Edge`/`layout` API. The goal is to make
Mermaid the authoring format while keeping the existing navigable `Diagram` domain model intact.

## Design Goals

1. **Mermaid is the source of truth** for diagram structure — nodes, edges, layout hints, detail text.
2. **`Diagram` stays unchanged** — the navigable domain model, renderer, and navigation code are not
   touched.
3. **Two source origins** — embedded statics (built-in diagrams, no I/O) and file-loaded
   (`~/.minibox/diagrams/*.mmd`, no recompile needed for user diagrams).
4. **Full navigation preserved** — parsed diagrams work identically to hand-coded ones: arrow-key
   navigation, Tab cycling, detail panel, edge list.
5. **No new runtime dependencies** — parser is hand-rolled (the Mermaid subset is small).

## Non-Goals

- Full Mermaid spec support (subgraphs, classDef, click handlers, etc.)
- Live-reload of `.mmd` files while dashbox is running
- Rendering Mermaid SVG/HTML (TUI ASCII rendering is unchanged)

---

## Domain Model (unchanged)

```
Diagram { name, nodes: Vec<Node>, edges: Vec<Edge>, layout: Vec<Vec<usize>> }
Node    { id: usize, label: &'static str, detail: &'static str, kind: NodeKind }
Edge    { from: usize, to: usize, label: Option<&'static str>, style: EdgeStyle }
```

`Diagram` is the port. The Mermaid parser is an adapter that produces it.

For file-loaded diagrams, `label`/`detail` are owned `String`s, not `&'static str`. This requires
a small extension to `Node` and `Edge` (see § Data Model Changes).

---

## Mermaid Subset

Only `flowchart`/`graph` diagrams are supported. Supported syntax:

### Graph declaration

```
graph LR
flowchart LR
```

Direction is parsed but ignored — layout comes from `%% layout:` hints or topological order.

### Nodes

```
A[label]          →  NodeKind::Command   (rectangle)
A([label])        →  NodeKind::Branch    (rounded)
A{label}          →  NodeKind::Job       (diamond)
A>label<          →  NodeKind::Artifact  (asymmetric / flag)
```

Node IDs are arbitrary identifiers. Labels are the display text.

### Edges

```
A --> B            →  EdgeStyle::Solid,   no label
A -->|text| B      →  EdgeStyle::Solid,   label = "text"
A -.-> B           →  EdgeStyle::Dashed,  no label
A -.->|text| B     →  EdgeStyle::Dashed,  label = "text"
A ==> B            →  EdgeStyle::Manual,  no label
A ==>|text| B      →  EdgeStyle::Manual,  label = "text"
```

### Extension comments

Standard Mermaid comments (`%% ...`) are used for metadata. These are ignored by any external
Mermaid renderer, so `.mmd` files remain valid for web preview tools.

```
%% detail: tooltip text for the most recently declared node
%% kind: branch|job|command|artifact   (override shape-inferred kind)
%% layout: A B C                       (assign node IDs to a row, in order)
%% name: Human-readable diagram title
```

`%% detail:` applies to the **last node declared above it** (or the last node in the same block).
`%% layout:` lines are collected in order; each defines one row of the layout grid.
`%% name:` sets `Diagram::name`. If absent, the filename stem is used.

### Example

```mermaid
graph LR
  A([feature/*])
  %% detail: Short-lived branches. PRs trigger fmt+clippy+check gates.
  B([main])
  %% detail: Active R&D. CI gates: check + fmt + clippy -D warnings.
  C([next])
  %% detail: Auto-promoted from main on green CI.
  D([stable])
  %% detail: Manual promotion from next via workflow_dispatch.
  E>v* tag<
  %% detail: Triggers release.yml: musl binaries + GitHub Release.

  A -->|PR| B
  B -->|auto| C
  C ==>|manual| D
  D -.->|tag| E

%% name: CI Flow
%% layout: A B C D E
```

---

## Architecture

### New files

```
crates/dashbox/src/
  diagram/
    mod.rs          ← existing diagram.rs content (move + re-export)
    mermaid.rs      ← parser: &str → OwnedDiagram
    source.rs       ← DiagramSource enum + loader
  diagrams/
    ci_flow.mmd
    dev_loop.mmd
    container_lifecycle.mmd
    image_pull.mmd
    adapter_suite.mmd
    workspace_deps.mmd
```

### Modified files

```
crates/dashbox/src/
  diagram.rs          ← replaced by diagram/mod.rs (or kept as thin re-export)
  diagrams.rs         ← replaced: returns DiagramSource::Embedded for each built-in
  tabs/diagrams.rs    ← DiagramsTab holds Vec<OwnedDiagram> instead of 6 named fields
  main.rs             ← load file diagrams at startup, pass into DiagramsTab::new()
```

---

## Data Model Changes

The current `Node` uses `&'static str` for `label` and `detail`. File-loaded diagrams need owned
strings. Two options:

**Option A — owned strings everywhere**: Change `Node` to use `String`. Built-in diagrams pay a
small heap allocation at startup. Simplest.

**Option B — `Cow<'static, str>`**: `label: Cow<'static, str>`. Static strings stay zero-copy;
parsed strings allocate. More correct but more noise in match arms.

**Decision: Option A** (owned strings). The existing static diagrams are created once at startup;
the allocation cost is negligible. This avoids `Cow` throughout the render/nav code.

### `OwnedDiagram`

A type alias or newtype:

```rust
// diagram/mod.rs
pub struct OwnedNode {
    pub id: usize,
    pub label: String,
    pub detail: String,
    pub kind: NodeKind,
}

pub struct OwnedEdge {
    pub from: usize,
    pub to: usize,
    pub label: Option<String>,
    pub style: EdgeStyle,
}

pub struct OwnedDiagram {
    pub name: String,
    pub nodes: Vec<OwnedNode>,
    pub edges: Vec<OwnedEdge>,
    pub layout: Vec<Vec<usize>>,
    // navigation helpers (same logic as current Diagram methods)
}
```

The renderer and nav code move to `OwnedDiagram`. The existing `Diagram`/`Node`/`Edge` with static
strings are removed (they were only used internally).

---

## Parser Design (`mermaid.rs`)

```rust
/// Parse a Mermaid flowchart/graph source string into an OwnedDiagram.
/// Returns Err with a human-readable message on syntax errors.
pub fn parse(src: &str) -> Result<OwnedDiagram, MermaidError>
```

### Parse algorithm

1. Strip lines starting with `%%` into a metadata pass; non-comment lines go to the node/edge pass.
2. **Metadata pass**: collect `name`, `detail` annotations (keyed to preceding node ID), `kind`
   overrides, `layout` rows.
3. **Node/edge pass**: line-by-line regex-free tokenization:
   - If line contains `-->`, `-.->`, or `==>`: parse as edge.
   - Else if line matches `ID[...]`, `ID([...])`, `ID{...}`, `ID>...<`: parse as node.
   - `graph`/`flowchart` declaration line: skip.
   - Blank lines: skip.
4. **Layout inference**: if no `%% layout:` hints, assign nodes to rows via BFS from root nodes
   (nodes with no incoming edges). Nodes at the same BFS depth share a row.
5. **ID → usize mapping**: node string IDs (e.g. `"A"`, `"feature"`) are mapped to monotonically
   increasing `usize` IDs in declaration order.
6. **Detail assignment**: `%% detail:` comment applies to the last node declared before that line.

### Error handling

`MermaidError` carries the line number and a message. Parse errors are non-fatal at the tab level:
a failed diagram is shown as a single "parse error" node with the error message as detail text.
This avoids crashing the TUI on a malformed user `.mmd` file.

---

## `DiagramSource` and Loader (`source.rs`)

```rust
pub enum DiagramSource {
    /// Built-in: Mermaid source embedded as a &'static str constant.
    Embedded { name: &'static str, src: &'static str },
    /// User-defined: path to a .mmd file, loaded at startup.
    File(PathBuf),
}

impl DiagramSource {
    /// Load and parse into OwnedDiagram. Called once at startup.
    pub fn load(&self) -> OwnedDiagram { ... }
}

/// Load all .mmd files from ~/.minibox/diagrams/, returning one DiagramSource per file.
pub fn load_user_diagrams() -> Vec<DiagramSource> { ... }
```

Built-in diagrams use `include_str!("../../diagrams/ci_flow.mmd")` so the `.mmd` files are
compiled into the binary. No I/O at runtime for built-ins.

---

## `DiagramsTab` changes

```rust
pub struct DiagramsTab {
    diagrams: Vec<OwnedDiagram>,  // replaces 6 named fields
    active: usize,                // index into diagrams
    selected: usize,              // selected node id
    ci_source: CachedSource<CiSource>,
}

impl DiagramsTab {
    pub fn new(extra: Vec<OwnedDiagram>) -> Self {
        let mut diagrams = built_in_diagrams();  // parse embedded statics
        diagrams.extend(extra);                  // append user file diagrams
        let first = diagrams[0].first_node().unwrap_or(0);
        Self { diagrams, active: 0, selected: first, ci_source: ... }
    }
}
```

`d`/`D` key cycles `active` index. The `View` enum is removed; diagram name comes from
`OwnedDiagram::name`. The `(1/6)` counter in the title bar still works: `active+1 / diagrams.len()`.

---

## `main.rs` wiring

```rust
let user_diagrams = source::load_user_diagrams()
    .into_iter()
    .map(|s| s.load())
    .collect();

let diagrams_tab = DiagramsTab::new(user_diagrams);
```

---

## Migration: `diagrams.rs` → `.mmd` files

Each of the 6 existing hand-coded `Diagram` constructors is converted to a `.mmd` file and deleted
from `diagrams.rs`. `diagrams.rs` is replaced by `built_in_diagrams()` in `source.rs` (or a thin
`diagrams.rs` that calls it).

Order of migration:

1. Write parser + `OwnedDiagram` types
2. Add `DiagramSource::Embedded` + `load()`
3. Convert `ci_flow` as a smoke test
4. Convert remaining 5 diagrams
5. Delete old `Diagram`/`Node`/`Edge` static-str types
6. Add file loader + `main.rs` wiring

---

## Testing

- **Unit tests in `mermaid.rs`**: round-trip a known `.mmd` snippet, assert node count, edge count,
  layout rows, detail text, kind inference, layout BFS fallback.
- **Parse error test**: malformed input produces `MermaidError`, not panic.
- **File loader test**: write a temp `.mmd` file, call `load_user_diagrams()` with overridden dir,
  assert `OwnedDiagram` name matches.
- **Navigation smoke**: parse a diagram, call `navigate()` in all four directions, assert no panic
  and correct node IDs returned.

No new dependencies needed. Tests run on any platform (`cargo test -p dashbox`).

---

## Open Questions (resolved)

| Question               | Decision                                                      |
| ---------------------- | ------------------------------------------------------------- |
| Detail text in `.mmd`? | `%% detail:` comment convention (Option B from brainstorm)    |
| String ownership?      | `OwnedDiagram` with `String` fields (Option A — simplest)     |
| Diagram source?        | Both embedded statics and `~/.minibox/diagrams/` file loading |
| Layout?                | `%% layout:` hints; BFS topological fallback                  |
| Parse errors?          | Non-fatal — render as single error node in TUI                |
