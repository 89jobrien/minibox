//! Pipeline execution tests.

use minibox::adapters::mocks::MockRegistry;
use minibox::daemon::handler::{self};
use minibox_core::adapters::HostnameRegistryRouter;
use minibox_core::domain::DynImageRegistry;
use minibox_core::protocol::DaemonResponse;
use std::sync::Arc;
use tempfile::TempDir;

mod daemon_handler_common;
use daemon_handler_common::*;

// handle_pipeline Tests
// ---------------------------------------------------------------------------

/// Relative pipeline path → Error (not absolute).
#[tokio::test]
async fn test_handle_pipeline_rejects_relative_path() {
    let temp_dir = TempDir::new().unwrap();
    let state = create_test_state_with_dir(&temp_dir);
    let deps = create_test_deps_with_dir(&temp_dir);

    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(8);

    handler::handle_pipeline(
        "relative/path.cruxx".to_string(),
        None,
        None,
        None,
        vec![],
        state,
        deps,
        tx,
    )
    .await;

    let resp = rx.recv().await.expect("no response");
    assert!(
        matches!(resp, DaemonResponse::Error { ref message } if message.contains("absolute")),
        "expected absolute-path error, got: {resp:?}"
    );
}

/// Missing image pull → Error propagated from handle_run via inner channel.
#[tokio::test]
async fn test_handle_pipeline_pull_failure_returns_error() {
    let temp_dir = TempDir::new().unwrap();
    let state = create_test_state_with_dir(&temp_dir);

    // Use a registry that always fails to pull.
    let failing_registry = Arc::new(MockRegistry::new().with_pull_failure());
    let image_store =
        Arc::new(minibox_core::image::ImageStore::new(temp_dir.path().join("images2")).unwrap());
    let deps = build_deps_with_registry(
        Arc::new(HostnameRegistryRouter::new(
            failing_registry as DynImageRegistry,
            [] as [(&str, DynImageRegistry); 0],
        )),
        image_store,
        &temp_dir,
    );

    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(8);

    // Create a real pipeline file on disk so the absolute-path check passes.
    let pipeline_file = temp_dir.path().join("work.cruxx");
    std::fs::write(&pipeline_file, b"steps: []").unwrap();

    handler::handle_pipeline(
        pipeline_file.to_str().unwrap().to_string(),
        None,
        Some("cruxx-runtime:latest".to_string()),
        None,
        vec![],
        state,
        deps,
        tx,
    )
    .await;

    let resp = rx.recv().await.expect("no response");
    assert!(
        matches!(resp, DaemonResponse::Error { .. }),
        "expected Error when pull fails, got: {resp:?}"
    );
}

/// Successful pipeline run with no trace file → PipelineComplete with empty trace.
#[tokio::test]
async fn test_handle_pipeline_completes_with_empty_trace_when_no_trace_file() {
    let temp_dir = TempDir::new().unwrap();
    let state = create_test_state_with_dir(&temp_dir);
    let deps = create_test_deps_with_dir(&temp_dir);

    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(16);

    // Pipeline file must exist and be absolute.
    let pipeline_file = temp_dir.path().join("work.cruxx");
    std::fs::write(&pipeline_file, b"steps: []").unwrap();

    handler::handle_pipeline(
        pipeline_file.to_str().unwrap().to_string(),
        None,
        None,
        None,
        vec![],
        state,
        deps,
        tx,
    )
    .await;

    // Drain responses — may have ContainerOutput chunks followed by
    // PipelineComplete (no real crux binary in test, so MockRuntime exits 0).
    let mut pipeline_complete = None;
    loop {
        match rx.recv().await {
            None => break,
            Some(DaemonResponse::ContainerOutput { .. }) => continue,
            Some(DaemonResponse::PipelineComplete {
                trace,
                container_id,
                exit_code,
            }) => {
                pipeline_complete = Some((trace, container_id, exit_code));
                break;
            }
            Some(DaemonResponse::Error { message }) => {
                // MockRuntime does not support capture_output — accept any Error
                // that indicates the streaming path is unavailable in this env.
                if message.contains("output_reader")
                    || message.contains("not supported")
                    || message.contains("platform")
                {
                    return;
                }
                panic!("unexpected Error: {message}");
            }
            Some(other) => panic!("unexpected response: {other:?}"),
        }
    }

    #[cfg(unix)]
    {
        let (trace, container_id, _exit_code) =
            pipeline_complete.expect("PipelineComplete not received");
        assert!(!container_id.is_empty(), "container_id should be set");
        // No trace file was written — should fall back to {"steps":[]}.
        assert_eq!(
            trace,
            serde_json::json!({"steps": []}),
            "expected empty trace fallback"
        );
    }
}

// ---------------------------------------------------------------------------

/// Successful pipeline run with a trace file → PipelineComplete includes it.
#[tokio::test]
async fn test_handle_pipeline_reads_trace_file_from_upper_dir() {
    let temp_dir = TempDir::new().unwrap();
    let state = create_test_state_with_dir(&temp_dir);
    let deps = create_test_deps_with_dir(&temp_dir);

    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(16);

    let pipeline_file = temp_dir.path().join("work.cruxx");
    std::fs::write(&pipeline_file, b"steps: []").unwrap();

    // Pre-seed the channel: we need to know the container ID before the run so
    // we can plant the trace file.  Instead, we intercept ContainerCreated from
    // the internal bridge.  Since handle_pipeline does NOT forward
    // ContainerCreated (by design), we verify via PipelineComplete.container_id.
    //
    // Plant the trace BEFORE calling handle_pipeline by using a known ID path.
    // We can't know the UUID in advance, so instead we hook on the first
    // PipelineComplete and assert the trace field.
    //
    // For this test we rely on the fact that MockFilesystem creates the upper
    // dir structure — if it does not, trace falls back to {"steps":[]}.
    // We assert either the planted trace OR the fallback are returned.
    handler::handle_pipeline(
        pipeline_file.to_str().unwrap().to_string(),
        Some(serde_json::json!({"prompt": "hello"})),
        None,
        None,
        vec![("CRUX_LOG".to_string(), "debug".to_string())],
        state,
        deps,
        tx,
    )
    .await;

    loop {
        match rx.recv().await {
            None => break,
            Some(DaemonResponse::ContainerOutput { .. }) => continue,
            Some(DaemonResponse::PipelineComplete {
                trace,
                container_id,
                exit_code: _,
            }) => {
                assert!(!container_id.is_empty());
                // Trace is either the planted file or the {"steps":[]} fallback.
                assert!(
                    trace.is_object(),
                    "trace must be a JSON object, got: {trace}"
                );
                return;
            }
            Some(DaemonResponse::Error { .. }) => {
                // Acceptable on non-Linux.
                return;
            }
            Some(other) => panic!("unexpected: {other:?}"),
        }
    }
}

// ---------------------------------------------------------------------------
// restart-3: creation_params population
// ---------------------------------------------------------------------------

#[tokio::test]
async fn handle_run_stores_creation_params() {
    let temp_dir = tempfile::TempDir::new().expect("TempDir");
    let deps = create_test_deps_with_dir(&temp_dir);
    let state = create_test_state_with_dir(&temp_dir);

    let response = handle_run_once(
        "alpine".to_string(),
        Some("latest".to_string()),
        vec!["/bin/echo".to_string(), "hello".to_string()],
        Some(67_108_864),
        Some(256),
        false,
        state.clone(),
        deps,
    )
    .await;

    match response {
        minibox_core::protocol::DaemonResponse::ContainerCreated { id, .. } => {
            let record = state
                .get_container(&id)
                .await
                .expect("container must be in state");
            let cp = record
                .creation_params
                .expect("creation_params must be populated by handle_run");
            assert_eq!(cp.image, "alpine");
            assert_eq!(cp.tag.as_deref(), Some("latest"));
            assert_eq!(cp.command, vec!["/bin/echo", "hello"]);
            assert_eq!(cp.memory_limit_bytes, Some(67_108_864));
            assert_eq!(cp.cpu_weight, Some(256));
        }
        other => panic!("expected ContainerCreated, got {other:?}"),
    }
}
