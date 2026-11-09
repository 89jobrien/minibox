//! Component-removal audit: automates the manual checklist in `AGENTS.md`
//! ("Component Removal Audit Checklist") so deleting a crate, module, file,
//! feature, or dependency doesn't rely on an agent remembering to grep every
//! extension by hand. 18 instances of incomplete grep/audit prompted this —
//! see AGENTS.md and the reference commit `833692a7`.

use anyhow::{Context, Result, bail};
use std::path::Path;
use std::process::Command;

/// One match of the removed component's name in a tracked file.
#[derive(Debug, Clone)]
pub struct Hit {
    pub path: String,
    pub line: usize,
    pub text: String,
}

/// Grouped findings for one audited component name.
#[derive(Debug, Default)]
pub struct RemovalAuditReport {
    pub rust_hits: Vec<Hit>,
    pub toml_hits: Vec<Hit>,
    pub md_hits: Vec<Hit>,
    pub cargo_tree_dependents: Vec<String>,
}

impl RemovalAuditReport {
    pub const fn total_hits(&self) -> usize {
        self.rust_hits.len() + self.toml_hits.len() + self.cargo_tree_dependents.len()
    }
}

/// Run the removal audit for `name` against every tracked file under `root`.
///
/// Walks `.rs`, `.toml`, and `.md` files the same way `git grep` would (but
/// without a git dependency), and additionally runs `cargo tree -i <name>`
/// in case `name` is itself a crate name still present in the dependency
/// graph. Markdown hits are collected for visibility (e.g. a CHANGELOG entry
/// is expected to mention a removed component) but excluded from
/// `total_hits`, since a historical mention there is not itself a defect.
pub fn run(root: &Path, name: &str) -> Result<()> {
    if name.trim().is_empty() {
        bail!("removal-audit: component name must not be empty");
    }

    let mut report = RemovalAuditReport::default();
    let mut files = Vec::new();
    collect_files(root, &mut files)?;

    for path in &files {
        let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
            continue;
        };
        let bucket = match ext {
            "rs" => Some(&mut report.rust_hits),
            "toml" => Some(&mut report.toml_hits),
            "md" => Some(&mut report.md_hits),
            _ => None,
        };
        let Some(bucket) = bucket else { continue };
        let Ok(content) = std::fs::read_to_string(path) else {
            continue;
        };
        let rel = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();
        for (i, line) in content.lines().enumerate() {
            if line.contains(name) {
                bucket.push(Hit {
                    path: rel.clone(),
                    line: i + 1,
                    text: line.trim().to_string(),
                });
            }
        }
    }

    report.cargo_tree_dependents = run_cargo_tree(root, name);

    print_report(name, &report);

    if report.total_hits() > 0 {
        bail!(
            "removal-audit: {} residual reference(s) to \"{name}\" found (rust: {}, toml: {}, cargo-tree dependents: {})",
            report.total_hits(),
            report.rust_hits.len(),
            report.toml_hits.len(),
            report.cargo_tree_dependents.len(),
        );
    }

    Ok(())
}

fn print_report(name: &str, report: &RemovalAuditReport) {
    eprintln!("removal-audit: \"{name}\"");
    eprintln!(
        "  rust: {}  toml: {}  cargo-tree dependents: {}  (markdown mentions, informational: {})",
        report.rust_hits.len(),
        report.toml_hits.len(),
        report.cargo_tree_dependents.len(),
        report.md_hits.len(),
    );
    for (label, hits) in [("rust", &report.rust_hits), ("toml", &report.toml_hits)] {
        for hit in hits {
            eprintln!("    [{label}] {}:{}: {}", hit.path, hit.line, hit.text);
        }
    }
    for dep in &report.cargo_tree_dependents {
        eprintln!("    [cargo-tree] {dep}");
    }
    if !report.md_hits.is_empty() {
        eprintln!("  markdown mentions (review manually; not counted as a failure):");
        for hit in &report.md_hits {
            eprintln!("    [md] {}:{}: {}", hit.path, hit.line, hit.text);
        }
    }
}

/// `cargo tree -i <name>` lists reverse dependents of a crate still present
/// in the workspace graph. A non-crate name (module, file, feature) makes
/// this fail harmlessly — that's not itself a finding, just not applicable.
fn run_cargo_tree(root: &Path, name: &str) -> Vec<String> {
    let output = Command::new("cargo")
        .args(["tree", "-i", name])
        .current_dir(root)
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(str::to_string)
        .collect()
}

const SKIP_DIRS: &[&str] = &["target", "node_modules", ".git"];

fn collect_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) -> Result<()> {
    let entries = std::fs::read_dir(dir).with_context(|| format!("read_dir {}", dir.display()))?;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            if path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| SKIP_DIRS.contains(&n) || n.starts_with('.') && n != ".")
            {
                continue;
            }
            collect_files(&path, out)?;
        } else {
            out.push(path);
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn empty_name_rejected() {
        let temp = tempfile::tempdir().expect("tempdir");
        let err = run(temp.path(), "").unwrap_err();
        assert!(err.to_string().contains("must not be empty"));
    }

    #[test]
    fn finds_rust_and_toml_hits_but_not_markdown() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(temp.path().join("lib.rs"), "use linuxbox::Foo;\n").unwrap();
        std::fs::write(
            temp.path().join("Cargo.toml"),
            "[dependencies]\nlinuxbox = \"0.1\"\n",
        )
        .unwrap();
        std::fs::write(
            temp.path().join("CHANGELOG.md"),
            "- renamed linuxbox to mbx\n",
        )
        .unwrap();

        let mut report = RemovalAuditReport::default();
        let mut files = Vec::new();
        collect_files(temp.path(), &mut files).expect("collect");
        for path in &files {
            let ext = path.extension().and_then(|e| e.to_str());
            let content = std::fs::read_to_string(path).unwrap();
            let rel = path
                .strip_prefix(temp.path())
                .unwrap()
                .display()
                .to_string();
            for (i, line) in content.lines().enumerate() {
                if line.contains("linuxbox") {
                    let hit = Hit {
                        path: rel.clone(),
                        line: i + 1,
                        text: line.to_string(),
                    };
                    match ext {
                        Some("rs") => report.rust_hits.push(hit),
                        Some("toml") => report.toml_hits.push(hit),
                        Some("md") => report.md_hits.push(hit),
                        _ => {}
                    }
                }
            }
        }

        assert_eq!(report.rust_hits.len(), 1);
        assert_eq!(report.toml_hits.len(), 1);
        assert_eq!(report.md_hits.len(), 1);
        assert_eq!(report.total_hits(), 2);
    }

    #[test]
    fn clean_component_passes() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(temp.path().join("lib.rs"), "fn main() {}\n").unwrap();
        run(temp.path(), "definitely_not_present_anywhere").expect("should pass");
    }
}
