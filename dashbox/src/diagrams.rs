// dashbox/src/diagrams.rs — built-in diagram definitions
//
// Nodes and edges are defined independently, then composed into a Diagram via
// a `layout` grid.  New diagrams can reuse any subset of the node/edge pools.

use crate::diagram::{Diagram, Edge, EdgeStyle, Node, NodeKind};

/// The 3-tier git promotion pipeline: feature → main → next → stable → tag.
pub fn ci_flow() -> Diagram {
    Diagram {
        name: "CI Flow",
        nodes: vec![
            Node {
                id: 0,
                label: "feature/*",
                detail: "Short-lived branches targeting main. \
                         PRs trigger fmt+clippy+check gates. \
                         Auto-deleted on merge.",
                kind: NodeKind::Branch,
            },
            Node {
                id: 1,
                label: "main",
                detail: "Active R&D. CI gates: cargo check + \
                         fmt --check + clippy -D warnings. \
                         Direct push or PR merge.",
                kind: NodeKind::Branch,
            },
            Node {
                id: 2,
                label: "next",
                detail: "Auto-promoted from main on green CI via \
                         phased-deployment.yml. Extra gates: \
                         nextest + cargo audit + deny + machete.",
                kind: NodeKind::Branch,
            },
            Node {
                id: 3,
                label: "stable",
                detail: "Manual promotion from next via \
                         workflow_dispatch. Extra gates: \
                         cargo geiger + release build.",
                kind: NodeKind::Branch,
            },
            Node {
                id: 4,
                label: "v* tag",
                detail: "Versioned release cut from stable. \
                         Triggers release.yml: cross-compiled \
                         musl binaries + GitHub Release.",
                kind: NodeKind::Artifact,
            },
        ],
        edges: vec![
            Edge {
                from: 0,
                to: 1,
                label: Some("PR"),
                style: EdgeStyle::Solid,
            },
            Edge {
                from: 1,
                to: 2,
                label: Some("auto"),
                style: EdgeStyle::Solid,
            },
            Edge {
                from: 2,
                to: 3,
                label: Some("manual"),
                style: EdgeStyle::Manual,
            },
            Edge {
                from: 3,
                to: 4,
                label: Some("tag"),
                style: EdgeStyle::Dashed,
            },
        ],
        layout: vec![vec![0, 1, 2, 3, 4]],
    }
}

/// Local development loop: edit → check → pre-commit → commit → push → CI → next.
pub fn dev_loop() -> Diagram {
    Diagram {
        name: "Dev Loop",
        nodes: vec![
            Node {
                id: 0,
                label: "edit",
                detail: "Write Rust. Use bacon for fast watch-mode \
                         check. Shared CARGO_TARGET_DIR at \
                         ~/.mbx/cache/target/ across worktrees.",
                kind: NodeKind::Command,
            },
            Node {
                id: 1,
                label: "cargo chk",
                detail: "Fast type-check pass, no codegen. \
                         Catches most errors in <1s. \
                         Run via bacon or directly.",
                kind: NodeKind::Command,
            },
            Node {
                id: 2,
                label: "pre-commit",
                detail: "cargo xtask pre-commit: fmt --check + \
                         clippy -D warnings + release build. \
                         macOS-safe (no Linux-only crates).",
                kind: NodeKind::Command,
            },
            Node {
                id: 3,
                label: "commit",
                detail: "git commit. SSH-signed via 1Password agent. \
                         AI-generated message: just commit-msg. \
                         Hooks run obfsck secrets audit.",
                kind: NodeKind::Command,
            },
            Node {
                id: 4,
                label: "git push",
                detail: "just sync-check fetches+rebases onto \
                         origin/main first. Then pushes to remote, \
                         triggering GitHub Actions.",
                kind: NodeKind::Command,
            },
            Node {
                id: 5,
                label: "CI (GHA)",
                detail: "ci.yml: check+fmt+clippy on all branches. \
                         nextest+audit+deny+machete on next+stable. \
                         geiger on stable only.",
                kind: NodeKind::Job,
            },
            Node {
                id: 6,
                label: "next",
                detail: "phased-deployment.yml auto-promotes main→next \
                         after green CI. Triggers the full nextest \
                         + audit gate suite.",
                kind: NodeKind::Branch,
            },
        ],
        edges: vec![
            Edge {
                from: 0,
                to: 1,
                label: None,
                style: EdgeStyle::Solid,
            },
            Edge {
                from: 1,
                to: 2,
                label: None,
                style: EdgeStyle::Solid,
            },
            Edge {
                from: 2,
                to: 3,
                label: None,
                style: EdgeStyle::Solid,
            },
            Edge {
                from: 3,
                to: 4,
                label: None,
                style: EdgeStyle::Solid,
            },
            Edge {
                from: 4,
                to: 5,
                label: Some("triggers"),
                style: EdgeStyle::Solid,
            },
            Edge {
                from: 5,
                to: 6,
                label: Some("promotes"),
                style: EdgeStyle::Dashed,
            },
        ],
        layout: vec![vec![0, 1, 2, 3], vec![4, 5, 6]],
    }
}

/// Full container lifecycle from CLI request to running process.
pub fn container_lifecycle() -> Diagram {
    Diagram {
        name: "Container Lifecycle",
        nodes: vec![
            Node {
                id: 0,
                label: "run req",
                detail: "CLI sends RunContainer JSON over Unix socket \
                         at /run/minibox/miniboxd.sock. \
                         Protocol: JSON-over-newline.",
                kind: NodeKind::Command,
            },
            Node {
                id: 1,
                label: "auth",
                detail: "SO_PEERCRED on Unix socket. Kernel provides \
                         client UID/PID. Only UID 0 (root) permitted. \
                         Logged for audit trail.",
                kind: NodeKind::Job,
            },
            Node {
                id: 2,
                label: "img cache",
                detail: "Check /var/lib/minibox/images/ for cached \
                         layers. If missing, pulls from Docker Hub \
                         with anonymous token auth.",
                kind: NodeKind::Job,
            },
            Node {
                id: 3,
                label: "overlay",
                detail: "mount overlay: lowerdir=layers (read-only), \
                         upperdir=container_rw, workdir=container_work. \
                         Requires CLONE_NEWNS + root.",
                kind: NodeKind::Command,
            },
            Node {
                id: 4,
                label: "clone()",
                detail: "clone(2) with CLONE_NEWPID | CLONE_NEWNS | \
                         CLONE_NEWUTS | CLONE_NEWIPC | CLONE_NEWNET. \
                         Parent spawns reaper task for child PID.",
                kind: NodeKind::Command,
            },
            Node {
                id: 5,
                label: "pivot_root",
                detail: "Child: MS_PRIVATE propagation, bind-mount \
                         rootfs, pivot_root to container FS, \
                         unmount old root.",
                kind: NodeKind::Command,
            },
            Node {
                id: 6,
                label: "exec",
                detail: "execve() with explicit envp (not execvp). \
                         Closes extra FDs via close_range(). \
                         PID 1 inside container namespace.",
                kind: NodeKind::Command,
            },
        ],
        edges: vec![
            Edge {
                from: 0,
                to: 1,
                label: None,
                style: EdgeStyle::Solid,
            },
            Edge {
                from: 1,
                to: 2,
                label: None,
                style: EdgeStyle::Solid,
            },
            Edge {
                from: 2,
                to: 3,
                label: None,
                style: EdgeStyle::Solid,
            },
            Edge {
                from: 3,
                to: 4,
                label: None,
                style: EdgeStyle::Solid,
            },
            Edge {
                from: 4,
                to: 5,
                label: None,
                style: EdgeStyle::Solid,
            },
            Edge {
                from: 5,
                to: 6,
                label: None,
                style: EdgeStyle::Solid,
            },
        ],
        layout: vec![vec![0, 1, 2, 3], vec![4, 5, 6]],
    }
}

/// OCI image pull pipeline: reference parsing through layer extraction to cache.
pub fn image_pull() -> Diagram {
    Diagram {
        name: "Image Pull",
        nodes: vec![
            Node {
                id: 0,
                label: "ImageRef",
                detail: "Parse [REGISTRY/]NAMESPACE/NAME[:TAG]. \
                         Routes to correct registry adapter. \
                         Default: docker.io/library.",
                kind: NodeKind::Command,
            },
            Node {
                id: 1,
                label: "token auth",
                detail: "Docker Hub: POST /token with scope \
                         repository:pull. Returns short-lived JWT. \
                         Anonymous auth, no login required.",
                kind: NodeKind::Job,
            },
            Node {
                id: 2,
                label: "manifest",
                detail: "GET /v2/{name}/manifests/{ref}. \
                         Max size: 10MB. Parses OCI image manifest \
                         JSON for layer digest list.",
                kind: NodeKind::Job,
            },
            Node {
                id: 3,
                label: "layers",
                detail: "GET /v2/{name}/blobs/{digest} per layer. \
                         Max: 1GB/layer, 5GB total. \
                         Streamed to disk.",
                kind: NodeKind::Job,
            },
            Node {
                id: 4,
                label: "verify",
                detail: "SHA256 digest of downloaded blob compared \
                         against manifest entry. \
                         Reject on mismatch.",
                kind: NodeKind::Job,
            },
            Node {
                id: 5,
                label: "untar",
                detail: "Extract tar layer. Security checks: reject \
                         path traversal (..), absolute symlinks, \
                         device nodes. Strip setuid bits.",
                kind: NodeKind::Command,
            },
            Node {
                id: 6,
                label: "cached",
                detail: "Layers written to \
                         /var/lib/minibox/images/{name}/{digest}/. \
                         Ready for overlay mount.",
                kind: NodeKind::Artifact,
            },
        ],
        edges: vec![
            Edge {
                from: 0,
                to: 1,
                label: None,
                style: EdgeStyle::Solid,
            },
            Edge {
                from: 1,
                to: 2,
                label: None,
                style: EdgeStyle::Solid,
            },
            Edge {
                from: 2,
                to: 3,
                label: None,
                style: EdgeStyle::Solid,
            },
            Edge {
                from: 3,
                to: 4,
                label: None,
                style: EdgeStyle::Solid,
            },
            Edge {
                from: 4,
                to: 5,
                label: None,
                style: EdgeStyle::Solid,
            },
            Edge {
                from: 5,
                to: 6,
                label: None,
                style: EdgeStyle::Solid,
            },
        ],
        layout: vec![vec![0, 1, 2, 3], vec![4, 5, 6]],
    }
}

/// MINIBOX_ADAPTER selector: one env var, four runtime backends.
pub fn adapter_suite() -> Diagram {
    Diagram {
        name: "Adapter Suite",
        nodes: vec![
            Node {
                id: 0,
                label: "ADAPTER",
                detail: "MINIBOX_ADAPTER env var selects the adapter \
                         suite at daemon startup. Wired in \
                         miniboxd/src/main.rs.",
                kind: NodeKind::Command,
            },
            Node {
                id: 1,
                label: "native",
                detail: "Linux namespaces + cgroups v2 + overlay FS. \
                         Requires root. Default adapter. \
                         Full isolation.",
                kind: NodeKind::Job,
            },
            Node {
                id: 2,
                label: "gke",
                detail: "Unprivileged: proot + copy FS + no-op limiter. \
                         No root required. \
                         For GKE/restricted environments.",
                kind: NodeKind::Job,
            },
            Node {
                id: 3,
                label: "colima",
                detail: "macOS via limactl + nerdctl inside Colima VM. \
                         Routed through ColimaRuntime adapter. \
                         Requires Colima running.",
                kind: NodeKind::Job,
            },
            Node {
                id: 4,
                label: "vz",
                detail: "macOS Virtualization.framework: boots Alpine \
                         Linux VM, forwards commands via vsock. \
                         Requires --features vz + VM image.",
                kind: NodeKind::Job,
            },
        ],
        edges: vec![
            Edge {
                from: 0,
                to: 1,
                label: Some("native"),
                style: EdgeStyle::Solid,
            },
            Edge {
                from: 0,
                to: 2,
                label: Some("gke"),
                style: EdgeStyle::Solid,
            },
            Edge {
                from: 0,
                to: 3,
                label: Some("colima"),
                style: EdgeStyle::Solid,
            },
            Edge {
                from: 0,
                to: 4,
                label: Some("vz"),
                style: EdgeStyle::Dashed,
            },
        ],
        layout: vec![vec![0], vec![1, 2, 3, 4]],
    }
}
