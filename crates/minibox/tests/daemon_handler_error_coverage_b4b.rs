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
//! Handler error-path coverage — chain-b-4b (GH #116).
//!
//! Covers error branches in:
//! - `handle_get_manifest`: container not found, missing file, invalid JSON
//! - `handle_verify_manifest`: container not found, invalid policy JSON
//! - `handle_save_snapshot` / `handle_restore_snapshot` / `handle_list_snapshots`: NoopVmCheckpoint errors
//! - `handle_logs`: unknown container returns Error via channel
//! - `handle_pause` / `handle_resume`: container not found, wrong state

use minibox::daemon::handler;
use minibox::daemon::state::{ContainerRecord, DaemonState};
use minibox_core::protocol::{ContainerInfo, DaemonResponse};
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;

mod daemon_handler_common;
use daemon_handler_common::*;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_state(tmp: &TempDir) -> Arc<DaemonState> {
    let image_store =
        minibox::image::ImageStore::new(tmp.path().join("images")).expect("unwrap in test");
    Arc::new(DaemonState::new(image_store, tmp.path()))
}

fn make_record(id: &str, state_str: &str) -> ContainerRecord {
    ContainerRecord {
        info: ContainerInfo {
            id: id.to_string(),
            name: None,
            image: "alpine:latest".to_string(),
            command: "/bin/sh".to_string(),
            state: state_str.to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
            pid: None,
        },
        pid: None,
        rootfs_path: PathBuf::from("/tmp/fake-rootfs"),
        cgroup_path: PathBuf::from("/sys/fs/cgroup/fake"),
        post_exit_hooks: vec![],
        rootfs_metadata: None,
        source_image_ref: None,
        step_state: None,
        priority: None,
        urgency: None,
        execution_context: None,
        creation_params: None,
        manifest_path: None,
        workload_digest: None,
    }
}

// ---------------------------------------------------------------------------
// handle_get_manifest — container not found
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_handle_get_manifest_container_not_found() {
    let tmp = TempDir::new().expect("unwrap in test");
    let state = make_state(&tmp);
    let deps = create_test_deps_with_dir(&tmp);
    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);

    handler::handle_get_manifest("nonexistent-id-b4b".to_string(), state, deps, tx).await;

    let resp = rx.recv().await.expect("no response");
    assert!(
        matches!(resp, DaemonResponse::Error { ref message } if message.contains("container not found")),
        "expected container not found error, got {resp:?}"
    );
}

// ---------------------------------------------------------------------------
// handle_get_manifest — manifest file missing (container exists, no file)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_handle_get_manifest_file_missing() {
    let tmp = TempDir::new().expect("unwrap in test");
    let state = make_state(&tmp);
    let deps = create_test_deps_with_dir(&tmp);

    let id = "manifest-missing-b4b";
    state.add_container(make_record(id, "Stopped")).await;

    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);
    handler::handle_get_manifest(id.to_string(), state, deps, tx).await;

    let resp = rx.recv().await.expect("no response");
    assert!(
        matches!(resp, DaemonResponse::Error { ref message } if message.contains("failed to read manifest")),
        "expected missing file error, got {resp:?}"
    );
}

// ---------------------------------------------------------------------------
// handle_get_manifest — invalid JSON in manifest file
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_handle_get_manifest_invalid_json() {
    let tmp = TempDir::new().expect("unwrap in test");
    let state = make_state(&tmp);
    let deps = create_test_deps_with_dir(&tmp);

    let id = "manifest-badjson-b4b";
    state.add_container(make_record(id, "Stopped")).await;

    // Write an invalid JSON file where the handler expects to find it.
    let manifest_dir = tmp.path().join("containers").join(id);
    std::fs::create_dir_all(&manifest_dir).expect("unwrap in test");
    std::fs::write(manifest_dir.join("execution-manifest.json"), b"not json {{")
        .expect("unwrap in test");

    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);
    handler::handle_get_manifest(id.to_string(), state, deps, tx).await;

    let resp = rx.recv().await.expect("no response");
    assert!(
        matches!(resp, DaemonResponse::Error { ref message } if message.contains("failed to parse manifest")),
        "expected JSON parse error, got {resp:?}"
    );
}

// ---------------------------------------------------------------------------
// handle_verify_manifest — container not found
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_handle_verify_manifest_container_not_found() {
    let tmp = TempDir::new().expect("unwrap in test");
    let state = make_state(&tmp);
    let deps = create_test_deps_with_dir(&tmp);
    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);

    handler::handle_verify_manifest(
        "no-such-container-b4b".to_string(),
        "{}".to_string(),
        state,
        deps,
        tx,
    )
    .await;

    let resp = rx.recv().await.expect("no response");
    assert!(
        matches!(resp, DaemonResponse::Error { ref message } if message.contains("container not found")),
        "expected container not found error, got {resp:?}"
    );
}

// ---------------------------------------------------------------------------
// handle_verify_manifest — manifest file missing (container exists, no file)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_handle_verify_manifest_manifest_file_missing() {
    let tmp = TempDir::new().expect("unwrap in test");
    let state = make_state(&tmp);
    let deps = create_test_deps_with_dir(&tmp);

    let id = "verify-nomanifest-b4b";
    state.add_container(make_record(id, "Stopped")).await;
    // No manifest file written — read step must fail.

    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);
    handler::handle_verify_manifest(id.to_string(), "{}".to_string(), state, deps, tx).await;

    let resp = rx.recv().await.expect("no response");
    assert!(
        matches!(resp, DaemonResponse::Error { ref message } if message.contains("failed to read manifest")),
        "expected missing manifest file error, got {resp:?}"
    );
}

// ---------------------------------------------------------------------------
// handle_verify_manifest — invalid policy JSON
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_handle_verify_manifest_invalid_policy_json() {
    let tmp = TempDir::new().expect("unwrap in test");
    let state = make_state(&tmp);
    let deps = create_test_deps_with_dir(&tmp);

    let id = "verify-badpolicy-b4b";
    state.add_container(make_record(id, "Stopped")).await;

    // Write a valid manifest so the manifest-read and parse steps pass.
    let manifest_dir = tmp.path().join("containers").join(id);
    std::fs::create_dir_all(&manifest_dir).expect("unwrap in test");
    // Minimal valid ExecutionManifest JSON matching the schema.
    let manifest_json = serde_json::json!({
        "schema_version": 1,
        "container_id": id,
        "created_at": "2026-01-01T00:00:00Z",
        "subject": {
            "image_ref": "alpine:latest",
            "image": {
                "manifest_digest": null,
                "config_digest": null,
                "layer_digests": []
            }
        },
        "runtime": {
            "command": ["/bin/sh"],
            "env": [],
            "mounts": [],
            "resource_limits": null,
            "network_mode": "none",
            "privileged": false,
            "platform": null
        },
        "request": {
            "name": null,
            "ephemeral": false
        }
    });
    std::fs::write(
        manifest_dir.join("execution-manifest.json"),
        manifest_json.to_string().as_bytes(),
    )
    .expect("unwrap in test");

    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);
    handler::handle_verify_manifest(
        id.to_string(),
        "not valid json {{{{".to_string(),
        state,
        deps,
        tx,
    )
    .await;

    let resp = rx.recv().await.expect("no response");
    assert!(
        matches!(resp, DaemonResponse::Error { ref message } if message.contains("failed to parse policy")),
        "expected policy parse error, got {resp:?}"
    );
}

// ---------------------------------------------------------------------------
// handle_save_snapshot — NoopVmCheckpoint always errors
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_handle_save_snapshot_noop_checkpoint_returns_error() {
    let tmp = TempDir::new().expect("unwrap in test");
    let state = make_state(&tmp);
    let deps = create_test_deps_with_dir(&tmp);

    let resp = handler::handle_save_snapshot(
        "some-container-b4b".to_string(),
        Some("snap-1".to_string()),
        state,
        deps,
    )
    .await;

    assert!(
        matches!(resp, DaemonResponse::Error { ref message } if message.contains("not supported")),
        "expected checkpoint not supported error, got {resp:?}"
    );
}

// ---------------------------------------------------------------------------
// handle_restore_snapshot — NoopVmCheckpoint always errors
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_handle_restore_snapshot_noop_checkpoint_returns_error() {
    let tmp = TempDir::new().expect("unwrap in test");
    let state = make_state(&tmp);
    let deps = create_test_deps_with_dir(&tmp);

    let resp = handler::handle_restore_snapshot(
        "some-container-b4b".to_string(),
        "snap-1".to_string(),
        state,
        deps,
    )
    .await;

    assert!(
        matches!(resp, DaemonResponse::Error { ref message } if message.contains("not supported")),
        "expected checkpoint not supported error, got {resp:?}"
    );
}

// ---------------------------------------------------------------------------
// handle_list_snapshots — NoopVmCheckpoint always errors
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_handle_list_snapshots_noop_checkpoint_returns_error() {
    let tmp = TempDir::new().expect("unwrap in test");
    let deps = create_test_deps_with_dir(&tmp);

    let resp = handler::handle_list_snapshots("some-container-b4b".to_string(), deps).await;

    assert!(
        matches!(resp, DaemonResponse::Error { ref message } if message.contains("not supported")),
        "expected checkpoint not supported error, got {resp:?}"
    );
}

// ---------------------------------------------------------------------------
// handle_logs — unknown container sends Error via channel
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_handle_logs_unknown_container_sends_error() {
    let tmp = TempDir::new().expect("unwrap in test");
    let state = make_state(&tmp);
    let deps = create_test_deps_with_dir(&tmp);
    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);

    handler::handle_logs("no-such-container-b4b".to_string(), false, state, deps, tx).await;

    let resp = rx.recv().await.expect("no response");
    assert!(
        matches!(resp, DaemonResponse::Error { ref message } if message.contains("container not found")),
        "expected container not found error, got {resp:?}"
    );
}

// ---------------------------------------------------------------------------
// handle_pause — container not found
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_handle_pause_unknown_container_returns_error() {
    let tmp = TempDir::new().expect("unwrap in test");
    let state = make_state(&tmp);
    let event_sink = Arc::new(minibox_core::events::NoopEventSink);

    let resp = handler::handle_pause("no-such-b4b".to_string(), state, event_sink).await;

    assert!(
        matches!(resp, DaemonResponse::Error { ref message } if message.contains("not found")),
        "expected not found error, got {resp:?}"
    );
}

// ---------------------------------------------------------------------------
// handle_resume — container not found
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_handle_resume_unknown_container_returns_error() {
    let tmp = TempDir::new().expect("unwrap in test");
    let state = make_state(&tmp);
    let event_sink = Arc::new(minibox_core::events::NoopEventSink);

    let resp = handler::handle_resume("no-such-b4b".to_string(), state, event_sink).await;

    assert!(
        matches!(resp, DaemonResponse::Error { ref message } if message.contains("not found")),
        "expected not found error, got {resp:?}"
    );
}

// ---------------------------------------------------------------------------
// handle_resume — container exists but is not paused (wrong state)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_handle_resume_running_container_wrong_state_returns_error() {
    let tmp = TempDir::new().expect("unwrap in test");
    let state = make_state(&tmp);
    let event_sink = Arc::new(minibox_core::events::NoopEventSink);

    let id = "resume-wrong-state-b4b";
    state.add_container(make_record(id, "Running")).await;

    let resp = handler::handle_resume(id.to_string(), state, event_sink).await;

    assert!(
        matches!(resp, DaemonResponse::Error { ref message } if message.contains("not paused")),
        "expected wrong state error, got {resp:?}"
    );
}
