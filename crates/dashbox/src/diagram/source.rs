// dashbox/src/diagram/source.rs

use std::path::PathBuf;

use crate::diagram::OwnedDiagram;
use crate::diagram::mermaid;

pub enum DiagramSource {
    /// Built-in diagram embedded as a `&'static str` via `include_str!`.
    Embedded {
        #[allow(dead_code)]
        name: &'static str,
        src: &'static str,
    },
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
                Err(e) => mermaid::parse(&format!("%% error loading file: {e}")),
            },
        }
    }
}

/// All built-in diagrams as embedded Mermaid sources.
pub fn built_in_diagrams() -> Vec<OwnedDiagram> {
    let sources: &[(&str, &str)] = &[
        ("CI Flow", include_str!("../diagrams/ci_flow.mmd")),
        ("Dev Loop", include_str!("../diagrams/dev_loop.mmd")),
        (
            "Container Lifecycle",
            include_str!("../diagrams/container_lifecycle.mmd"),
        ),
        ("Image Pull", include_str!("../diagrams/image_pull.mmd")),
        (
            "Adapter Suite",
            include_str!("../diagrams/adapter_suite.mmd"),
        ),
        (
            "Workspace Deps",
            include_str!("../diagrams/workspace_deps.mmd"),
        ),
    ];
    sources
        .iter()
        .map(|(name, src)| DiagramSource::Embedded { name, src }.load())
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
            .filter(|p| p.extension().is_some_and(|ext| ext == "mmd"))
            .collect(),
        Err(_) => return Vec::new(),
    };
    paths.sort();
    paths
        .into_iter()
        .map(|p| DiagramSource::File(p).load())
        .collect()
}
