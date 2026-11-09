//! Markdown frontmatter linting and SARIF diagnostic generation.

use anyhow::{Context, Result, bail};
use std::path::Path;

use crate::frontmatter::{self, ContractViolation};
use crate::sarif::{self, Diagnostic, Level};

/// Allowed values for the `status` frontmatter field in plans and specs.
const ALLOWED_STATUSES: &[&str] = &[
    "open",
    "done",
    "archived",
    "approved",
    "draft",
    "active",
    "proposed",
    "superseded",
    "partial",
];

/// Lint all Markdown files under `docs/`.
///
/// Checks:
/// 1. If YAML frontmatter delimiters (`---`) are present, every non-blank line
///    between them must be a valid `key: value` pair or a YAML list item
///    (a line starting with `-`).
/// 2. If a `status` key exists, its value must be one of [`ALLOWED_STATUSES`].
pub fn lint_docs(root: &Path, sarif_path: Option<&Path>) -> Result<()> {
    let mut diagnostics: Vec<Diagnostic> = Vec::new();
    let mut checked = 0u32;

    let dir = root.join("docs");
    if dir.is_dir() {
        let mut md_files = Vec::new();
        collect_markdown_files(&dir, &mut md_files)?;
        for path in md_files {
            checked += 1;
            lint_file_diagnostics(&path, root, &mut diagnostics);
        }
    }

    eprintln!(
        "docs-lint: checked {checked} files, {n} error(s)",
        n = diagnostics.len()
    );

    if let Some(sarif_out) = sarif_path {
        let log =
            sarif::from_diagnostics("minibox-docs-lint", env!("CARGO_PKG_VERSION"), &diagnostics);
        log.write_to(sarif_out)?;
    }

    if diagnostics.is_empty() {
        Ok(())
    } else {
        let errors: Vec<String> = diagnostics
            .iter()
            .map(|d| {
                format!(
                    "  {}: {}",
                    d.file.as_deref().unwrap_or("unknown"),
                    d.message
                )
            })
            .collect();
        bail!("docs-lint failed:\n{}", errors.join("\n"));
    }
}

/// Recursively collect every `.md` file under `dir`.
fn collect_markdown_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) -> Result<()> {
    let entries = std::fs::read_dir(dir).with_context(|| format!("read_dir {}", dir.display()))?;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_markdown_files(&path, out)?;
        } else if path.extension().is_some_and(|e| e == "md") {
            out.push(path);
        }
    }
    Ok(())
}

fn lint_file_diagnostics(path: &Path, root: &Path, diagnostics: &mut Vec<Diagnostic>) {
    let relative = path
        .strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .to_string();

    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            diagnostics.push(Diagnostic {
                rule_id: "docs-lint/read-error".into(),
                level: Level::Error,
                message: format!("failed to read: {e}"),
                file: Some(relative),
                line: None,
                column: None,
            });
            return;
        }
    };

    let Some(fm) = frontmatter::frontmatter_block(&content) else {
        return;
    };

    for (i, line) in fm.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !trimmed.starts_with('-') && !trimmed.contains(':') {
            diagnostics.push(Diagnostic {
                rule_id: "docs-lint/malformed-frontmatter".into(),
                level: Level::Error,
                message: format!(
                    "frontmatter line {} is not a key: value pair: {trimmed}",
                    i + 1
                ),
                file: Some(relative.clone()),
                line: Some((i + 2) as u32), // +2: 1-indexed + skip opening ---
                column: None,
            });
        }
    }

    if let Some(status) = frontmatter::frontmatter_scalar(fm, "status")
        && !ALLOWED_STATUSES.contains(&status)
    {
        diagnostics.push(Diagnostic {
            rule_id: "docs-lint/invalid-status".into(),
            level: Level::Error,
            message: format!(
                "invalid status \"{status}\" (allowed: {})",
                ALLOWED_STATUSES.join(", ")
            ),
            file: Some(relative.clone()),
            line: None,
            column: None,
        });
    }

    for violation in frontmatter::validate_provenance_contract(fm, &relative) {
        let message = match violation {
            ContractViolation::LegacySourceSha => {
                "frontmatter declares the deleted source_sha scalar".to_string()
            }
            ContractViolation::UndeclaredRegeneratedSource { key } => {
                format!("regenerated_from key \"{key}\" is not declared in sources")
            }
            ContractViolation::MalformedRegeneratedValue { key, value } => {
                format!("regenerated_from value for \"{key}\" is not a 40-char hex sha: {value}")
            }
        };
        diagnostics.push(Diagnostic {
            rule_id: "docs-lint/provenance-contract".into(),
            level: Level::Error,
            message,
            file: Some(relative.clone()),
            line: None,
            column: None,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn collect_diagnostics(path: &Path) -> Vec<Diagnostic> {
        let mut diags = Vec::new();
        lint_file_diagnostics(
            path,
            path.parent().unwrap_or_else(|| Path::new("/")),
            &mut diags,
        );
        diags
    }

    #[test]
    fn valid_frontmatter() {
        let content = "---\nstatus: done\ncompleted: \"2026-04-23\"\n---\n# Title\n";
        let fm = frontmatter::frontmatter_block(content).unwrap();
        assert_eq!(frontmatter::frontmatter_scalar(fm, "status"), Some("done"));
    }

    #[test]
    fn no_frontmatter_is_ok() {
        assert!(frontmatter::frontmatter_block("# Just a heading\n").is_none());
    }

    #[test]
    fn bad_status_rejected() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), "---\nstatus: wip\n---\n# Doc\n").unwrap();
        let diags = collect_diagnostics(tmp.path());
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("invalid status \"wip\""));
        assert_eq!(diags[0].rule_id, "docs-lint/invalid-status");
    }

    #[test]
    fn list_line_accepted() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(
            tmp.path(),
            "---\nstatus: done\ntags:\n  - alpha\n  - beta\n---\n# Doc\n",
        )
        .unwrap();
        let diags = collect_diagnostics(tmp.path());
        assert!(diags.is_empty());
    }

    #[test]
    fn malformed_line_rejected() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), "---\nnot a pair\n---\n").unwrap();
        let diags = collect_diagnostics(tmp.path());
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].rule_id, "docs-lint/malformed-frontmatter");
    }

    #[test]
    fn legacy_source_sha_rejected() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(
            tmp.path(),
            "---\nstatus: done\nsource_sha: deadbeef\n---\n# Doc\n",
        )
        .unwrap();
        let diags = collect_diagnostics(tmp.path());
        assert!(
            diags
                .iter()
                .any(|d| d.rule_id == "docs-lint/provenance-contract")
        );
    }

    #[test]
    fn diagnostics_produce_valid_sarif() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), "---\nstatus: wip\n---\n# Doc\n").unwrap();
        let diags = collect_diagnostics(tmp.path());
        let log = sarif::from_diagnostics("test", "0.1.0", &diags);
        let json = serde_json::to_string(&log).expect("serialize");
        assert!(json.contains("docs-lint/invalid-status"));
    }

    #[test]
    fn lint_docs_sarif_output() {
        let tmp = tempfile::TempDir::new().unwrap();
        let sarif_out = tmp.path().join("docs.sarif");
        // No docs/superpowers dir — zero files checked, no errors, but SARIF written.
        lint_docs(tmp.path(), Some(&sarif_out)).unwrap();
        assert!(sarif_out.exists());
        let content = std::fs::read_to_string(&sarif_out).unwrap();
        assert!(content.contains("minibox-docs-lint"));
    }
}
