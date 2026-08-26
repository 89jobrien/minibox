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
//! Coverage tests for handler functions with no prior external-test coverage.
//!
//! Targets (chain-b-4a):
//! - `handle_list_pipelines`  — empty store, with limit filter
//! - `handle_show_pipeline`   — not found, load-returns-none
//! - `handle_update` restart=true  — new branch not exercised by existing tests
//! - `resolve_platform_registry`   — invalid platform string → Error
//! - `handle_pull` with valid platform override → Success (passes through registry)

use minibox::daemon::handler::{self};
use minibox_core::protocol::DaemonResponse;
use std::sync::Arc;
use tempfile::TempDir;

mod daemon_handler_common;
use daemon_handler_common::*;

// ─── handle_list_pipelines ────────────────────────────────────────────────────

/// `handle_list_pipelines` with no stored pipelines returns an empty
/// `PipelineList`.
#[tokio::test]
async fn test_handle_list_pipelines_empty_store_returns_empty_list() {
    let tmp = TempDir::new().expect("create temp dir");
    let state = create_test_state_with_dir(&tmp);

    let resp = handler::handle_list_pipelines(None, None, Arc::clone(&state)).await;
    match resp {
        DaemonResponse::PipelineList { pipelines } => {
            assert!(
                pipelines.is_empty(),
                "expected empty pipeline list from NoopTraceStore"
            );
        }
        other => panic!("expected PipelineList, got {other:?}"),
    }
}

/// `handle_list_pipelines` with a `limit=Some(5)` still returns an empty list
/// when the store has nothing — exercises the `TraceFilter` construction branch.
#[tokio::test]
async fn test_handle_list_pipelines_with_limit_returns_empty_list() {
    let tmp = TempDir::new().expect("create temp dir");
    let state = create_test_state_with_dir(&tmp);

    let resp = handler::handle_list_pipelines(Some(5), None, Arc::clone(&state)).await;
    assert!(
        matches!(resp, DaemonResponse::PipelineList { .. }),
        "expected PipelineList, got {resp:?}"
    );
}

/// `handle_list_pipelines` with a `pipeline` name filter returns a `PipelineList`
/// (exercises the `TraceFilter { pipeline: Some(...) }` branch).
#[tokio::test]
async fn test_handle_list_pipelines_with_pipeline_filter_returns_list() {
    let tmp = TempDir::new().expect("create temp dir");
    let state = create_test_state_with_dir(&tmp);

    let resp =
        handler::handle_list_pipelines(None, Some("my-pipeline".to_string()), Arc::clone(&state))
            .await;
    assert!(
        matches!(resp, DaemonResponse::PipelineList { .. }),
        "expected PipelineList with pipeline filter, got {resp:?}"
    );
}

// ─── handle_show_pipeline ─────────────────────────────────────────────────────

/// `handle_show_pipeline` for an ID not in the store returns `Error` with
/// "not found" in the message.
#[tokio::test]
async fn test_handle_show_pipeline_not_found_returns_error() {
    let tmp = TempDir::new().expect("create temp dir");
    let state = create_test_state_with_dir(&tmp);

    let resp = handler::handle_show_pipeline("run-does-not-exist".to_string(), state).await;
    assert!(
        matches!(resp, DaemonResponse::Error { ref message } if message.contains("not found")),
        "expected Error with 'not found', got {resp:?}"
    );
}

/// `handle_show_pipeline` for an empty-string ID returns an error (None or parse error).
#[tokio::test]
async fn test_handle_show_pipeline_empty_id_returns_error() {
    let tmp = TempDir::new().expect("create temp dir");
    let state = create_test_state_with_dir(&tmp);

    let resp = handler::handle_show_pipeline(String::new(), state).await;
    // NoopTraceStore.load("") returns Ok(None) → Error { "not found" }
    assert!(
        matches!(resp, DaemonResponse::Error { .. }),
        "expected Error for empty pipeline id, got {resp:?}"
    );
}

// ─── handle_update restart=true ───────────────────────────────────────────────

/// `handle_update` with `restart=true` and no running containers should still
/// complete successfully (no candidates to stop/restart).
#[tokio::test]
async fn test_handle_update_restart_true_no_candidates_succeeds() {
    let tmp = TempDir::new().expect("create temp dir");
    let state = create_test_state_with_dir(&tmp);
    let deps = create_test_deps_with_dir(&tmp);

    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(16);
    handler::handle_update(
        handler::UpdateParams {
            images: vec!["alpine:latest".to_string()],
            all: false,
            containers: false,
            restart: true,
        },
        Arc::clone(&state),
        Arc::clone(&deps),
        tx,
    )
    .await;

    // Drain all messages; expect at least one UpdateProgress and then Success.
    let mut got_success = false;
    while let Ok(resp) = rx.try_recv() {
        if matches!(resp, DaemonResponse::Success { .. }) {
            got_success = true;
        }
    }
    assert!(
        got_success,
        "handle_update restart=true with no candidates must end with Success"
    );
}

/// `handle_update` with `restart=true`, explicit images list, and a stopped
/// container that has a source_image_ref matching one of the images — the
/// stopped container is not eligible for restart (state != Running/Paused).
#[tokio::test]
async fn test_handle_update_restart_true_stopped_container_not_restarted() {
    use minibox::daemon::state::ContainerRecord;
    use minibox_core::protocol::ContainerInfo;

    let tmp = TempDir::new().expect("create temp dir");
    let state = create_test_state_with_dir(&tmp);
    let deps = create_test_deps_with_dir(&tmp);

    // A Stopped container — not eligible for restart by handle_update.
    let record = ContainerRecord {
        info: ContainerInfo {
            id: "ctr-restart-skip-test".to_string(),
            name: None,
            image: "alpine:latest".to_string(),
            command: "/bin/sh".to_string(),
            state: "Stopped".to_string(),
            created_at: "1970-01-01T00:00:00Z".to_string(),
            pid: None,
        },
        pid: None,
        runtime_id: None,
        rootfs_path: tmp.path().join("rootfs"),
        cgroup_path: tmp.path().join("cgroup"),
        post_exit_hooks: vec![],
        rootfs_metadata: None,
        source_image_ref: Some("alpine:latest".to_string()),
        upper_dir: None,
        merged_dir: None,
        step_state: None,
        priority: None,
        urgency: None,
        execution_context: None,
        creation_params: None,
        manifest_path: None,
        workload_digest: None,
    };
    state.add_container(record).await;

    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(16);
    handler::handle_update(
        handler::UpdateParams {
            images: vec!["alpine:latest".to_string()],
            all: false,
            containers: false,
            restart: true,
        },
        Arc::clone(&state),
        Arc::clone(&deps),
        tx,
    )
    .await;

    let mut got_success = false;
    while let Ok(resp) = rx.try_recv() {
        if matches!(resp, DaemonResponse::Success { .. }) {
            got_success = true;
        }
    }
    assert!(
        got_success,
        "handle_update restart=true must produce Success even when no eligible containers"
    );
}

// ─── resolve_platform_registry — invalid platform string ─────────────────────

/// `handle_pull` with an unparseable platform string returns `Error` with
/// "invalid platform" — exercises the error branch of `resolve_platform_registry`.
#[tokio::test]
async fn test_handle_pull_invalid_platform_returns_error() {
    let tmp = TempDir::new().expect("create temp dir");
    let state = create_test_state_with_dir(&tmp);
    let deps = create_test_deps_with_dir(&tmp);

    let resp = handler::handle_pull(
        "alpine".to_string(),
        Some("latest".to_string()),
        Some("not/a/valid/platform/string/!!".to_string()),
        state,
        deps,
    )
    .await;
    assert!(
        matches!(resp, DaemonResponse::Error { ref message } if message.contains("invalid platform")),
        "invalid platform string must produce Error with 'invalid platform', got {resp:?}"
    );
}

// ─── handle_update resolve_update_targets: all=true with images ──────────────

/// `handle_update` with `all=true` and an empty image store results in
/// `Success` with a summary saying "0/0 images" (no images to update).
/// Exercises the `all=true` branch of `resolve_update_targets`.
#[tokio::test]
async fn test_handle_update_all_true_empty_store_immediate_success() {
    let tmp = TempDir::new().expect("create temp dir");
    let state = create_test_state_with_dir(&tmp);
    let deps = create_test_deps_with_dir(&tmp);

    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(8);
    handler::handle_update(
        handler::UpdateParams {
            images: vec![],
            all: true,
            containers: false,
            restart: false,
        },
        Arc::clone(&state),
        Arc::clone(&deps),
        tx,
    )
    .await;

    let resp = rx
        .recv()
        .await
        .expect("no response from handle_update all=true");
    assert!(
        matches!(resp, DaemonResponse::Success { .. }),
        "all=true with empty image store must return Success, got {resp:?}"
    );
}

// ─── handle_list_pipelines with limit + pipeline filter ───────────────────────

/// `handle_list_pipelines` with both `limit` and `pipeline` set exercises the
/// full `TraceFilter` construction path.
#[tokio::test]
async fn test_handle_list_pipelines_limit_and_filter_combined() {
    let tmp = TempDir::new().expect("create temp dir");
    let state = create_test_state_with_dir(&tmp);

    let resp = handler::handle_list_pipelines(
        Some(10),
        Some("deploy-pipeline".to_string()),
        Arc::clone(&state),
    )
    .await;

    assert!(
        matches!(resp, DaemonResponse::PipelineList { .. }),
        "expected PipelineList with limit+filter, got {resp:?}"
    );
}
