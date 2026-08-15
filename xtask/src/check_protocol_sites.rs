//! `cargo xtask check-protocol-sites` — scan the codebase for `DaemonRequest`
//! and `DaemonResponse` variant usage and report any variants that appear in
//! zero files outside `protocol.rs` itself (dead protocol surface).
//!
//! ## Algorithm
//!
//! 1. Read `crates/minibox-core/src/protocol.rs` and extract all variant names
//!    from both enums using a simple line-by-line heuristic (no AST parsing).
//! 2. Walk all `.rs` files under `crates/`, skipping `protocol.rs` itself.
//! 3. For each variant name, count how many files contain the bare name as a
//!    substring (sufficient for `DaemonRequest::Foo`, `DaemonResponse::Foo`,
//!    match arm `Foo {`, etc.).
//! 4. Report variants with a file count of zero as "dead protocol surface".
//!    Return `Ok(())` when all variants are referenced; bail with the list
//!    otherwise.

use anyhow::{Context, Result, bail};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Run the protocol-sites check from the workspace root.
pub fn run(root: &Path) -> Result<()> {
    let protocol_path = root.join("crates/minibox-core/src/protocol.rs");

    let proto_src = fs::read_to_string(&protocol_path).with_context(|| {
        format!(
            "check-protocol-sites: failed to read {}",
            protocol_path.display()
        )
    })?;

    // Extract variant names from DaemonRequest and DaemonResponse.
    let request_variants = extract_variants(&proto_src, "DaemonRequest");
    let response_variants = extract_variants(&proto_src, "DaemonResponse");

    let all_variants: Vec<String> = request_variants
        .into_iter()
        .chain(response_variants)
        .collect();

    if all_variants.is_empty() {
        bail!("check-protocol-sites: no variants found in protocol.rs — check parsing logic");
    }

    // Walk all .rs files under crates/, excluding protocol.rs itself.
    let crates_dir = root.join("crates");
    let rs_files = collect_rs_files(&crates_dir, &protocol_path)?;

    // Count references per variant.
    let counts = count_variant_references(&all_variants, &rs_files)?;

    let total = all_variants.len();
    let dead: Vec<&str> = all_variants
        .iter()
        .filter(|v| counts.get(v.as_str()).copied().unwrap_or(0) == 0)
        .map(String::as_str)
        .collect();

    println!(
        "check-protocol-sites: {total} variants checked, {} dead variants found",
        dead.len()
    );

    if dead.is_empty() {
        println!("check-protocol-sites: OK — all variants referenced outside protocol.rs");
        Ok(())
    } else {
        for v in &dead {
            eprintln!("  dead variant: {v}");
        }
        bail!(
            "check-protocol-sites: {} dead variant(s) found: {}",
            dead.len(),
            dead.join(", ")
        )
    }
}

// ---------------------------------------------------------------------------
// Variant extraction
// ---------------------------------------------------------------------------

/// Extract enum variant names from `src` that belong to the enum named
/// `enum_name`.  Uses a simple state machine: enter the enum on a line that
/// matches `pub enum <name>` and exit on an unindented `}`.
///
/// Variant lines look like (after trimming):
///   - `Foo,`
///   - `Foo {`
///   - `Foo(T),`
pub fn extract_variants(src: &str, enum_name: &str) -> Vec<String> {
    let marker = format!("pub enum {enum_name}");
    let mut inside = false;
    let mut depth: u32 = 0;
    let mut variants = Vec::new();

    for line in src.lines() {
        let trimmed = line.trim();

        if !inside {
            if trimmed.contains(&marker) {
                inside = true;
                // Count braces on the marker line itself. The enum opens with
                // `pub enum Foo {` so depth becomes 1 after this line.
                for ch in trimmed.chars() {
                    match ch {
                        '{' => depth += 1,
                        '}' => depth = depth.saturating_sub(1),
                        _ => {}
                    }
                }
            }
            continue;
        }

        // Capture variant name BEFORE updating depth for this line so we can
        // test depth == 1 (direct enum body, not nested struct fields).
        let depth_before = depth;

        // Update depth for this line.
        let mut exited = false;
        for ch in trimmed.chars() {
            match ch {
                '{' => depth += 1,
                '}' => {
                    if depth == 0 {
                        // Closing brace of the enum itself — done.
                        inside = false;
                        exited = true;
                        break;
                    }
                    depth -= 1;
                }
                _ => {}
            }
        }

        // Only capture variants at depth_before == 1 (direct enum body).
        if depth_before == 1 && !exited {
            // Skip doc comments and attributes.
            if trimmed.starts_with("///") || trimmed.starts_with('#') || trimmed.is_empty() {
                continue;
            }
            // Extract the variant name: first word, strip trailing `{`, `(`, `,`.
            if let Some(name) = trimmed
                .split_whitespace()
                .next()
                .map(|w| w.trim_end_matches(['{', '(', ',']))
            {
                // Variant names are PascalCase; skip keywords and noise.
                let first = name.chars().next();
                if first.is_some_and(char::is_uppercase) {
                    variants.push(name.to_string());
                }
            }
        }

        if !inside {
            break;
        }
    }

    variants
}

// ---------------------------------------------------------------------------
// File walking
// ---------------------------------------------------------------------------

/// Recursively collect all `.rs` files under `dir`, excluding `skip`.
pub fn collect_rs_files(dir: &Path, skip: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    collect_rs_files_inner(dir, skip, &mut files)?;
    Ok(files)
}

fn collect_rs_files_inner(dir: &Path, skip: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let entries = fs::read_dir(dir)
        .with_context(|| format!("check-protocol-sites: failed to read dir {}", dir.display()))?;

    for entry in entries {
        let entry = entry.with_context(|| {
            format!("check-protocol-sites: dir entry error in {}", dir.display())
        })?;
        let path = entry.path();

        if path.is_dir() {
            collect_rs_files_inner(&path, skip, out)?;
        } else if path.extension().is_some_and(|e| e == "rs") && path != skip {
            out.push(path);
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Reference counting
// ---------------------------------------------------------------------------

/// For each variant name, count how many files in `files` contain that name as
/// a substring.
pub fn count_variant_references(
    variants: &[String],
    files: &[PathBuf],
) -> Result<BTreeMap<String, usize>> {
    let mut counts: BTreeMap<String, usize> =
        variants.iter().map(|v| (v.clone(), 0usize)).collect();

    for path in files {
        let content = fs::read_to_string(path)
            .with_context(|| format!("check-protocol-sites: failed to read {}", path.display()))?;

        for variant in variants {
            if content.contains(variant.as_str()) {
                *counts.entry(variant.clone()).or_insert(0) += 1;
            }
        }
    }

    Ok(counts)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;
    use tempfile::TempDir;

    // --- extract_variants ---

    #[test]
    fn extract_variants_basic() {
        let src = r#"
pub enum DaemonRequest {
    Run { image: String },
    Stop { id: String },
    List,
    Pull { image: String },
}
"#;
        let variants = extract_variants(src, "DaemonRequest");
        assert!(
            variants.contains(&"Run".to_string()),
            "expected Run: {variants:?}"
        );
        assert!(
            variants.contains(&"Stop".to_string()),
            "expected Stop: {variants:?}"
        );
        assert!(
            variants.contains(&"List".to_string()),
            "expected List: {variants:?}"
        );
        assert!(
            variants.contains(&"Pull".to_string()),
            "expected Pull: {variants:?}"
        );
    }

    #[test]
    fn extract_variants_skips_other_enums() {
        let src = r#"
pub enum Other {
    Alpha,
    Beta,
}

pub enum DaemonResponse {
    Success { message: String },
    Error { message: String },
}
"#;
        let variants = extract_variants(src, "DaemonResponse");
        assert!(
            variants.contains(&"Success".to_string()),
            "expected Success: {variants:?}"
        );
        assert!(
            variants.contains(&"Error".to_string()),
            "expected Error: {variants:?}"
        );
        assert!(
            !variants.contains(&"Alpha".to_string()),
            "should not include Other enum: {variants:?}"
        );
    }

    #[test]
    fn extract_variants_empty_when_missing() {
        let src = "pub struct Foo { x: u32 }";
        let variants = extract_variants(src, "DaemonRequest");
        assert!(variants.is_empty(), "expected empty: {variants:?}");
    }

    // --- count_variant_references ---

    fn write_file(dir: &TempDir, name: &str, content: &str) -> PathBuf {
        let path = dir.path().join(name);
        let mut f = std::fs::File::create(&path).expect("create temp file");
        write!(f, "{content}").expect("write temp file");
        path
    }

    #[test]
    fn dead_variant_detection() {
        let dir = TempDir::new().expect("tempdir");
        let f1 = write_file(&dir, "a.rs", "DaemonRequest::Run { image: foo }");
        let f2 = write_file(&dir, "b.rs", "let x = DaemonRequest::Stop { id };");

        let variants = vec!["Run".to_string(), "Stop".to_string(), "Orphan".to_string()];
        let counts = count_variant_references(&variants, &[f1, f2]).expect("count");

        assert_eq!(counts["Run"], 1, "Run should appear in one file");
        assert_eq!(counts["Stop"], 1, "Stop should appear in one file");
        assert_eq!(counts["Orphan"], 0, "Orphan should be dead");
    }

    #[test]
    fn all_variants_present_returns_ok() {
        let dir = TempDir::new().expect("tempdir");
        let f = write_file(
            &dir,
            "handler.rs",
            "match req { DaemonRequest::Run => run(), DaemonRequest::Stop => stop() }",
        );

        let variants = vec!["Run".to_string(), "Stop".to_string()];
        let counts = count_variant_references(&variants, &[f]).expect("count");
        let dead: Vec<_> = variants.iter().filter(|v| counts[*v] == 0).collect();
        assert!(
            dead.is_empty(),
            "all present, should have no dead: {dead:?}"
        );
    }

    #[test]
    fn variant_in_multiple_files_counted_per_file() {
        let dir = TempDir::new().expect("tempdir");
        let f1 = write_file(&dir, "x.rs", "Run and Run again");
        let f2 = write_file(&dir, "y.rs", "also Run here");

        let variants = vec!["Run".to_string()];
        let counts = count_variant_references(&variants, &[f1, f2]).expect("count");
        // Each file containing Run increments by 1 (file count, not occurrence count).
        assert_eq!(counts["Run"], 2, "Run present in 2 files");
    }
}
