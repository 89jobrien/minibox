use anyhow::{Context, Result, bail};
use std::path::Path;

/// Allowed values for the `status` frontmatter field in plans and specs.
const ALLOWED_STATUSES: &[&str] = &["open", "done", "archived", "approved", "draft"];

/// Lint all Markdown files under `docs/superpowers/{plans,specs}/`.
///
/// Checks:
/// 1. If YAML frontmatter delimiters (`---`) are present, every non-blank line
///    between them must be a valid `key: value` pair.
/// 2. If a `status` key exists, its value must be one of [`ALLOWED_STATUSES`].
pub fn lint_docs(root: &Path) -> Result<()> {
    let mut errors: Vec<String> = Vec::new();
    let mut checked = 0u32;

    for subdir in &["plans", "specs"] {
        let dir = root.join("docs/superpowers").join(subdir);
        if !dir.is_dir() {
            continue;
        }
        let entries =
            std::fs::read_dir(&dir).with_context(|| format!("read_dir {}", dir.display()))?;
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "md") {
                checked += 1;
                if let Err(e) = lint_file(&path) {
                    errors.push(format!("  {}: {e}", path.display()));
                }
            }
        }
    }

    eprintln!(
        "docs-lint: checked {checked} files, {n} error(s)",
        n = errors.len()
    );

    if errors.is_empty() {
        Ok(())
    } else {
        bail!("docs-lint failed:\n{}", errors.join("\n"));
    }
}

fn lint_file(path: &Path) -> Result<()> {
    let content =
        std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;

    let Some(fm) = extract_frontmatter(&content) else {
        return Ok(());
    };

    // Validate each non-blank line is a `key: value` pair.
    for (i, line) in fm.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !trimmed.contains(':') {
            bail!(
                "frontmatter line {} is not a key: value pair: {trimmed}",
                i + 1
            );
        }
    }

    // Extract and validate status.
    if let Some(status) = frontmatter_value(fm, "status")
        && !ALLOWED_STATUSES.contains(&status)
    {
        bail!(
            "invalid status \"{status}\" (allowed: {})",
            ALLOWED_STATUSES.join(", ")
        );
    }

    Ok(())
}

/// Extract the text between the opening and closing `---` delimiters.
fn extract_frontmatter(content: &str) -> Option<&str> {
    let trimmed = content.trim_start();
    let rest = trimmed.strip_prefix("---")?;
    let end = rest.find("\n---")?;
    Some(&rest[..end])
}

/// Get the value for a key from simple `key: value` frontmatter.
fn frontmatter_value<'a>(fm: &'a str, key: &str) -> Option<&'a str> {
    for line in fm.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix(key) {
            let rest = rest.trim_start();
            if let Some(val) = rest.strip_prefix(':') {
                let val = val.trim().trim_matches('"');
                if !val.is_empty() {
                    return Some(val);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_frontmatter() {
        let content = "---\nstatus: done\ncompleted: \"2026-04-23\"\n---\n# Title\n";
        let fm = extract_frontmatter(content).unwrap();
        assert_eq!(frontmatter_value(fm, "status"), Some("done"));
    }

    #[test]
    fn no_frontmatter_is_ok() {
        assert!(extract_frontmatter("# Just a heading\n").is_none());
    }

    #[test]
    fn bad_status_rejected() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), "---\nstatus: wip\n---\n# Doc\n").unwrap();
        let err = lint_file(tmp.path()).unwrap_err();
        assert!(err.to_string().contains("invalid status \"wip\""));
    }

    #[test]
    fn malformed_line_rejected() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), "---\nnot a pair\n---\n").unwrap();
        assert!(lint_file(tmp.path()).is_err());
    }
}
