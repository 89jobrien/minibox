//! Automate the `Last updated:` stamp in `docs/FEATURE_MATRIX.mbx.md`.
//!
//! Rewrites the first line matching `^Last updated: YYYY-MM-DD` to today's UTC date.
//! Idempotent: running it twice on the same day produces no diff.
//!
//! Run: `cargo xtask update-feature-matrix-date`

use anyhow::{Context, Result};
use chrono::Utc;
use std::{
    fs,
    path::Path,
};

const FEATURE_MATRIX: &str = "docs/FEATURE_MATRIX.mbx.md";

/// Rewrite the `Last updated:` line in FEATURE_MATRIX.mbx.md to today's UTC date.
pub fn update_feature_matrix_date(root: &Path) -> Result<()> {
    let path = root.join(FEATURE_MATRIX);
    let content = fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;

    let today = Utc::now().format("%Y-%m-%d").to_string();
    let updated = rewrite_date(&content, &today);

    if updated == content {
        eprintln!("feature-matrix-date: already up to date ({today})");
        return Ok(());
    }

    fs::write(&path, updated.as_bytes())
        .with_context(|| format!("failed to write {}", path.display()))?;

    eprintln!("feature-matrix-date: updated Last updated to {today}");
    Ok(())
}

/// Replace the first `Last updated: YYYY-MM-DD` line with today's date.
///
/// Lines that do not match the prefix are left unchanged. The replacement is
/// applied only to the first matching line so that embedded examples in the
/// file body are not touched.
fn rewrite_date(content: &str, today: &str) -> String {
    let prefix = "Last updated: ";
    let mut replaced = false;
    content
        .lines()
        .map(|line| {
            if !replaced && line.starts_with(prefix) {
                replaced = true;
                format!("{prefix}{today}")
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        + if content.ends_with('\n') { "\n" } else { "" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_date_line() {
        let input = "# Title\nLast updated: 2024-01-01\nsome content\n";
        let result = rewrite_date(input, "2026-05-14");
        assert_eq!(result, "# Title\nLast updated: 2026-05-14\nsome content\n");
    }

    #[test]
    fn idempotent_when_already_current() {
        let input = "Last updated: 2026-05-14\ncontent\n";
        let result = rewrite_date(input, "2026-05-14");
        assert_eq!(result, input);
    }

    #[test]
    fn replaces_only_first_match() {
        let input = "Last updated: 2024-01-01\ntext\nLast updated: 2024-01-01\n";
        let result = rewrite_date(input, "2026-05-14");
        assert_eq!(
            result,
            "Last updated: 2026-05-14\ntext\nLast updated: 2024-01-01\n"
        );
    }

    #[test]
    fn preserves_no_trailing_newline() {
        let input = "Last updated: 2024-01-01";
        let result = rewrite_date(input, "2026-05-14");
        assert_eq!(result, "Last updated: 2026-05-14");
    }
}
