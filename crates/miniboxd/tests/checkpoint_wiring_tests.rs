#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::doc_markdown,
    clippy::redundant_field_names,
    clippy::uninlined_format_args,
    clippy::redundant_clone,
    clippy::redundant_closure,
    clippy::single_char_pattern,
    clippy::unwrap_in_result,
    clippy::collapsible_if,
    clippy::match_same_arms,
    clippy::only_used_in_recursion,
    clippy::used_underscore_binding,
    clippy::map_unwrap_or,
    clippy::manual_assert,
    clippy::as_ptr_cast_mut,
    clippy::ptr_as_ptr,
    clippy::must_use_candidate,
    clippy::used_underscore_items,
    clippy::missing_const_for_fn,
    clippy::manual_string_new,
    clippy::semicolon_if_nothing_returned,
    clippy::unreadable_literal,
    clippy::default_constructed_unit_structs,
    clippy::ref_as_ptr,
    clippy::allow_attributes_without_reason,
    clippy::redundant_closure_for_method_calls,
    clippy::needless_raw_string_hashes,
    clippy::manual_is_variant_and,
    clippy::ignore_without_reason,
    clippy::default_trait_access,
    clippy::cast_lossless,
    clippy::match_wild_err_arm,
    clippy::format_push_string,
    clippy::bool_assert_comparison,
    clippy::struct_excessive_bools
)]
//! Checkpoint wiring conformance for the `miniboxd` composition root.
//!
//! No adapter in this workspace implements
//! [`VmCheckpoint`](minibox_core::domain::VmCheckpoint) and none declares
//! `BackendCapability::Checkpoint`, so production wiring deliberately installs
//! `NoopVmCheckpoint`. That is the truthful state: the port exists, the
//! protocol surface exists, and the daemon answers snapshot requests with an
//! explicit "not supported" error instead of fabricating state.
//!
//! These tests pin three things:
//!
//! 1. **Production wiring stays honest.** Every `checkpoint:` assignment in
//!    `main.rs` is the no-op, is documented as such at the wiring site, and no
//!    longer carries a stale TODO. A future adapter that genuinely implements
//!    the port may replace the no-op — but then it must say why, here.
//! 2. **The no-op is observable.** `save`/`restore`/`list` each surface an
//!    error, `list` never claims "zero snapshots" (which would be a claim
//!    about the world rather than an admission of ignorance), and no snapshot
//!    file is ever written to disk.
//! 3. **The port is a real seam.** Swapping in an in-memory trait double makes
//!    save/restore/list succeed and round-trip, so the production failure is
//!    attributable to the absence of an adapter — not to hardcoded breakage in
//!    the daemon.
//!
//! No daemon process, no root, no network.

use std::sync::Arc;

use minibox::daemon::handler::{
    HandlerDependencies, handle_list_snapshots, handle_restore_snapshot, handle_save_snapshot,
};
use minibox::testing::helpers::{make_mock_deps, make_mock_state};
use minibox::testing::mocks::MockVmCheckpoint;
use minibox_core::domain::NoopVmCheckpoint;
use minibox_core::protocol::DaemonResponse;
use tempfile::TempDir;

/// `main.rs` is the composition root; its source is the only place production
/// adapter wiring is assembled (the builders are private to the binary).
const MAIN_RS: &str = include_str!("../src/main.rs");

/// The production no-op expression every wiring site must use.
const NOOP_WIRING: &str = "checkpoint: Arc::new(minibox_core::domain::NoopVmCheckpoint),";

/// Indices of the `checkpoint:` wiring lines in `main.rs`.
fn checkpoint_wiring_lines() -> Vec<usize> {
    MAIN_RS
        .lines()
        .enumerate()
        .filter(|(_, line)| line.trim_start().starts_with("checkpoint:"))
        .map(|(idx, _)| idx)
        .collect()
}

// ---------------------------------------------------------------------------
// 1. Production wiring stays honest
// ---------------------------------------------------------------------------

/// The `feature-idea-12` TODO claimed an adapter-specific implementation was
/// still owed. Investigation (issue #517) found no `VmCheckpoint` impl outside
/// the no-op and the test double, and no `BackendCapability::Checkpoint` in any
/// adapter, so the TODO was inaccurate: nothing is owed, the no-op *is* the
/// answer. It must not linger.
#[test]
fn production_wiring_drops_stale_checkpoint_todo() {
    assert!(
        !MAIN_RS.contains("TODO(feature-idea-12)"),
        "the feature-idea-12 checkpoint TODO is resolved: no adapter implements \
         VmCheckpoint, so there is no implementation owed. Remove the TODO."
    );
}

/// Each wiring site carries a comment naming the port, so a reader landing on
/// any single adapter builder sees why the no-op is deliberate rather than
/// assuming an oversight.
#[test]
fn every_production_checkpoint_wiring_is_documented_in_place() {
    let lines: Vec<&str> = MAIN_RS.lines().collect();
    let wiring = checkpoint_wiring_lines();
    assert!(
        !wiring.is_empty(),
        "expected at least one production checkpoint wiring in main.rs"
    );

    for &idx in &wiring {
        let documented = lines[idx.saturating_sub(4)..idx].iter().any(|line| {
            let trimmed = line.trim();
            trimmed.starts_with("//") && trimmed.contains("VmCheckpoint")
        });
        assert!(
            documented,
            "checkpoint wiring at main.rs:{} must be preceded by a comment naming \
             VmCheckpoint, so the no-op reads as deliberate: {}",
            idx + 1,
            lines[idx].trim()
        );
    }
}

/// Guards against a silently-wired fake. Any `checkpoint:` assignment that is
/// not the no-op means an adapter claims to persist state — that must be a
/// deliberate, reviewed change, not an accident.
#[test]
fn every_production_checkpoint_wiring_is_the_honest_noop() {
    let wiring = checkpoint_wiring_lines();
    assert!(
        !wiring.is_empty(),
        "expected at least one production checkpoint wiring in main.rs"
    );

    for idx in wiring {
        let line = MAIN_RS.lines().nth(idx).expect("index in range");
        assert_eq!(
            line.trim(),
            NOOP_WIRING,
            "main.rs:{} wires a non-noop VmCheckpoint. No adapter implements the \
             port yet; a replacement must be a real snapshot implementation, \
             not a stub that claims to persist state.",
            idx + 1
        );
    }
}

// ---------------------------------------------------------------------------
// 2. The no-op is observable: it errors, and it lies about nothing
// ---------------------------------------------------------------------------

/// The production no-op, exercised through the protocol handler the CLI calls.
#[tokio::test]
async fn save_snapshot_reports_unsupported_and_writes_no_snapshot() {
    let tmp = TempDir::new().expect("create temp dir");
    let state = make_mock_state(tmp.path());
    let deps = make_mock_deps(&tmp);

    let response = handle_save_snapshot(
        "ctr-1".to_string(),
        Some("before-upgrade".to_string()),
        state,
        Arc::clone(&deps),
    )
    .await;

    let DaemonResponse::Error { message } = &response else {
        panic!("unsupported checkpoint must return Error, got {response:?}");
    };
    assert!(
        message.contains("not supported"),
        "error must say checkpointing is unsupported, got: {message}"
    );

    // Nothing may be persisted: an on-disk artefact would imply a snapshot
    // exists even though the request failed.
    let snap_dir = tmp.path().join("snapshots").join("ctr-1");
    let snap_file = snap_dir.join("before-upgrade.snap");
    assert!(
        !snap_file.exists(),
        "a failed save must not create {}, got a file where none should exist",
        snap_file.display()
    );
    let written: Vec<String> = std::fs::read_dir(&snap_dir)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    assert!(
        written.is_empty(),
        "a failed save must not leave snapshot files behind, found: {written:?}"
    );
}

#[tokio::test]
async fn restore_snapshot_reports_unsupported() {
    let tmp = TempDir::new().expect("create temp dir");
    let state = make_mock_state(tmp.path());
    let deps = make_mock_deps(&tmp);

    let response = handle_restore_snapshot(
        "ctr-1".to_string(),
        "before-upgrade".to_string(),
        state,
        deps,
    )
    .await;

    let DaemonResponse::Error { message } = &response else {
        panic!("unsupported checkpoint must return Error, got {response:?}");
    };
    assert!(
        message.contains("not supported"),
        "error must say checkpointing is unsupported, got: {message}"
    );
}

/// The honesty-critical case. `Ok(vec![])` would render as "No snapshots for
/// container ctr-1" — a claim that the daemon inspected real state and found
/// none. It did not; it cannot look. An error says "I don't know", which is
/// the truth, and it makes `mbx snapshot list` exit non-zero.
#[tokio::test]
async fn list_snapshots_errors_instead_of_claiming_no_snapshots_exist() {
    let tmp = TempDir::new().expect("create temp dir");
    let deps = make_mock_deps(&tmp);

    let response = handle_list_snapshots("ctr-1".to_string(), deps).await;

    assert!(
        !matches!(response, DaemonResponse::SnapshotList { .. }),
        "an unsupported checkpoint must not answer with a snapshot list, got {response:?}"
    );
    let DaemonResponse::Error { message } = &response else {
        panic!("unsupported checkpoint must return Error, got {response:?}");
    };
    assert!(
        message.contains("not supported"),
        "error must say checkpointing is unsupported, got: {message}"
    );
}

/// Errors must name the failing operation so a user can tell which of the
/// three snapshot calls was rejected.
#[tokio::test]
async fn unsupported_snapshot_errors_name_the_operation() {
    let tmp = TempDir::new().expect("create temp dir");
    let state = make_mock_state(tmp.path());
    let deps = make_mock_deps(&tmp);

    let save = handle_save_snapshot(
        "ctr-1".to_string(),
        None,
        Arc::clone(&state),
        Arc::clone(&deps),
    )
    .await;
    let restore =
        handle_restore_snapshot("ctr-1".to_string(), "snap".to_string(), state, deps).await;
    let list = handle_list_snapshots("ctr-1".to_string(), make_mock_deps(&tmp)).await;

    for (op, response) in [
        ("save_snapshot", save),
        ("restore_snapshot", restore),
        ("list_snapshots", list),
    ] {
        let DaemonResponse::Error { message } = &response else {
            panic!("{op} must return Error, got {response:?}");
        };
        assert!(
            message.starts_with(op),
            "{op} error must start with the operation name, got: {message}"
        );
    }
}

// ---------------------------------------------------------------------------
// 3. The port is a real seam, not hardcoded breakage
// ---------------------------------------------------------------------------

/// Swap the no-op for an in-memory trait double and the same handler succeeds.
/// This is the evidence that the daemon's snapshot path is generic over
/// `DynVmCheckpoint`: production fails only because no adapter implements it.
#[tokio::test]
async fn save_snapshot_succeeds_when_adapter_provides_an_implementation() {
    let tmp = TempDir::new().expect("create temp dir");
    let state = make_mock_state(tmp.path());
    let base = make_mock_deps(&tmp);
    let deps: Arc<HandlerDependencies> = Arc::new(HandlerDependencies {
        checkpoint: Arc::new(MockVmCheckpoint::new()),
        ..(*base).clone()
    });

    let response = handle_save_snapshot(
        "ctr-1".to_string(),
        Some("before-upgrade".to_string()),
        state,
        deps,
    )
    .await;

    let DaemonResponse::SnapshotSaved { info } = &response else {
        panic!("a wired implementation must return SnapshotSaved, got {response:?}");
    };
    assert_eq!(info.container_id, "ctr-1");
}

/// Save → restore round-trip against the in-memory double: the protocol
/// surface is complete and correct once an adapter exists, so wiring one in
/// later is a drop-in change with no handler edits.
#[tokio::test]
async fn save_restore_round_trips_through_an_in_memory_checkpoint_adapter() {
    let tmp = TempDir::new().expect("create temp dir");
    let state = make_mock_state(tmp.path());
    let base = make_mock_deps(&tmp);
    let deps: Arc<HandlerDependencies> = Arc::new(HandlerDependencies {
        checkpoint: Arc::new(MockVmCheckpoint::new()),
        ..(*base).clone()
    });

    let saved = handle_save_snapshot(
        "ctr-1".to_string(),
        Some("before-upgrade".to_string()),
        Arc::clone(&state),
        Arc::clone(&deps),
    )
    .await;
    assert!(
        matches!(saved, DaemonResponse::SnapshotSaved { .. }),
        "save must succeed with a wired implementation, got {saved:?}"
    );

    let listed = handle_list_snapshots("ctr-1".to_string(), Arc::clone(&deps)).await;
    let DaemonResponse::SnapshotList { snapshots, .. } = &listed else {
        panic!("list must succeed with a wired implementation, got {listed:?}");
    };
    assert_eq!(
        snapshots.len(),
        1,
        "saved snapshot must be listed: {snapshots:?}"
    );

    let restored = handle_restore_snapshot(
        "ctr-1".to_string(),
        "before-upgrade".to_string(),
        state,
        deps,
    )
    .await;
    let DaemonResponse::SnapshotRestored { id, name } = &restored else {
        panic!("restore must succeed for a listed snapshot, got {restored:?}");
    };
    assert_eq!(id, "ctr-1");
    assert_eq!(name, "before-upgrade");
}

// ---------------------------------------------------------------------------
// 4. The no-op itself, at the port boundary
// ---------------------------------------------------------------------------

/// `NoopVmCheckpoint` coerces to `DynVmCheckpoint` and rejects all three
/// operations. This is the exact type every production wiring site installs,
/// so this pins the behaviour those sites depend on.
#[test]
fn noop_checkpoint_rejects_every_operation() {
    let noop: minibox_core::domain::DynVmCheckpoint = Arc::new(NoopVmCheckpoint);
    let path = std::path::Path::new("/nonexistent/snapshot.snap");

    for message in [
        noop.save_snapshot("ctr-1", path).err(),
        noop.restore_snapshot("ctr-1", path).err(),
        noop.list_snapshots("ctr-1").err(),
    ] {
        let message = message.expect("no-op must reject, never fabricate success");
        assert!(
            message.to_string().contains("not supported"),
            "rejection must say 'not supported', got: {message}"
        );
    }
}
