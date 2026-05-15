//! `cargo xtask collect-metrics` — aggregate project health metrics.
//!
//! Counts source lines, test annotations, and workspace crate count from the
//! live source tree. Reads the `Last updated` date from the feature matrix doc.
//! Outputs a compact JSON object to stdout, optionally saving to
//! `metrics/latest.json` when `--save` is passed.

use anyhow::{Context, Result};
use serde::Serialize;
use std::path::Path;

// ─── Output schema ────────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct MetricsSnapshot {
    /// ISO-8601 UTC timestamp when collection ran.
    collected_at: String,
    /// Number of workspace member crates.
    crate_count: usize,
    /// Lines matching `#[test]` or `#[tokio::test]` across all `.rs` files.
    test_count: usize,
    /// Total non-empty lines across all `.rs` files under workspace crates.
    source_lines: usize,
    /// Value of the `Last updated:` field in `docs/FEATURE_MATRIX.mbx.md`,
    /// or `"unknown"` if the file is absent or the field is missing.
    feature_matrix_date: String,
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn now_utc() -> String {
    // Avoid adding a time crate dep; delegate to `date`.
    std::process::Command::new("date")
        .args(["-u", "+%Y-%m-%dT%H:%M:%SZ"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

/// Parse workspace member count from `cargo metadata`.
fn count_crates(root: &Path) -> Result<usize> {
    let output = std::process::Command::new("cargo")
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .current_dir(root)
        .output()
        .context("run cargo metadata")?;

    let raw =
        String::from_utf8(output.stdout).context("cargo metadata output is not valid UTF-8")?;
    let meta: serde_json::Value = serde_json::from_str(&raw).context("parse cargo metadata")?;
    let count = meta["workspace_members"]
        .as_array()
        .map(|a| a.len())
        .unwrap_or(0);
    Ok(count)
}

/// Walk all `.rs` files under `dir`, accumulating total non-empty source lines
/// and lines that contain a test annotation.
fn scan_rs_files(dir: &Path, lines: &mut usize, tests: &mut usize) -> Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir)
        .with_context(|| format!("read_dir {}", dir.display()))?
        .flatten()
    {
        let path = entry.path();
        let ft = entry.file_type().context("file type")?;
        if ft.is_dir() {
            // Skip build artifacts to avoid double-counting.
            let name = entry.file_name();
            if name == "target" {
                continue;
            }
            scan_rs_files(&path, lines, tests)?;
        } else if ft.is_file() && path.extension().is_some_and(|e| e == "rs") {
            let content = std::fs::read_to_string(&path)
                .with_context(|| format!("read {}", path.display()))?;
            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                *lines += 1;
                if trimmed == "#[test]" || trimmed == "#[tokio::test]" {
                    *tests += 1;
                }
            }
        }
    }
    Ok(())
}

/// Read the `Last updated:` date from `docs/FEATURE_MATRIX.mbx.md`.
fn feature_matrix_date(root: &Path) -> String {
    let path = root.join("docs/FEATURE_MATRIX.mbx.md");
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return "unknown".to_string(),
    };
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("Last updated:") {
            return rest.trim().to_string();
        }
    }
    "unknown".to_string()
}

// ─── Entry point ──────────────────────────────────────────────────────────────

pub fn collect_metrics(root: &Path, save: bool) -> Result<()> {
    let collected_at = now_utc();
    let crate_count = count_crates(root).context("count workspace crates")?;
    let feature_matrix_date = feature_matrix_date(root);

    let mut source_lines = 0usize;
    let mut test_count = 0usize;
    scan_rs_files(root, &mut source_lines, &mut test_count)
        .context("scan source files")?;

    let snapshot = MetricsSnapshot {
        collected_at,
        crate_count,
        test_count,
        source_lines,
        feature_matrix_date,
    };

    let json = serde_json::to_string_pretty(&snapshot).context("serialize metrics")?;

    if save {
        let dir = root.join("metrics");
        std::fs::create_dir_all(&dir).context("create metrics/")?;
        let dest = dir.join("latest.json");
        std::fs::write(&dest, &json)
            .with_context(|| format!("write {}", dest.display()))?;
        eprintln!("Metrics saved to {}", dest.display());
    } else {
        println!("{json}");
    }

    Ok(())
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;

    fn make_tmp() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "collect_metrics_test_{}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("create tmp dir");
        dir
    }

    #[test]
    fn scan_counts_test_annotations() {
        let dir = make_tmp().join("scan_test");
        fs::create_dir_all(&dir).expect("create dir");
        let rs = dir.join("lib.rs");
        let mut f = fs::File::create(&rs).expect("create file");
        writeln!(f, "fn foo() {{}}").expect("write");
        writeln!(f, "#[test]").expect("write");
        writeln!(f, "fn bar() {{}}").expect("write");
        writeln!(f, "#[tokio::test]").expect("write");
        writeln!(f, "async fn baz() {{}}").expect("write");

        let mut lines = 0;
        let mut tests = 0;
        scan_rs_files(&dir, &mut lines, &mut tests).expect("scan");
        assert_eq!(tests, 2, "should count both #[test] and #[tokio::test]");
        assert_eq!(lines, 5, "five non-empty lines");

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn feature_matrix_date_missing_returns_unknown() {
        let dir = make_tmp().join("no_docs");
        fs::create_dir_all(&dir).expect("create dir");
        assert_eq!(feature_matrix_date(&dir), "unknown");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn feature_matrix_date_parsed_correctly() {
        let dir = make_tmp().join("docs_test");
        let docs = dir.join("docs");
        fs::create_dir_all(&docs).expect("create docs dir");
        fs::write(
            docs.join("FEATURE_MATRIX.mbx.md"),
            "# Feature Matrix\n\nLast updated: 2026-05-08\n",
        )
        .expect("write matrix");
        assert_eq!(feature_matrix_date(&dir), "2026-05-08");
        fs::remove_dir_all(&dir).ok();
    }
}
