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
//! Stream trait boundary tests for issue #370.
//!
//! Tests handler logic with in-memory mock implementations of the stream
//! (ContainerRuntime) and transport (ImageRegistry) traits, verifying:
//! - Correct framing of output chunks (base64-encoded ContainerOutput)
//! - Correct error propagation on stream/spawn failure
//! - Registry pull path exercised end-to-end with no network

mod daemon_handler_common;

use daemon_handler_common::*;
use minibox::adapters::mocks::{MockFilesystem, MockLimiter, MockNetwork, MockRegistry};
use minibox::daemon::handler::{
    self, BuildDeps, EventDeps, ExecDeps, HandlerDependencies, ImageDeps, LifecycleDeps,
};
use minibox_core::adapters::HostnameRegistryRouter;
use minibox_core::domain::DynImageRegistry;
use minibox_core::protocol::DaemonResponse;
use std::sync::Arc;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// MockAsyncStream — a ContainerRuntime that writes controlled payloads
// ---------------------------------------------------------------------------

/// Runtime mock that writes multiple distinct chunks to the output pipe,
/// allowing tests to verify that each chunk is framed as a separate
/// `ContainerOutput` message with correct base64 encoding.
#[cfg(unix)]
struct ChunkedMockRuntime {
    /// Each entry becomes a separate write to the pipe.
    chunks: Vec<Vec<u8>>,
}

#[cfg(unix)]
impl minibox_core::domain::AsAny for ChunkedMockRuntime {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(unix)]
#[async_trait::async_trait]
impl minibox_core::domain::ContainerRuntime for ChunkedMockRuntime {
    fn capabilities(&self) -> minibox_core::domain::RuntimeCapabilities {
        minibox_core::domain::RuntimeCapabilities {
            supports_user_namespaces: false,
            supports_cgroups_v2: false,
            supports_overlay_fs: false,
            supports_network_isolation: false,
            max_containers: None,
        }
    }

    async fn spawn_process(
        &self,
        _config: &minibox_core::domain::ContainerSpawnConfig,
    ) -> anyhow::Result<minibox_core::domain::SpawnResult> {
        use std::io::Write;
        use std::os::unix::io::{FromRawFd, IntoRawFd, OwnedFd};

        let (read_fd, write_fd) = nix::unistd::pipe().expect("pipe");
        let write_raw = write_fd.into_raw_fd();
        let chunks = self.chunks.clone();

        // Write chunks in a background thread so the pipe doesn't block.
        std::thread::spawn(move || {
            // SAFETY: write_raw is the write end of our pipe, valid until close.
            let mut w = unsafe { std::fs::File::from_raw_fd(write_raw) };
            for chunk in &chunks {
                let _ = w.write_all(chunk);
                // Small sleep to encourage separate read() calls in the drain loop.
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            // File::drop closes write_raw → EOF for reader.
        });

        let read_raw = read_fd.into_raw_fd();
        // SAFETY: read_raw is the read end of our pipe, transferred to OwnedFd.
        let output_reader = unsafe { OwnedFd::from_raw_fd(read_raw) };

        Ok(minibox_core::domain::SpawnResult {
            pid: u32::MAX - 1,
            output_reader: Some(output_reader),
            runtime_id: None,
        })
    }

    async fn wait_for_exit(&self, _runtime_id: Option<&str>, _pid: u32) -> anyhow::Result<i32> {
        // Simulate container exit after a brief delay.
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        Ok(0)
    }
}

// ---------------------------------------------------------------------------
// RecordingRegistry — records pull calls and returns pre-baked data
// ---------------------------------------------------------------------------

/// Registry mock that records every `pull_image` call with the full image ref
/// and returns pre-configured layer data. Wraps MockRegistry for the heavy
/// lifting and adds call recording.
struct RecordingRegistry {
    inner: MockRegistry,
    pull_log: Arc<std::sync::Mutex<Vec<String>>>,
}

impl minibox_core::domain::AsAny for RecordingRegistry {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[async_trait::async_trait]
impl minibox_core::domain::ImageRegistry for RecordingRegistry {
    async fn has_image(&self, name: &str, tag: &str) -> bool {
        self.inner.has_image(name, tag).await
    }

    async fn pull_image(
        &self,
        image_ref: &minibox_core::image::reference::ImageRef,
    ) -> anyhow::Result<minibox_core::domain::ImageMetadata> {
        self.pull_log.lock().expect("lock pull_log").push(format!(
            "{}:{}",
            image_ref.cache_name(),
            image_ref.tag
        ));
        self.inner.pull_image(image_ref).await
    }

    fn get_image_layers(&self, name: &str, tag: &str) -> anyhow::Result<Vec<std::path::PathBuf>> {
        self.inner.get_image_layers(name, tag)
    }
}

// ---------------------------------------------------------------------------
// Helper: build deps with custom runtime and registry
// ---------------------------------------------------------------------------

#[cfg(unix)]
fn build_stream_test_deps(
    temp_dir: &TempDir,
    runtime: Arc<dyn minibox_core::domain::ContainerRuntime>,
    registry: Arc<dyn minibox_core::domain::ImageRegistry>,
) -> Arc<HandlerDependencies> {
    let image_store = Arc::new(
        minibox_core::image::ImageStore::new(temp_dir.path().join("img-stream"))
            .expect("image store"),
    );
    Arc::new(HandlerDependencies {
        image: ImageDeps {
            registry_router: Arc::new(HostnameRegistryRouter::new(
                registry as DynImageRegistry,
                [("ghcr.io", Arc::new(MockRegistry::new()) as DynImageRegistry)],
            )),
            image_loader: Arc::new(minibox::daemon::handler::NoopImageLoader),
            image_gc: Arc::new(NoopImageGc),
            image_store,
        },
        lifecycle: LifecycleDeps {
            filesystem: Arc::new(MockFilesystem::new()),
            resource_limiter: Arc::new(MockLimiter::new()),
            runtime,
            network_provider: Arc::new(MockNetwork::new()),
            containers_base: temp_dir.path().join("containers"),
            run_containers_base: temp_dir.path().join("run"),
        },
        exec: ExecDeps {
            exec_runtime: None,
            pty_sessions: Arc::new(tokio::sync::Mutex::new(
                minibox::daemon::handler::PtySessionRegistry::default(),
            )),
        },
        build: BuildDeps {
            image_pusher: None,
            commit_adapter: None,
            image_builder: None,
        },
        events: EventDeps {
            event_sink: Arc::new(minibox_core::events::NoopEventSink),
            event_source: Arc::new(minibox_core::events::BroadcastEventBroker::new()),
            metrics: Arc::new(minibox::daemon::telemetry::NoOpMetricsRecorder::new()),
        },
        policy: minibox::daemon::handler::ContainerPolicy {
            allow_bind_mounts: true,
            allow_privileged: true,
            ..Default::default()
        },
        execution_policy: None,
        checkpoint: Arc::new(minibox_core::domain::NoopVmCheckpoint),
    })
}

// ---------------------------------------------------------------------------
// Test 1: Correct framing of output chunks
// ---------------------------------------------------------------------------

/// Streaming run with multiple payload chunks verifies that all bytes arrive
/// as base64-encoded `ContainerOutput` messages in the correct order,
/// bookended by `ContainerCreated` and `ContainerStopped`.
#[tokio::test]
#[cfg(unix)]
async fn test_stream_output_chunks_are_correctly_framed() {
    let chunk_a = b"chunk-alpha\n".to_vec();
    let chunk_b = b"chunk-bravo\n".to_vec();
    let expected_total: Vec<u8> = [chunk_a.as_slice(), chunk_b.as_slice()].concat();

    let temp_dir = TempDir::new().expect("tempdir");
    let registry = Arc::new(MockRegistry::new().with_cached_image("library/alpine", "latest"));
    let runtime = Arc::new(ChunkedMockRuntime {
        chunks: vec![chunk_a, chunk_b],
    });
    let deps = build_stream_test_deps(&temp_dir, runtime, registry);
    let state = create_test_state_with_dir(&temp_dir);

    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(32);
    handler::handle_run(
        handler::RunParams {
            image: "alpine".to_string(),
            tag: Some("latest".to_string()),
            command: vec!["/bin/true".to_string()],
            memory_limit_bytes: None,
            cpu_weight: None,
            ephemeral: true,
            network: // ephemeral → streaming
        None,
            mounts: vec![],
            privileged: false,
            shared_uid_range: false,
            env: vec![],
            name: None,
            platform: None,
            cgroup_parent: None, priority: None, policy_override: None,
        },
        state,
        deps,
        tx,
    )
    .await;

    let mut responses = Vec::new();
    while let Some(r) = rx.recv().await {
        responses.push(r);
    }

    // First message must be ContainerCreated.
    assert!(
        matches!(&responses[0], DaemonResponse::ContainerCreated { .. }),
        "first message must be ContainerCreated, got: {:?}",
        responses[0]
    );

    // Last message must be ContainerStopped.
    let last = responses.last().expect("at least one response");
    assert!(
        matches!(last, DaemonResponse::ContainerStopped { .. }),
        "last message must be ContainerStopped, got: {last:?}"
    );

    // All ContainerOutput data concatenated must equal the original payload.
    let output_bytes: Vec<u8> = responses
        .iter()
        .filter_map(|r| {
            if let DaemonResponse::ContainerOutput { data, .. } = r {
                use base64::Engine;
                base64::engine::general_purpose::STANDARD.decode(data).ok()
            } else {
                None
            }
        })
        .flatten()
        .collect();

    assert_eq!(
        output_bytes, expected_total,
        "concatenated output must match all written chunks"
    );

    // There must be at least one ContainerOutput message.
    let output_count = responses
        .iter()
        .filter(|r| matches!(r, DaemonResponse::ContainerOutput { .. }))
        .count();
    assert!(
        output_count >= 1,
        "expected at least 1 ContainerOutput message, got {output_count}"
    );
}

// ---------------------------------------------------------------------------
// Test 2: Error propagation on spawn failure
// ---------------------------------------------------------------------------

/// When the runtime's `spawn_process` fails, the streaming path must
/// propagate the error as a `DaemonResponse::Error` through the channel.
#[tokio::test]
#[cfg(unix)]
async fn test_stream_spawn_failure_propagates_error() {
    let temp_dir = TempDir::new().expect("tempdir");
    let registry = Arc::new(MockRegistry::new().with_cached_image("library/alpine", "latest"));
    let runtime = Arc::new(minibox::adapters::mocks::MockRuntime::new().with_spawn_failure());
    let deps = build_stream_test_deps(&temp_dir, runtime, registry);
    let state = create_test_state_with_dir(&temp_dir);

    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);
    handler::handle_run(
        handler::RunParams {
            image: "alpine".to_string(),
            tag: Some("latest".to_string()),
            command: vec!["/bin/sh".to_string()],
            memory_limit_bytes: None,
            cpu_weight: None,
            ephemeral: true,
            network: // ephemeral → streaming
        None,
            mounts: vec![],
            privileged: false,
            shared_uid_range: false,
            env: vec![],
            name: None,
            platform: None,
            cgroup_parent: None, priority: None, policy_override: None,
        },
        state,
        deps,
        tx,
    )
    .await;

    let response = rx.recv().await.expect("handler must send a response");
    assert!(
        matches!(response, DaemonResponse::Error { .. }),
        "spawn failure must produce Error response, got: {response:?}"
    );
}

// ---------------------------------------------------------------------------
// Test 3: Registry pull path exercised end-to-end with recording
// ---------------------------------------------------------------------------

/// When the image is not cached, `handle_run` pulls it via the registry.
/// `RecordingRegistry` captures the pull call and verifies the image ref.
/// The full run completes without network access.
#[tokio::test]
#[cfg(unix)]
async fn test_registry_pull_path_recorded_end_to_end() {
    let temp_dir = TempDir::new().expect("tempdir");
    let pull_log = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let registry = Arc::new(RecordingRegistry {
        inner: MockRegistry::new(), // no cached images → forces pull
        pull_log: Arc::clone(&pull_log),
    });
    let runtime = Arc::new(ChunkedMockRuntime {
        chunks: vec![b"output\n".to_vec()],
    });
    let deps = build_stream_test_deps(&temp_dir, runtime, registry);
    let state = create_test_state_with_dir(&temp_dir);

    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(32);
    handler::handle_run(
        handler::RunParams {
            image: "myapp".to_string(),
            tag: Some("v2".to_string()),
            command: vec!["/bin/true".to_string()],
            memory_limit_bytes: None,
            cpu_weight: None,
            ephemeral: true,
            network: // ephemeral
        None,
            mounts: vec![],
            privileged: false,
            shared_uid_range: false,
            env: vec![],
            name: None,
            platform: None,
            cgroup_parent: None, priority: None, policy_override: None,
        },
        state,
        deps,
        tx,
    )
    .await;

    // Drain responses.
    let mut responses = Vec::new();
    while let Some(r) = rx.recv().await {
        responses.push(r);
    }

    // Verify the pull was recorded with the correct image ref.
    let log = pull_log.lock().expect("lock");
    assert_eq!(
        log.len(),
        1,
        "expected exactly 1 pull call, got {}",
        log.len()
    );
    assert!(
        log[0].contains("myapp") && log[0].contains("v2"),
        "pull log entry should contain image name and tag, got: {}",
        log[0]
    );

    // Verify the run completed successfully (ContainerCreated + ContainerStopped).
    assert!(
        responses
            .iter()
            .any(|r| matches!(r, DaemonResponse::ContainerCreated { .. })),
        "expected ContainerCreated in responses"
    );
    assert!(
        responses
            .iter()
            .any(|r| matches!(r, DaemonResponse::ContainerStopped { .. })),
        "expected ContainerStopped in responses"
    );
}

// ---------------------------------------------------------------------------
// Test 4: Registry pull failure propagates as Error
// ---------------------------------------------------------------------------

/// When the registry's pull fails, the handler sends a `DaemonResponse::Error`
/// without attempting to spawn a container.
#[tokio::test]
#[cfg(unix)]
async fn test_registry_pull_failure_propagates_through_stream() {
    let temp_dir = TempDir::new().expect("tempdir");
    let pull_log = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let registry = Arc::new(RecordingRegistry {
        inner: MockRegistry::new().with_pull_failure(),
        pull_log: Arc::clone(&pull_log),
    });
    let runtime = Arc::new(ChunkedMockRuntime { chunks: vec![] });
    let deps = build_stream_test_deps(&temp_dir, runtime, registry);
    let state = create_test_state_with_dir(&temp_dir);

    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);
    handler::handle_run(
        handler::RunParams {
            image: "badimage".to_string(),
            tag: Some("v1".to_string()),
            command: vec!["/bin/sh".to_string()],
            memory_limit_bytes: None,
            cpu_weight: None,
            ephemeral: true,
            network: None,
            mounts: vec![],
            privileged: false,
            shared_uid_range: false,
            env: vec![],
            name: None,
            platform: None,
            cgroup_parent: None,
            priority: None,
            policy_override: None,
        },
        state,
        deps,
        tx,
    )
    .await;

    let response = rx.recv().await.expect("handler must send a response");
    assert!(
        matches!(response, DaemonResponse::Error { .. }),
        "registry pull failure must produce Error, got: {response:?}"
    );

    // Pull was attempted.
    let log = pull_log.lock().expect("lock");
    assert_eq!(log.len(), 1, "pull should have been attempted once");
}
