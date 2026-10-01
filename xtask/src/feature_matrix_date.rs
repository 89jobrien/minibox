//! Automate `Last updated:` stamps across all docs in `docs/`.
//!
//! Rewrites the first line matching `^Last updated: YYYY-MM-DD` to today's UTC
//! date. Idempotent: running it twice on the same day produces no diff.
//!
//! Run: `cargo xtask update-date`
//!
//! Two entry points with deliberately different scope:
//!
//! - [`update_feature_matrix_date`] sweeps **every** stamped doc under `docs/`.
//!   It is the explicit manual command, so a blanket rewrite matches intent.
//! - [`update_staged_docs`] touches only the specific docs handed to it. The
//!   pre-commit gate uses this so a commit bumps the stamp of the docs it
//!   actually changes and leaves every other doc alone.
//!
//! Covered files (any `*.mbx.md` or `*.md` under `docs/` that contains a
//! `Last updated:` line):
//!
//! - `docs/FEATURE_MATRIX.mbx.md`
//! - `docs/SECURITY_INVARIANTS.mbx.md`
//! - `docs/ROADMAP.mbx.md`
//! - `docs/STABILITY_CHECKLIST.mbx.md`
//! - `docs/STATE_MODEL.mbx.md`
//! - `docs/CRATE_TIERS.mbx.md`
//! - `docs/SUPPORT_TIERS.mbx.md`
//! - `docs/GOTCHAS.mbx.md`
//! - … and any future docs that add a `Last updated:` stamp.

use anyhow::{Context, Result};
use chrono::Utc;
use std::{
    fs,
    path::{Path, PathBuf},
};

/// Line prefix identifying a doc date stamp.
const STAMP_PREFIX: &str = "Last updated: ";

/// Tally of what a stamp pass did, for reporting.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct StampOutcome {
    /// Files whose stamp line was rewritten.
    pub updated: usize,
    /// Files already carrying today's stamp.
    pub already_current: usize,
    /// Files with no stamp line.
    pub no_stamp: usize,
}

impl StampOutcome {
    /// True when no file carried a stamp at all.
    const fn is_empty(&self) -> bool {
        self.updated == 0 && self.already_current == 0 && self.no_stamp == 0
    }
}

/// Update `Last updated:` stamps in all docs under `docs/`.
///
/// This is the blanket sweep behind `cargo xtask update-date`. Because it is an
/// explicit operator action, touching every stamped doc is the intended
/// behaviour. Automated callers should prefer [`update_staged_docs`].
pub fn update_feature_matrix_date(root: &Path) -> Result<()> {
    let today = Utc::now().format("%Y-%m-%d").to_string();
    let docs_dir = root.join("docs");

    let candidates = collect_doc_files(&docs_dir)
        .with_context(|| format!("failed to list docs dir: {}", docs_dir.display()))?;

    let outcome = update_dates_in(&candidates, &today);
    report(outcome, &today, false);
    Ok(())
}

/// Update `Last updated:` stamps in only the given docs.
///
/// The pre-commit gate calls this with the staged doc paths (repo-relative, as
/// reported by `git diff --cached`) so that committing one doc does not
/// restamp the whole `docs/` tree. Paths are used verbatim; callers are
/// responsible for filtering to docs that genuinely changed.
pub fn update_staged_docs(paths: &[PathBuf]) -> StampOutcome {
    let today = Utc::now().format("%Y-%m-%d").to_string();
    let outcome = update_dates_in(paths, &today);
    report(outcome, &today, true);
    outcome
}

/// Apply today's stamp to each path, tallying the result.
fn update_dates_in(paths: &[PathBuf], today: &str) -> StampOutcome {
    let mut outcome = StampOutcome::default();
    for path in paths {
        match update_doc_date(path, today) {
            Ok(UpdateResult::Updated) => {
                eprintln!("doc-dates: updated  {}", path.display());
                outcome.updated += 1;
            }
            Ok(UpdateResult::AlreadyCurrent) => outcome.already_current += 1,
            Ok(UpdateResult::NoStamp) => outcome.no_stamp += 1,
            Err(error) => {
                // A single unreadable doc must not abort the gate; report and
                // continue so the rest of the tree still gets stamped.
                eprintln!("doc-dates: SKIPPED {}: {error:#}", path.display());
                outcome.no_stamp += 1;
            }
        }
    }
    outcome
}

/// Print a one-line summary of a stamp pass.
fn report(outcome: StampOutcome, today: &str, scoped: bool) {
    let scope = if scoped { "staged docs" } else { "docs tree" };
    if outcome.is_empty() {
        eprintln!("doc-dates: no files contain a 'Last updated:' stamp ({scope})");
    } else if outcome.updated == 0 {
        eprintln!(
            "doc-dates: all {scope} already up to date ({today}, {} stamped)",
            outcome.already_current
        );
    } else {
        eprintln!(
            "doc-dates: updated {scope}: {} file(s) rewritten, {} already current ({today})",
            outcome.updated, outcome.already_current
        );
    }
}

/// Whether `line` is a doc date stamp line.
#[must_use]
pub fn is_stamp_line(line: &str) -> bool {
    line.trim_start().starts_with(STAMP_PREFIX)
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
        } else if path.extension().is_some_and(|e| e == "md") {
            out.push(path);
        }
    }
    Ok(())
}

/// Rewrite the first `Last updated: YYYY-MM-DD` line in `path` to `today`.
fn update_doc_date(path: &Path, today: &str) -> Result<UpdateResult> {
    let content =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;

    if !content.contains(STAMP_PREFIX) {
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
    let prefix = STAMP_PREFIX;
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

    // ── stamp-line detection ─────────────────────────────────────────────

    #[test]
    fn is_stamp_line_recognises_the_stamp() {
        assert!(is_stamp_line("Last updated: 2026-01-01"));
        assert!(is_stamp_line("  Last updated: 2026-01-01"));
    }

    #[test]
    fn is_stamp_line_rejects_other_content() {
        assert!(!is_stamp_line("Last update: 2026-01-01"));
        assert!(!is_stamp_line("## Last updated"));
        assert!(!is_stamp_line("The last updated doc is 2026-01-01"));
        assert!(!is_stamp_line("+added a real line"));
    }

    #[test]
    fn is_stamp_line_expects_diff_marker_already_stripped() {
        // The caller strips a leading +/- before asking. A surviving marker
        // means the line is not the stamp, so it counts as a real change.
        assert!(!is_stamp_line("-Last updated: 2026-01-01"));
        assert!(!is_stamp_line("+Last updated: 2026-01-01"));
    }

    // ── scoped (commit-aware) updates ────────────────────────────────────

    #[test]
    fn update_staged_docs_only_touches_given_paths() {
        let dir = tempfile::tempdir().expect("tempdir");
        let staged = dir.path().join("STAGED.md");
        let untouched = dir.path().join("UNTOUCHED.md");
        for p in [&staged, &untouched] {
            fs::write(p, "Last updated: 2020-01-01\nbody\n").expect("write");
        }

        let outcome = update_staged_docs(std::slice::from_ref(&staged));
        assert_eq!(outcome.updated, 1);

        let stamped = fs::read_to_string(&staged).expect("read");
        assert_ne!(stamped, "Last updated: 2020-01-01\nbody\n");

        // The path that was not passed in must be byte-identical.
        let other = fs::read_to_string(&untouched).expect("read");
        assert_eq!(
            other, "Last updated: 2020-01-01\nbody\n",
            "a doc outside the staged set must never be restamped"
        );
    }

    #[test]
    fn update_staged_docs_reports_already_current() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("DOC.md");
        let today = Utc::now().format("%Y-%m-%d").to_string();
        fs::write(&path, format!("Last updated: {today}\nbody\n")).expect("write");

        let outcome = update_staged_docs(&[path]);
        assert_eq!(outcome.updated, 0);
        assert_eq!(outcome.already_current, 1);
    }

    #[test]
    fn update_staged_docs_tolerates_a_missing_file() {
        // A doc deleted between staging and the gate must not abort the gate.
        let dir = tempfile::tempdir().expect("tempdir");
        let missing = dir.path().join("GONE.md");
        let outcome = update_staged_docs(&[missing]);
        assert_eq!(outcome.updated, 0);
        assert_eq!(outcome.no_stamp, 1);
    }

    #[test]
    fn update_staged_docs_with_empty_list_is_a_no_op() {
        let outcome = update_staged_docs(&[]);
        assert!(outcome.is_empty());
    }
}
