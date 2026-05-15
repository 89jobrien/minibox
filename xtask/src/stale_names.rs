//! Lint for stale crate/binary names that were removed during the crate consolidation.
//!
//! Fails if any banned identifier appears outside the changelog, archive docs, or spec/plan
//! files that pre-date the consolidation (which document the migration itself).
//!
//! Run: `cargo xtask check-stale-names`

use anyhow::{Result, bail};
use std::path::{Path, PathBuf};

/// Names removed during the crate consolidation (v0.23.0).
///
/// These are banned from appearing in source code, CI workflows, scripts, and Cargo.toml files.
/// Historical plan/spec docs (`docs/superpowers/`) and CHANGELOG are exempt — they document the
/// migration.
const BANNED_NAMES: &[&str] = &[
    "dockerboxd",
    "dashbox",
    "linuxbox",
    "daemonbox",
    "minibox-client",
    "minibox-oci",
    "minibox-testers",
    "minibox-llm",
    "minibox_client",
    "minibox_oci",
    "minibox_testers",
    "minibox_llm",
    "daemonbox_state",
];

/// File extensions we scan (text files that could contain source or config).
const SCANNED_EXTENSIONS: &[&str] = &["rs", "toml", "yml", "yaml", "sh", "nu", "md"];

/// Path prefixes (relative to workspace root) that are exempt from the lint.
///
/// Files under these paths contain historical records of the migration and are
/// allowed to reference old names.
const EXEMPT_PREFIXES: &[&str] = &[
    "CHANGELOG",
    "docs/superpowers",
    "docs/DOCS_AUDIT",
    "docs/CRATE_TIERS",
    "docs/plans",
    ".claude/commands",
    "xtask/src/stale_names.rs",
    "docs/archive",
    "archive",
    ".git",
    ".worktrees",
    "target",
];

/// Check the workspace for banned stale crate/binary names.
pub fn check_stale_names(root: &Path) -> Result<()> {
    let mut violations: Vec<String> = Vec::new();
    let mut queue: Vec<PathBuf> = vec![root.to_path_buf()];

    while let Some(dir) = queue.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let rel = path.strip_prefix(root).unwrap_or(&path);
            let rel_str = rel.to_string_lossy();

            // Skip exempt paths.
            if EXEMPT_PREFIXES
                .iter()
                .any(|exempt| rel_str.starts_with(exempt) || rel_str.contains(exempt))
            {
                continue;
            }

            if path.is_dir() {
                queue.push(path);
                continue;
            }

            // Skip files without a scannable extension.
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or_default();
            if !SCANNED_EXTENSIONS.contains(&ext) {
                continue;
            }

            let content = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(_) => continue, // binary or unreadable — skip
            };

            for (lineno, line) in content.lines().enumerate() {
                for banned in BANNED_NAMES {
                    if line.contains(banned) {
                        violations.push(format!(
                            "  {}:{} — contains banned name `{}`\n    > {}",
                            rel.display(),
                            lineno + 1,
                            banned,
                            line.trim()
                        ));
                    }
                }
            }
        }
    }

    let n = violations.len();
    eprintln!("check-stale-names: {n} violation(s) found");
    if violations.is_empty() {
        Ok(())
    } else {
        bail!(
            "stale name violations — remove these references or move them to docs/superpowers/:\n\n{}",
            violations.join("\n")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write(dir: &Path, rel: &str, content: &str) {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    #[test]
    fn clean_workspace_passes() {
        let tmp = TempDir::new().unwrap();
        write(
            tmp.path(),
            "crates/minibox/src/lib.rs",
            "pub mod container_state;\n",
        );
        assert!(check_stale_names(tmp.path()).is_ok());
    }

    #[test]
    fn banned_name_in_source_fails() {
        let tmp = TempDir::new().unwrap();
        write(
            tmp.path(),
            "crates/minibox/src/lib.rs",
            "pub mod daemonbox_state;\n",
        );
        let err = check_stale_names(tmp.path()).unwrap_err();
        assert!(err.to_string().contains("daemonbox_state"));
    }

    #[test]
    fn changelog_is_exempt() {
        let tmp = TempDir::new().unwrap();
        write(
            tmp.path(),
            "CHANGELOG.md",
            "## v0.22.0\n- removed daemonbox\n",
        );
        assert!(check_stale_names(tmp.path()).is_ok());
    }

    #[test]
    fn docs_superpowers_is_exempt() {
        let tmp = TempDir::new().unwrap();
        write(
            tmp.path(),
            "docs/superpowers/plans/2026-03-17-macbox-daemonbox.md",
            "# Macbox daemonbox plan\ndaemonbox was the old name\n",
        );
        assert!(check_stale_names(tmp.path()).is_ok());
    }
}
