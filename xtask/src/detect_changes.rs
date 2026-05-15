//! CI change detection: classify changed paths into workspace areas.

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
}

#[derive(Debug, Default, PartialEq, Eq)]
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
}

impl ChangeSet {
    fn set(&mut self, area: Area) {
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
        }
    }
}

// ---------------------------------------------------------------------------
// Path classifier
// ---------------------------------------------------------------------------

/// Map a changed file path (relative to workspace root) to a workspace area.
///
/// Returns `None` for paths that don't match any tracked area (e.g. `fuzz/`).
pub fn classify_path(path: &str) -> Option<Area> {
    if path.starts_with("crates/minibox-core/") || path.starts_with("crates/minibox-macros/") {
        Some(Area::Core)
    } else if path.starts_with("crates/miniboxd/") {
        Some(Area::Daemon)
    } else if path.starts_with("crates/mbx/") {
        Some(Area::Cli)
    } else if path.starts_with("crates/minibox/") {
        Some(Area::Runtime)
    } else if path.starts_with("crates/macbox/") {
        Some(Area::Macbox)
    } else if path.starts_with("crates/winbox/") {
        Some(Area::Winbox)
    } else if path.starts_with("crates/minibox-testsuite/")
        || path.starts_with("crates/minibox-crux-plugin/")
    {
        Some(Area::Conformance)
    } else if path.starts_with("xtask/") {
        Some(Area::Xtask)
    } else if path.starts_with("docs/") || (path.ends_with(".md") && !path.contains('/')) {
        Some(Area::Docs)
    } else if path.starts_with(".github/") {
        Some(Area::Workflows)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Run `git diff --name-only <base_ref>...HEAD` and classify changed paths.
pub fn detect_changes(root: &Path, base_ref: &str) -> Result<ChangeSet> {
    let sh = Shell::new()?;
    sh.change_dir(root);

    let range = format!("{base_ref}...HEAD");
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

/// Serialise a ChangeSet to `key=value` output lines.
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

    #[test]
    fn classify_minibox_core() {
        assert_eq!(
            classify_path("crates/minibox-core/src/domain.rs"),
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
    fn classify_unknown_returns_none() {
        assert_eq!(classify_path("fuzz/corpus/something"), None);
        assert_eq!(classify_path("Cargo.lock"), None);
        assert_eq!(classify_path("scripts/preflight.nu"), None);
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
