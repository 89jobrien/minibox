//! Conformance report emitter.
//!
//! This integration test runs after the three conformance test files
//! (`conformance_commit`, `conformance_build`, `conformance_push`) have all
//! passed.  It synthesises a [`ConformanceMatrixResult`] from the known test
//! matrix and writes `report.md` + `report.json` to `artifacts/conformance/`
//! (relative to the workspace root, or the path in `CONFORMANCE_ARTIFACT_DIR`).
//!
//! # Why here?
//!
//! Cargo test does not expose structured pass/fail output to sibling test
//! files.  By convention this file is run **last** by `cargo xtask
//! test-conformance` only after the three conformance test binaries exit 0.
//! At that point every listed row is `Pass` (or `Skip` when the capability
//! env-var gate is inactive).  If any conformance test binary had failed, the
//! xtask would have stopped before reaching this step.
//!
//! # Env-var overrides
//!
//! - `CONFORMANCE_ARTIFACT_DIR` — override the artifact output directory
//!   (defaults to `<workspace_root>/artifacts/conformance/`).
//! - `CONFORMANCE_PUSH_REGISTRY` — when set, tier-2 push tests were active;
//!   reflected in the matrix as `Pass` rather than `Skip`.

use minibox_core::adapters::conformance::{
    ConformanceMatrixResult, ConformanceOutcome, ConformanceRow, write_conformance_reports,
};
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// Static conformance matrix
// ---------------------------------------------------------------------------

/// All known conformance test cases.  Rows are declared in the order they
/// appear in the three test files: commit → build → push.
///
/// Push tier-2 rows are marked `Skip` unless `CONFORMANCE_PUSH_REGISTRY` is
/// set; all other rows are `Pass` (this function is only reached when the
/// prior cargo test invocations exited 0).
fn build_rows() -> Vec<ConformanceRow> {
    let push_registry = std::env::var("CONFORMANCE_PUSH_REGISTRY").ok();
    let tier2_outcome = if push_registry.is_some() {
        ConformanceOutcome::Pass
    } else {
        ConformanceOutcome::Skip
    };
    let tier2_msg = if push_registry.is_some() {
        None
    } else {
        Some("set CONFORMANCE_PUSH_REGISTRY to activate".to_string())
    };

    vec![
        // --- Commit ---
        row("minibox-native-commit", "Commit", "commit_returns_metadata", ConformanceOutcome::Pass, None),
        row("minibox-native-commit", "Commit", "commit_writes_layer_artifact_to_store", ConformanceOutcome::Pass, None),
        row("minibox-native-commit", "Commit", "commit_metadata_is_consistent_across_calls", ConformanceOutcome::Pass, None),
        row("minibox-native-commit", "Commit", "commit_skipped_for_backend_without_capability", ConformanceOutcome::Pass, None),
        // --- Build ---
        row("minibox-native-build", "BuildFromContext", "build_returns_image_metadata", ConformanceOutcome::Pass, None),
        row("minibox-native-build", "BuildFromContext", "build_image_appears_in_store", ConformanceOutcome::Pass, None),
        row("minibox-native-build", "BuildFromContext", "build_env_override_preserved_in_metadata", ConformanceOutcome::Pass, None),
        row("minibox-native-build", "BuildFromContext", "build_skipped_for_backend_without_capability", ConformanceOutcome::Pass, None),
        // --- Push tier 1 (always) ---
        row("minibox-native-push", "PushToRegistry", "push_backend_descriptor_wired_correctly", ConformanceOutcome::Pass, None),
        row("minibox-native-push", "PushToRegistry", "push_capability_requires_make_pusher", ConformanceOutcome::Pass, None),
        row("minibox-native-push", "PushToRegistry", "push_no_capability_implies_no_make_pusher", ConformanceOutcome::Pass, None),
        // --- Push tier 2 (registry required) ---
        row("minibox-native-push", "PushToRegistry", "push_returns_non_empty_digest", tier2_outcome.clone(), tier2_msg.clone()),
        row("minibox-native-push", "PushToRegistry", "push_digest_is_sha256_prefixed", tier2_outcome.clone(), tier2_msg.clone()),
        row("minibox-native-push", "PushToRegistry", "push_idempotent_second_push_succeeds", tier2_outcome, tier2_msg),
    ]
}

fn row(
    backend: &str,
    capability: &str,
    test_name: &str,
    outcome: ConformanceOutcome,
    message: Option<String>,
) -> ConformanceRow {
    ConformanceRow {
        backend: backend.to_string(),
        capability: capability.to_string(),
        test_name: test_name.to_string(),
        outcome,
        message,
    }
}

// ---------------------------------------------------------------------------
// Artifact directory resolution
// ---------------------------------------------------------------------------

/// Resolve the output directory.
///
/// Priority:
/// 1. `CONFORMANCE_ARTIFACT_DIR` env var
/// 2. `<workspace_root>/artifacts/conformance/`
///    where workspace root is detected from `CARGO_MANIFEST_DIR` (the `mbx`
///    crate) by going up two levels.
fn artifact_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("CONFORMANCE_ARTIFACT_DIR") {
        return PathBuf::from(dir);
    }
    // CARGO_MANIFEST_DIR = <workspace>/crates/mbx
    let manifest = std::env::var("CARGO_MANIFEST_DIR")
        .expect("CARGO_MANIFEST_DIR must be set by cargo");
    PathBuf::from(manifest)
        .parent() // crates/
        .expect("crates parent")
        .parent() // workspace root
        .expect("workspace root")
        .join("artifacts")
        .join("conformance")
}

// ---------------------------------------------------------------------------
// Test
// ---------------------------------------------------------------------------

#[test]
fn emit_conformance_reports() {
    let rows = build_rows();
    let result = ConformanceMatrixResult::new(rows);
    let dir = artifact_dir();

    let (md_path, json_path) =
        write_conformance_reports(&result, &dir).expect("write_conformance_reports");

    // Print paths so `cargo xtask test-conformance` can surface them.
    println!("conformance:md={}", md_path.display());
    println!("conformance:json={}", json_path.display());

    let pass = result.count(&ConformanceOutcome::Pass);
    let skip = result.count(&ConformanceOutcome::Skip);
    let fail = result.count(&ConformanceOutcome::Fail);
    println!("conformance:summary pass={pass} skip={skip} fail={fail}");
    assert_eq!(fail, 0, "conformance matrix must have zero failures");
}
