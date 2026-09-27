//! CI change detection: classify changed paths into workspace areas.
//!
//! Invariant: every workspace member is classified into some `Area`, and any
//! `crates/` path that is not explicitly recognised falls back to
//! [`Area::Workspace`] so the full CI gate set runs. A newly added crate
//! therefore cannot silently skip lint and tests — see
//! `every_workspace_member_classifies` for the test that enforces this.

use anyhow::{Context, Result};
use std::path::Path;
use xshell::{Shell, cmd};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Area {
    Core,
    Daemon,
    Cli,
    Runtime,
    Macbox,
    Winbox,
    Conformance,
    Xtask,
    Docs,
    Workflows,
    /// Workspace-level config: Cargo.toml, Cargo.lock, rust-toolchain.toml,
    /// deny.toml, Justfile, and other root config files that affect all crates.
    ///
    /// Also the fallback for any `crates/` path not explicitly recognised, so a
    /// new crate defaults to the full gate set rather than to no gates.
    Workspace,
}

#[derive(Debug, Default, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub struct ChangeSet {
    pub core: bool,
    pub daemon: bool,
    pub cli: bool,
    pub runtime: bool,
    pub macbox: bool,
    pub winbox: bool,
    pub conformance: bool,
    pub xtask: bool,
    pub docs: bool,
    pub workflows: bool,
    pub workspace: bool,
}

impl ChangeSet {
    const fn set(&mut self, area: Area) {
        match area {
            Area::Core => self.core = true,
            Area::Daemon => self.daemon = true,
            Area::Cli => self.cli = true,
            Area::Runtime => self.runtime = true,
            Area::Macbox => self.macbox = true,
            Area::Winbox => self.winbox = true,
            Area::Conformance => self.conformance = true,
            Area::Xtask => self.xtask = true,
            Area::Docs => self.docs = true,
            Area::Workflows => self.workflows = true,
            Area::Workspace => self.workspace = true,
        }
    }
}

// ---------------------------------------------------------------------------
// Path classifier
// ---------------------------------------------------------------------------

/// Map a changed file path (relative to workspace root) to a workspace area.
///
/// Returns `None` only for paths that belong to no tracked area and are not
/// part of the product workspace (e.g. `fuzz/`, `cache/`, `traces/`).
///
/// Every `crates/` member is recognised explicitly. A `crates/` path with no
/// explicit mapping resolves to [`Area::Workspace`] so the full gate set runs —
/// this is deliberate: an unrecognised crate must fail safe (run everything),
/// not fail open (run nothing).
#[allow(clippy::case_sensitive_file_extension_comparisons)]
pub fn classify_path(path: &str) -> Option<Area> {
    if path.starts_with("crates/minibox-domain/")
        || path.starts_with("crates/minibox-core/")
        || path.starts_with("crates/minibox-macros/")
    {
        Some(Area::Core)
    } else if path.starts_with("crates/miniboxd/") {
        Some(Area::Daemon)
    } else if path.starts_with("crates/mbx/") {
        Some(Area::Cli)
    } else if path.starts_with("crates/minibox/") {
        Some(Area::Runtime)
    } else if path.starts_with("crates/smolbox/") {
        // krun / smolvm Linux microVM adapters.
        Some(Area::Runtime)
    } else if path.starts_with("crates/minibox-cni/") {
        // CNI network provider used by the runtime adapters.
        Some(Area::Runtime)
    } else if path.starts_with("crates/macbox/") {
        Some(Area::Macbox)
    } else if path.starts_with("crates/winbox/") {
        Some(Area::Winbox)
    } else if path.starts_with("crates/mcp/") {
        // MCP agent control surface — an operator/agent-facing interface.
        Some(Area::Cli)
    } else if path.starts_with("crates/minibox-tui/") {
        Some(Area::Cli)
    } else if path.starts_with("crates/ail/") {
        // Agent-improvement-loop tooling.
        Some(Area::Cli)
    } else if path.starts_with("crates/minibox-testsuite/")
        || path.starts_with("crates/minibox-crux-plugin/")
        || path.starts_with("crates/minibox-bench/")
    {
        Some(Area::Conformance)
    } else if path.starts_with("xtask/") || path.starts_with("scripts/") {
        Some(Area::Xtask)
    } else if path.starts_with("docs/") || (path.ends_with(".md") && !path.contains('/')) {
        Some(Area::Docs)
    } else if path.starts_with(".github/") {
        Some(Area::Workflows)
    } else if matches!(
        path,
        "Cargo.toml"
            | "Cargo.lock"
            | "rust-toolchain.toml"
            | "deny.toml"
            | "Justfile"
            | "clippy.toml"
            | ".rustfmt.toml"
            | "mise.toml"
            | "Dockerfile"
    ) || (path.ends_with(".toml") && !path.contains('/'))
    {
        Some(Area::Workspace)
    } else if path.starts_with("crates/") {
        // Default-deny catch-all: an unrecognised crate forces the full gate
        // set. Must stay last in this chain — see module docs.
        Some(Area::Workspace)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Run `git diff --name-only <base_ref>...HEAD` and classify changed paths.
///
/// If `base_ref` is a bare branch name (no `/`), it is prefixed with `origin/`
/// so that CI checkouts (which only create remote-tracking refs) resolve correctly.
pub fn detect_changes(root: &Path, base_ref: &str) -> Result<ChangeSet> {
    let sh = Shell::new()?;
    sh.change_dir(root);

    let resolved = if base_ref.contains('/') || base_ref.contains('^') || base_ref.contains('~') {
        base_ref.to_string()
    } else {
        format!("origin/{base_ref}")
    };
    let range = format!("{resolved}...HEAD");
    let output = cmd!(sh, "git diff --name-only {range}").read()?;

    let mut cs = ChangeSet::default();
    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(area) = classify_path(line) {
            cs.set(area);
        }
    }
    Ok(cs)
}

/// Serialise a `ChangeSet` to `key=value` output lines.
pub fn changeset_to_output_lines(cs: &ChangeSet) -> Vec<String> {
    vec![
        format!("core={}", cs.core),
        format!("daemon={}", cs.daemon),
        format!("cli={}", cs.cli),
        format!("runtime={}", cs.runtime),
        format!("macbox={}", cs.macbox),
        format!("winbox={}", cs.winbox),
        format!("conformance={}", cs.conformance),
        format!("xtask={}", cs.xtask),
        format!("docs={}", cs.docs),
        format!("workflows={}", cs.workflows),
        format!("workspace={}", cs.workspace),
    ]
}

/// Write outputs to `$GITHUB_OUTPUT` if set, otherwise print to stdout.
pub fn emit_gha_outputs(cs: &ChangeSet) -> Result<()> {
    use std::io::Write;

    let lines = changeset_to_output_lines(cs);

    if let Ok(output_path) = std::env::var("GITHUB_OUTPUT") {
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&output_path)
            .with_context(|| format!("failed to open GITHUB_OUTPUT: {output_path}"))?;
        for line in &lines {
            writeln!(f, "{line}")?;
        }
    } else {
        for line in &lines {
            println!("{line}");
        }
    }
    Ok(())
}

/// Classifies paths changed from the base ref to `HEAD` and emits CI area flags.
pub fn run(root: &Path, base_ref: &str) -> Result<()> {
    let cs = detect_changes(root, base_ref)?;
    emit_gha_outputs(&cs)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Every workspace member must classify to some area, and the area must be
    /// one the CI gate set actually reacts to.
    ///
    /// This is the regression test for the gate escape where six members
    /// (`smolbox`, `ail`, `minibox-bench`, `mcp`, `minibox-cni`,
    /// `minibox-tui`) had no branch in `classify_path`, returned `None`, and
    /// therefore produced all-false flags — so a PR touching only one of them
    /// skipped lint and unit tests and still went green.
    ///
    /// Reads the live `Cargo.toml` via `cargo_metadata` so it cannot drift out
    /// of sync with the real member list.
    #[test]
    fn every_workspace_member_classifies() {
        let metadata = cargo_metadata::MetadataCommand::new()
            .no_deps()
            .exec()
            .expect("failed to read workspace metadata");

        let mut members: Vec<String> = metadata
            .workspace_members
            .iter()
            .map(|id| {
                metadata[id]
                    .manifest_path
                    .parent()
                    .and_then(|p| p.strip_prefix(metadata.workspace_root.as_std_path()).ok())
                    .map_or_else(
                        || panic!("member {id} is not inside the workspace root"),
                        |p| p.as_str().to_owned(),
                    )
            })
            .collect();
        members.sort();

        assert!(
            members.len() >= 16,
            "expected the full product workspace, got {} members: {members:?}",
            members.len()
        );

        for member in &members {
            let probe = format!("{member}/src/lib.rs");
            let area = classify_path(&probe)
                .unwrap_or_else(|| panic!("{member} classifies to None — CI would skip it"));
            assert_ne!(
                area,
                Area::Docs,
                "{member} must not classify to the Docs area"
            );
        }
    }

    /// An unrecognised crate must fail safe (full gate set), not fail open.
    #[test]
    fn unrecognised_crate_forces_full_gate_set() {
        assert_eq!(
            classify_path("crates/some-future-crate/src/lib.rs"),
            Some(Area::Workspace),
            "a crate with no explicit mapping must fall back to Workspace so \
             lint and unit tests still run"
        );
        assert_eq!(
            classify_path("crates/nested/deep/src/lib.rs"),
            Some(Area::Workspace)
        );
    }

    /// Non-product paths stay out of the gate set — they belong to separate
    /// workspaces or are build artifacts.
    #[test]
    fn non_product_paths_are_unclassified() {
        for path in [
            "fuzz/fuzz_targets/anything.rs",
            "cache/layer.tar",
            "traces/trace.json",
            "artifacts/report.json",
        ] {
            assert_eq!(
                classify_path(path),
                None,
                "{path} should not be classified — it is not a product workspace path"
            );
        }
    }

    #[test]
    fn classify_minibox_core() {
        assert_eq!(
            classify_path("crates/minibox-core/src/domain.rs"),
            Some(Area::Core)
        );
    }

    #[test]
    fn classify_minibox_domain() {
        assert_eq!(
            classify_path("crates/minibox-domain/src/runtime.rs"),
            Some(Area::Core)
        );
    }

    #[test]
    fn classify_minibox_macros() {
        assert_eq!(
            classify_path("crates/minibox-macros/src/lib.rs"),
            Some(Area::Core)
        );
    }

    #[test]
    fn classify_miniboxd() {
        assert_eq!(
            classify_path("crates/miniboxd/src/handler.rs"),
            Some(Area::Daemon)
        );
    }

    #[test]
    fn classify_mbx_cli() {
        assert_eq!(classify_path("crates/mbx/src/main.rs"), Some(Area::Cli));
    }

    #[test]
    fn classify_minibox_runtime() {
        assert_eq!(
            classify_path("crates/minibox/src/adapters/docker.rs"),
            Some(Area::Runtime)
        );
    }

    #[test]
    fn classify_macbox() {
        assert_eq!(
            classify_path("crates/macbox/src/krun.rs"),
            Some(Area::Macbox)
        );
    }

    #[test]
    fn classify_winbox() {
        assert_eq!(
            classify_path("crates/winbox/src/lib.rs"),
            Some(Area::Winbox)
        );
    }

    #[test]
    fn classify_conformance() {
        assert_eq!(
            classify_path("crates/minibox-testsuite/src/lib.rs"),
            Some(Area::Conformance)
        );
    }

    #[test]
    fn classify_xtask() {
        assert_eq!(classify_path("xtask/src/gates.rs"), Some(Area::Xtask));
    }

    #[test]
    fn classify_docs_subdir() {
        assert_eq!(classify_path("docs/ARCHITECTURE.mbx.md"), Some(Area::Docs));
    }

    #[test]
    fn classify_root_md() {
        assert_eq!(classify_path("README.md"), Some(Area::Docs));
        assert_eq!(classify_path("CHANGELOG.md"), Some(Area::Docs));
    }

    #[test]
    fn classify_workflows() {
        assert_eq!(
            classify_path(".github/workflows/pr.yml"),
            Some(Area::Workflows)
        );
    }

    #[test]
    fn classify_scripts() {
        assert_eq!(classify_path("scripts/preflight.nu"), Some(Area::Xtask));
        assert_eq!(classify_path("scripts/ci-watch.nu"), Some(Area::Xtask));
    }

    #[test]
    fn classify_workspace_root_files() {
        assert_eq!(classify_path("Cargo.toml"), Some(Area::Workspace));
        assert_eq!(classify_path("Cargo.lock"), Some(Area::Workspace));
        assert_eq!(classify_path("rust-toolchain.toml"), Some(Area::Workspace));
        assert_eq!(classify_path("deny.toml"), Some(Area::Workspace));
        assert_eq!(classify_path("Justfile"), Some(Area::Workspace));
        assert_eq!(classify_path("Dockerfile"), Some(Area::Workspace));
        assert_eq!(classify_path("clippy.toml"), Some(Area::Workspace));
    }

    #[test]
    fn classify_unknown_returns_none() {
        assert_eq!(classify_path("fuzz/corpus/something"), None);
        assert_eq!(classify_path("assets/logo.png"), None);
    }

    #[test]
    fn changeset_folds_multiple_paths() {
        let paths = [
            "crates/minibox-core/src/protocol.rs",
            "crates/miniboxd/src/handler.rs",
            "docs/FEATURE_MATRIX.mbx.md",
        ];
        let mut cs = ChangeSet::default();
        for p in &paths {
            if let Some(area) = classify_path(p) {
                cs.set(area);
            }
        }
        assert!(cs.core);
        assert!(cs.daemon);
        assert!(cs.docs);
        assert!(!cs.cli);
        assert!(!cs.runtime);
    }

    #[test]
    fn emit_outputs_formats_correctly() {
        let cs = ChangeSet {
            core: true,
            daemon: false,
            cli: true,
            runtime: false,
            macbox: false,
            winbox: false,
            conformance: false,
            xtask: false,
            docs: false,
            workflows: false,
            workspace: false,
        };
        let lines = changeset_to_output_lines(&cs);
        assert!(lines.contains(&"core=true".to_string()));
        assert!(lines.contains(&"daemon=false".to_string()));
        assert!(lines.contains(&"cli=true".to_string()));
    }

    #[test]
    fn detect_changes_with_real_git() {
        use std::process::Command;
        use tempfile::TempDir;

        let dir = TempDir::new().expect("tempdir");
        let root = dir.path();

        let git = |args: &[&str]| {
            Command::new("git")
                .args(args)
                .current_dir(root)
                .output()
                .expect("git")
        };

        git(&["init", "-b", "main"]);
        git(&["config", "user.email", "test@test.com"]);
        git(&["config", "user.name", "Test"]);

        // First commit: a core file.
        std::fs::create_dir_all(root.join("crates/minibox-core/src")).expect("mkdir");
        std::fs::write(root.join("crates/minibox-core/src/lib.rs"), b"// v1").expect("write");
        git(&["add", "."]);
        git(&["commit", "-m", "initial"]);

        // Second commit: touch core + docs.
        std::fs::write(root.join("crates/minibox-core/src/lib.rs"), b"// v2").expect("write");
        std::fs::create_dir_all(root.join("docs")).expect("mkdir");
        std::fs::write(root.join("docs/ARCHITECTURE.md"), b"# arch").expect("write");
        git(&["add", "."]);
        git(&["commit", "-m", "update"]);

        let cs = detect_changes(root, "HEAD^").expect("detect_changes");
        assert!(cs.core, "core should be true");
        assert!(cs.docs, "docs should be true");
        assert!(!cs.daemon, "daemon should be false");
        assert!(!cs.cli, "cli should be false");
    }
}
