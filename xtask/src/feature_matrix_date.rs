//! Automate `Last updated:` stamps across all docs in `docs/`.
//!
//! Rewrites every line matching `^Last updated: YYYY-MM-DD` to today's UTC date.
//! Idempotent: running it twice on the same day produces no diff.
//!
//! Run: `cargo xtask update-feature-matrix-date`
//!
//! Covered files (any `*.mbx.md` or `*.md` under `docs/` that contains a
//! `Last updated:` line):
//!
//! - docs/FEATURE_MATRIX.mbx.md
//! - docs/SECURITY_INVARIANTS.mbx.md
//! - docs/ROADMAP.mbx.md
//! - docs/STABILITY_CHECKLIST.mbx.md
//! - docs/STATE_MODEL.mbx.md
//! - docs/CRATE_TIERS.mbx.md
//! - docs/SUPPORT_TIERS.mbx.md
//! - docs/GOTCHAS.mbx.md
//! - … and any future docs that add a `Last updated:` stamp.

use anyhow::{Context, Result};
use chrono::Utc;
use std::{
    fs,
    path::{Path, PathBuf},
};

/// Update `Last updated:` stamps in all docs under `docs/`.
///
/// This is the primary entry point called by `cargo xtask update-feature-matrix-date`.
/// It replaces the older single-file behaviour with a workspace-wide pass.
pub fn update_feature_matrix_date(root: &Path) -> Result<()> {
    let today = Utc::now().format("%Y-%m-%d").to_string();
    let docs_dir = root.join("docs");

    let candidates = collect_doc_files(&docs_dir)
        .with_context(|| format!("failed to list docs dir: {}", docs_dir.display()))?;

    let mut updated_count = 0usize;
    let mut skipped_count = 0usize;

    for path in &candidates {
        match update_doc_date(path, &today)? {
            UpdateResult::Updated => {
                eprintln!("doc-dates: updated  {}", path.display());
                updated_count += 1;
            }
            UpdateResult::AlreadyCurrent => {
                skipped_count += 1;
            }
            UpdateResult::NoStamp => {}
        }
    }

    if updated_count == 0 && skipped_count == 0 {
        eprintln!("doc-dates: no files contain a 'Last updated:' stamp");
    } else if updated_count == 0 {
        eprintln!("doc-dates: all {skipped_count} stamped file(s) already up to date ({today})");
    } else {
        eprintln!(
            "doc-dates: updated {updated_count} file(s), {skipped_count} already current ({today})"
        );
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

enum UpdateResult {
    /// The file was rewritten with today's date.
    Updated,
    /// The file already had today's date — no write needed.
    AlreadyCurrent,
    /// The file contains no `Last updated:` stamp — left untouched.
    NoStamp,
}

/// Walk `docs_dir` and return all `*.md` files (recursive).
fn collect_doc_files(docs_dir: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    collect_recursive(docs_dir, &mut files)?;
    files.sort();
    Ok(files)
}

fn collect_recursive(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(dir).with_context(|| format!("read_dir failed: {}", dir.display()))? {
        let entry = entry.with_context(|| format!("dir entry error in {}", dir.display()))?;
        let path = entry.path();
        if path.is_dir() {
            collect_recursive(&path, out)?;
        } else if path.extension().map_or(false, |e| e == "md") {
            out.push(path);
        }
    }
    Ok(())
}

/// Rewrite the first `Last updated: YYYY-MM-DD` line in `path` to `today`.
fn update_doc_date(path: &Path, today: &str) -> Result<UpdateResult> {
    let content =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;

    if !content.contains("Last updated: ") {
        return Ok(UpdateResult::NoStamp);
    }

    let updated = rewrite_date(&content, today);

    if updated == content {
        return Ok(UpdateResult::AlreadyCurrent);
    }

    fs::write(path, updated.as_bytes())
        .with_context(|| format!("failed to write {}", path.display()))?;

    Ok(UpdateResult::Updated)
}

/// Replace the first `Last updated: YYYY-MM-DD` line with today's date.
///
/// Lines that do not match the prefix are left unchanged. Only the first
/// matching line is replaced so that embedded examples are not touched.
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

    #[test]
    fn no_stamp_returns_no_stamp_variant() {
        // File without a stamp should be left unchanged
        let input = "# Just a title\n\nSome content.\n";
        // rewrite_date never gets called for these files, but let's verify
        // that content without the prefix is passed through unchanged.
        let result = rewrite_date(input, "2026-05-14");
        assert_eq!(result, input);
    }
}
