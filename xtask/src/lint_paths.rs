//! Advisory lint for path-handling anti-patterns in Rust source files.
//!
//! Scans all `.rs` files under `crates/` for constructs that may indicate
//! missing path validation or security issues. Findings are printed to stdout
//! and the command always exits 0 — this is informational, not a hard gate.
//!
//! Run: `cargo xtask lint-paths`

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// A single detected finding.
struct Finding {
    file: PathBuf,
    line: usize,
    pattern: &'static str,
    description: &'static str,
}

/// Patterns to detect and their descriptions.
///
/// Each entry is `(needle, pattern_label, description)`.
const PATTERNS: &[(&str, &str, &str)] = &[
    (
        ".join(",
        "Path::join",
        "path join — verify argument is not user-controlled without prior validate_layer_path()",
    ),
    (
        "fs::read_to_string(",
        "fs::read_to_string",
        "direct fs read — ensure path was canonicalized or validated before this call",
    ),
    (
        "fs::write(",
        "fs::write",
        "direct fs write — ensure path was canonicalized or validated before this call",
    ),
    (
        "fs::remove_file(",
        "fs::remove_file",
        "direct fs remove — ensure path was canonicalized or validated before this call",
    ),
    (
        "env::args()",
        "env::args",
        "env args piped to path ops — verify args are not used directly in path construction",
    ),
    (
        "format!(",
        "format! path",
        "format! macro — if used to construct a path string, prefer Path::join with validation",
    ),
];

/// Lines that suppress a finding when they also contain these strings.
///
/// These indicate the validation is already present on the same line.
const SUPPRESSORS: &[&str] = &["validate_layer_path", "canonicalize", "// lint-paths: ok"];

/// Scan all `.rs` files under `crates/` for path-handling anti-patterns.
pub fn run(root: &Path) -> Result<()> {
    let crates_dir = root.join("crates");
    let mut files_checked: usize = 0;
    let mut findings: Vec<Finding> = Vec::new();

    let mut queue: Vec<PathBuf> = vec![crates_dir.clone()];

    while let Some(dir) = queue.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                queue.push(path);
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            files_checked += 1;
            let content = std::fs::read_to_string(&path)
                .with_context(|| format!("read {}", path.display()))?;

            scan_file(&path, &content, &mut findings);
        }
    }

    for f in &findings {
        let rel = f.file.strip_prefix(root).unwrap_or(&f.file);
        println!(
            "{}:{}: {} — {}",
            rel.display(),
            f.line,
            f.pattern,
            f.description
        );
    }

    println!(
        "lint-paths: {} file(s) checked, {} finding(s)",
        files_checked,
        findings.len()
    );

    Ok(())
}

/// Scan a single file's content for pattern matches.
fn scan_file(path: &Path, content: &str, findings: &mut Vec<Finding>) {
    for (lineno, line) in content.lines().enumerate() {
        // Skip lines that already show a validation call (same-line suppression).
        if SUPPRESSORS.iter().any(|s| line.contains(s)) {
            continue;
        }
        // Skip comments.
        let trimmed = line.trim_start();
        if trimmed.starts_with("//") {
            continue;
        }

        for (needle, label, desc) in PATTERNS {
            // For `.join(`, skip if the argument is a string literal (low risk).
            if *needle == ".join(" && is_join_with_literal(line) {
                continue;
            }
            if line.contains(needle) {
                findings.push(Finding {
                    file: path.to_path_buf(),
                    line: lineno + 1,
                    pattern: label,
                    description: desc,
                });
            }
        }
    }
}

/// Returns true if the `.join(` call appears to take a string literal argument.
///
/// Example: `.join("subdir")` — considered low risk since the path component
/// is a compile-time constant, not user-controlled data.
fn is_join_with_literal(line: &str) -> bool {
    if let Some(after) = line.split(".join(").nth(1) {
        let arg = after.trim_start();
        // String literal: starts with `"` or `r"`
        arg.starts_with('"') || arg.starts_with("r\"") || arg.starts_with("r#")
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect(source: &str) -> Vec<String> {
        let mut findings = Vec::new();
        scan_file(Path::new("test.rs"), source, &mut findings);
        findings
            .iter()
            .map(|f| format!("{}:{}", f.line, f.pattern))
            .collect()
    }

    #[test]
    fn detects_fs_read_without_validation() {
        let src = r#"
fn load(p: &Path) {
    let _ = fs::read_to_string(p);
}
"#;
        let hits = collect(src);
        assert!(
            hits.iter().any(|h| h.contains("fs::read_to_string")),
            "expected fs::read_to_string finding, got: {hits:?}"
        );
    }

    #[test]
    fn clean_source_has_no_findings() {
        // Same-line suppressor: validate_layer_path on the same line suppresses the finding.
        let src = r#"
fn nothing() {}
fn also_nothing(x: i32) -> i32 { x + 1 }
"#;
        let hits = collect(src);
        assert!(hits.is_empty(), "expected no findings, got: {hits:?}");
    }

    #[test]
    fn same_line_suppressor_works() {
        // When validate_layer_path appears on the same line as fs::read_to_string, no finding.
        let src = "    let c = fs::read_to_string(validate_layer_path(p)?);\n";
        let hits = collect(src);
        assert!(
            hits.is_empty(),
            "suppressor on same line should clear finding, got: {hits:?}"
        );
    }

    #[test]
    fn join_with_literal_not_flagged() {
        let src = r#"
fn build_path(base: &Path) -> PathBuf {
    base.join("subdir").join("file.txt")
}
"#;
        let hits = collect(src);
        let join_hits: Vec<_> = hits.iter().filter(|h| h.contains("Path::join")).collect();
        assert!(
            join_hits.is_empty(),
            "literal join should not be flagged, got: {join_hits:?}"
        );
    }

    #[test]
    fn join_with_variable_is_flagged() {
        let src = r#"
fn build(base: &Path, name: &str) -> PathBuf {
    base.join(name)
}
"#;
        let hits = collect(src);
        assert!(
            hits.iter().any(|h| h.contains("Path::join")),
            "variable join should be flagged, got: {hits:?}"
        );
    }

    #[test]
    fn comment_lines_skipped() {
        let src = r#"
// fs::read_to_string(user_input)
// .join(var)
fn nothing() {}
"#;
        let hits = collect(src);
        assert!(
            hits.is_empty(),
            "comment lines should not produce findings, got: {hits:?}"
        );
    }
}
