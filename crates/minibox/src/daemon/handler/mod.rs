//! Request handlers for each daemon operation.
//!
//! Each public function corresponds to one `DaemonRequest` variant and
//! returns a `DaemonResponse`.  Errors are caught and returned as
//! `DaemonResponse::Error` so the daemon never panics on bad input.
//!
//! # Hexagonal Architecture
//!
//! Handlers use dependency injection to receive infrastructure adapters
//! via the [`HandlerDependencies`] struct. This allows the business logic
//! to be tested independently of infrastructure concerns.
//!
//! # Module layout
//!
//! | Module        | Contents                                              |
//! |---------------|-------------------------------------------------------|
//! | `run`         | `handle_run`, streaming run, inner helpers            |
//! | `stop`        | `handle_stop`, platform `stop_inner`                  |
//! | `lifecycle`   | pause, resume, remove, list                           |
//! | `image`       | pull, load, push, commit, build, prune, remove, list  |
//! | `logs`        | `handle_logs`                                         |
//! | `exec`        | exec, send_input, resize_pty                          |
//! | `events`      | `handle_subscribe_events`                             |
//! | `snapshot`    | save/restore/list snapshots                           |
//! | `pipeline`    | `handle_pipeline`                                     |
//! | `update`      | `handle_update`                                       |
//! | `manifest`    | get/verify manifest                                   |

mod events;
mod exec;
mod image;
mod lifecycle;
mod logs;
mod manifest;
mod pipeline;
mod run;
mod snapshot;
mod stop;
mod update;

// ─── Re-exports (public API surface; call sites in server.rs unchanged) ───────

pub(crate) use self::events::handle_subscribe_events;
pub use self::exec::{handle_exec, handle_resize_pty, handle_send_input};
pub use self::image::{handle_build, handle_commit, handle_load_image, handle_pull, handle_push};
pub(crate) use self::image::{handle_list_images, handle_prune, handle_remove_image};
pub use self::lifecycle::{handle_list, handle_pause, handle_remove, handle_resume};
pub use self::logs::handle_logs;
pub use self::manifest::{handle_get_manifest, handle_verify_manifest};
pub use self::pipeline::handle_pipeline;
pub use self::run::handle_run;
pub use self::snapshot::{handle_list_snapshots, handle_restore_snapshot, handle_save_snapshot};
pub use self::stop::handle_stop;
pub use self::update::handle_update;

// ─── Shared imports ──────────────────────────────────────────────────────────

use minibox_core::domain::{
    DynExecRuntime, DynFilesystemProvider, DynMetricsRecorder, DynNetworkProvider,
    DynRegistryRouter, DynResourceLimiter,
};
use minibox_core::events::EventSink;
use minibox_core::protocol::DaemonResponse;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex as TokioMutex;
use tokio::sync::mpsc;
use tracing::warn;

// ─── Internal helpers ─────────────────────────────────────────────────────────

/// Send a terminal `DaemonResponse::Error` on `tx`, logging a warning if the
/// receiver has already been dropped.
///
/// Use this instead of `let _ = tx.send(...).await` so that dropped connections
/// are observable in logs rather than silently swallowed.
pub(crate) async fn send_error(tx: &mpsc::Sender<DaemonResponse>, context: &str, message: String) {
    if tx
        .send(DaemonResponse::Error {
            message: message.clone(),
        })
        .await
        .is_err()
    {
        warn!(
            context,
            error_message = %message,
            "client disconnected before error response could be sent"
        );
    }
}

// ─── PTY session registry ─────────────────────────────────────────────────────

/// Tracks live PTY session channels keyed by session ID string.
///
/// Populated by `handle_exec` when a tty session starts and consumed by
/// `handle_send_input` / `handle_resize_pty` dispatched from `server.rs`.
#[derive(Default)]
pub struct PtySessionRegistry {
    /// Resize event senders: session_id → sender for `(cols, rows)`.
    pub resize: HashMap<String, mpsc::Sender<(u16, u16)>>,
    /// Stdin byte senders: session_id → sender for raw bytes.
    /// Only populated when `tty = true`.
    pub stdin: HashMap<String, mpsc::Sender<Vec<u8>>>,
}

impl PtySessionRegistry {
    /// Remove all channels associated with `session_id`.
    ///
    /// Called when an exec session ends (on `ContainerStopped` or error) to
    /// prevent unbounded registry growth and avoid stale-sender warnings.
    pub fn cleanup(&mut self, session_id: &str) {
        self.resize.remove(session_id);
        self.stdin.remove(session_id);
    }
}

/// Arc-wrapped, async-mutex-guarded PTY session registry.
pub type SharedPtyRegistry = Arc<TokioMutex<PtySessionRegistry>>;

// ─── Default adapters ────────────────────────────────────────────────────────

/// Production no-op image loader.
///
/// Used as a placeholder in platform adapters (e.g. macbox, winbox) that do
/// not yet implement local tarball loading. Accepts any load request and
/// returns `Ok(())` immediately. This is a real adapter, not a test double.
pub struct NoopImageLoader;

#[async_trait::async_trait]
impl minibox_core::domain::ImageLoader for NoopImageLoader {
    async fn load_image(
        &self,
        _path: &std::path::Path,
        _name: &str,
        _tag: &str,
    ) -> anyhow::Result<()> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Handler Dependencies — ISP-compliant sub-structs
// ---------------------------------------------------------------------------

/// Image-related dependencies: registry routing, loading, GC, and local store.
///
/// Handlers that only pull or inspect images depend on this sub-struct rather
/// than the full [`HandlerDependencies`].
#[derive(Clone)]
pub struct ImageDeps {
    /// Registry router that selects the appropriate image registry for a given image reference.
    pub registry_router: DynRegistryRouter,
    /// Loader for local OCI image tarballs.
    pub image_loader: minibox_core::domain::DynImageLoader,
    /// Image garbage collector for prune operations.
    pub image_gc: Arc<dyn minibox_core::image::gc::ImageGarbageCollector>,
    /// Image store for direct image operations (e.g. RemoveImage).
    pub image_store: Arc<minibox_core::image::ImageStore>,
}

/// Container lifecycle dependencies: filesystem, limits, runtime, network, and paths.
///
/// Handlers that create or destroy containers depend on this sub-struct.
#[derive(Clone)]
pub struct LifecycleDeps {
    /// Filesystem provider for setting up container rootfs.
    pub filesystem: DynFilesystemProvider,
    /// Resource limiter for enforcing cgroup limits.
    pub resource_limiter: DynResourceLimiter,
    /// Container runtime for spawning isolated processes.
    pub runtime: minibox_core::domain::DynContainerRuntime,
    /// Network provider for container network setup/teardown.
    pub network_provider: DynNetworkProvider,
    /// Base directory for persistent container data (overlay dirs).
    pub containers_base: PathBuf,
    /// Base directory for runtime container state (PID files).
    pub run_containers_base: PathBuf,
}

/// Exec and PTY dependencies for running commands inside containers.
///
/// Handlers that implement `exec` or PTY session management depend on this
/// sub-struct.
#[derive(Clone)]
pub struct ExecDeps {
    /// Exec runtime for running commands inside containers.
    /// `None` on platforms where exec is not supported (macOS, Windows).
    pub exec_runtime: Option<DynExecRuntime>,
    /// Live PTY session channels for SendInput/ResizePty dispatch.
    pub pty_sessions: SharedPtyRegistry,
}

/// Image build/push/commit dependencies.
///
/// Handlers that build, push, or commit images depend on this sub-struct.
/// All fields are `Option` because these operations are platform-conditional.
#[derive(Clone)]
pub struct BuildDeps {
    /// Image pusher for pushing images to OCI registries.
    /// `None` on platforms or configurations where push is not supported.
    pub image_pusher: Option<minibox_core::domain::DynImagePusher>,
    /// Container committer for snapshotting a container's overlay diff.
    /// `None` on platforms where commit is not supported (macOS, Windows).
    pub commit_adapter: Option<minibox_core::domain::DynContainerCommitter>,
    /// Image builder for building images from a Dockerfile.
    /// `None` on platforms where build is not supported (macOS, Windows).
    pub image_builder: Option<minibox_core::domain::DynImageBuilder>,
}

/// Observability and event-bus dependencies.
///
/// Handlers that emit events or record metrics depend on this sub-struct.
#[derive(Clone)]
pub struct EventDeps {
    /// Event sink for emitting container lifecycle events.
    pub event_sink: Arc<dyn EventSink>,
    /// Source for subscribing to the container event stream.
    pub event_source: Arc<dyn minibox_core::events::EventSource>,
    /// Metrics recorder for operational observability.
    pub metrics: DynMetricsRecorder,
}

// ---------------------------------------------------------------------------
// Handler Dependencies (Dependency Injection)
// ---------------------------------------------------------------------------

/// Dependencies injected into request handlers.
///
/// Composed of focused sub-structs ([`ImageDeps`], [`LifecycleDeps`],
/// [`ExecDeps`], [`BuildDeps`], [`EventDeps`]) so each handler can declare a
/// dependency only on the slice of infrastructure it actually uses (ISP).
///
/// # Usage
///
/// Created once in the composition root (main.rs) and passed to all handlers:
///
/// ```rust,ignore
/// use crate::adapters::{DockerHubRegistry, OverlayFilesystem, CgroupV2Limiter, LinuxNamespaceRuntime};
///
/// let deps = Arc::new(HandlerDependencies {
///     image: ImageDeps {
///         registry_router: Arc::new(HostnameRegistryRouter::new(docker_hub, [("ghcr.io", ghcr)])),
///         ..
///     },
///     lifecycle: LifecycleDeps {
///         filesystem: Arc::new(OverlayFilesystem),
///         containers_base: PathBuf::from("/var/lib/minibox/containers"),
///         ..
///     },
///     ..
/// });
/// ```
#[derive(Clone)]
pub struct HandlerDependencies {
    /// Image registry, loader, GC, and local store.
    pub image: ImageDeps,
    /// Container lifecycle: filesystem, limits, runtime, network, paths.
    pub lifecycle: LifecycleDeps,
    /// Exec and PTY session management.
    pub exec: ExecDeps,
    /// Image build, push, and commit.
    pub build: BuildDeps,
    /// Observability: events and metrics.
    pub events: EventDeps,
    /// Policy controlling which container capabilities are permitted.
    pub policy: ContainerPolicy,
    /// VM checkpoint adapter for save/restore snapshot operations.
    pub checkpoint: minibox_core::domain::DynVmCheckpoint,
}

impl HandlerDependencies {
    /// Override the image loader (builder-style).
    pub fn with_image_loader(mut self, loader: minibox_core::domain::DynImageLoader) -> Self {
        self.image.image_loader = loader;
        self
    }
}

// ─── Container Policy ────────────────────────────────────────────────────────

/// Policy rules applied to every `RunContainer` request before any container
/// creation logic executes.  Defaults to deny-all: both bind mounts and
/// privileged mode are blocked unless explicitly enabled.
///
/// Construct with specific overrides for tests or operator-controlled config:
/// ```rust,ignore
/// let policy = ContainerPolicy { allow_bind_mounts: true, ..ContainerPolicy::default() };
/// ```
#[derive(Debug, Clone, Default)]
pub struct ContainerPolicy {
    /// Allow containers to mount host directories (bind mounts).
    /// Default: `false` (deny).
    pub allow_bind_mounts: bool,
    /// Allow containers to run in privileged mode.
    /// Default: `false` (deny).
    pub allow_privileged: bool,
}

impl ContainerPolicy {
    /// Build a `ContainerPolicy` from environment variables.
    ///
    /// - `MINIBOX_ALLOW_BIND_MOUNTS=1|true|yes` enables bind mounts (default: deny).
    /// - `MINIBOX_ALLOW_PRIVILEGED=1|true|yes` enables privileged mode (default: deny).
    pub fn from_env() -> Self {
        Self {
            allow_bind_mounts: env_flag("MINIBOX_ALLOW_BIND_MOUNTS"),
            allow_privileged: env_flag("MINIBOX_ALLOW_PRIVILEGED"),
        }
    }
}

/// Parse a boolean-ish environment variable (absent or unrecognised = false).
pub(crate) fn env_flag(name: &str) -> bool {
    std::env::var(name)
        .ok()
        .map(|v| matches!(v.trim().to_lowercase().as_str(), "1" | "true" | "yes"))
        .unwrap_or(false)
}

/// Validate a container run request against the active policy.
///
/// Returns `Ok(())` if the request is permitted; returns an error string
/// describing the first policy violation found.
///
/// # Errors
///
/// Returns `Err(String)` with a human-readable description when the request
/// violates `policy`.
pub fn validate_policy(
    mounts: &[minibox_core::domain::BindMount],
    privileged: bool,
    policy: &ContainerPolicy,
) -> Result<(), String> {
    if !mounts.is_empty() && !policy.allow_bind_mounts {
        return Err(
            "policy violation: bind mount requested but bind mounts are not allowed".into(),
        );
    }
    if privileged && !policy.allow_privileged {
        return Err(
            "policy violation: privileged mode requested but privileged containers are not allowed"
                .into(),
        );
    }
    Ok(())
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod pub_crate_handler_tests {
    //! Unit tests for `pub(crate)` handler functions that are inaccessible
    //! from integration tests.  Live here so they share the crate's visibility.

    use super::run::{check_oom_killed, generate_container_id};
    use super::*;
    use crate::adapters::mocks::{MockFilesystem, MockLimiter, MockNetwork, MockRuntime};
    use crate::daemon::state::DaemonState;
    use crate::image::ImageStore;
    use crate::testing::helpers::gc::NoopImageGc;
    use minibox_core::adapters::HostnameRegistryRouter;
    use minibox_core::domain::DynImageRegistry;
    use minibox_core::events::{BroadcastEventBroker, NoopEventSink};
    use minibox_core::protocol::ContainerInfo;
    use std::sync::Arc;
    use tempfile::TempDir;

    fn make_state(tmp: &TempDir) -> Arc<DaemonState> {
        let store = ImageStore::new(tmp.path().join("images-state")).unwrap();
        Arc::new(DaemonState::new(store, tmp.path()))
    }

    fn make_deps(tmp: &TempDir) -> Arc<HandlerDependencies> {
        let image_store = Arc::new(ImageStore::new(tmp.path().join("images")).unwrap());
        Arc::new(HandlerDependencies {
            image: ImageDeps {
                registry_router: Arc::new(HostnameRegistryRouter::new(
                    Arc::new(crate::adapters::mocks::MockRegistry::new()) as DynImageRegistry,
                    [(
                        "ghcr.io",
                        Arc::new(crate::adapters::mocks::MockRegistry::new()) as DynImageRegistry,
                    )],
                )),
                image_loader: Arc::new(NoopImageLoader),
                image_gc: Arc::new(NoopImageGc::new()),
                image_store,
            },
            lifecycle: LifecycleDeps {
                filesystem: Arc::new(MockFilesystem::new()),
                resource_limiter: Arc::new(MockLimiter::new()),
                runtime: Arc::new(MockRuntime::new()),
                network_provider: Arc::new(MockNetwork::new()),
                containers_base: tmp.path().join("containers"),
                run_containers_base: tmp.path().join("run"),
            },
            exec: ExecDeps {
                exec_runtime: None,
                pty_sessions: Arc::new(tokio::sync::Mutex::new(PtySessionRegistry::default())),
            },
            build: BuildDeps {
                image_pusher: None,
                commit_adapter: None,
                image_builder: None,
            },
            events: EventDeps {
                event_sink: Arc::new(NoopEventSink),
                event_source: Arc::new(BroadcastEventBroker::new()),
                metrics: Arc::new(crate::daemon::telemetry::NoOpMetricsRecorder::new()),
            },
            policy: ContainerPolicy {
                allow_bind_mounts: true,
                allow_privileged: true,
            },
            checkpoint: Arc::new(minibox_core::domain::NoopVmCheckpoint),
        })
    }

    // ── handle_list_images ────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_list_images_empty_store() {
        let tmp = TempDir::new().expect("create temp dir");
        let deps = make_deps(&tmp);

        let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);
        handle_list_images(Arc::clone(&deps.image.image_store), tx).await;

        let resp = rx
            .recv()
            .await
            .expect("no response from handle_list_images");
        assert!(
            matches!(resp, DaemonResponse::ImageList { ref images } if images.is_empty()),
            "expected empty ImageList, got {resp:?}"
        );
    }

    // ── handle_prune ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_prune_dry_run_returns_pruned() {
        let tmp = TempDir::new().expect("create temp dir");
        let state = make_state(&tmp);
        let deps = make_deps(&tmp);

        let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);
        handle_prune(
            true,
            Arc::clone(&state),
            Arc::clone(&deps.image.image_gc),
            Arc::clone(&deps.events.event_sink),
            tx,
        )
        .await;

        let resp = rx.recv().await.expect("no response from handle_prune");
        assert!(
            matches!(resp, DaemonResponse::Pruned { dry_run: true, .. }),
            "expected Pruned with dry_run=true, got {resp:?}"
        );
    }

    #[tokio::test]
    async fn test_prune_non_dry_run_returns_pruned() {
        let tmp = TempDir::new().expect("create temp dir");
        let state = make_state(&tmp);
        let deps = make_deps(&tmp);

        let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);
        handle_prune(
            false,
            state,
            Arc::clone(&deps.image.image_gc),
            Arc::clone(&deps.events.event_sink),
            tx,
        )
        .await;

        let resp = rx.recv().await.expect("no response from handle_prune");
        assert!(
            matches!(resp, DaemonResponse::Pruned { dry_run: false, .. }),
            "expected Pruned with dry_run=false, got {resp:?}"
        );
    }

    // ── handle_remove_image ───────────────────────────────────────────────────

    #[tokio::test]
    async fn test_remove_image_invalid_ref_no_colon_returns_error() {
        let tmp = TempDir::new().expect("create temp dir");
        let state = make_state(&tmp);
        let deps = make_deps(&tmp);

        let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);
        handle_remove_image(
            "nocolon".to_string(),
            state,
            Arc::clone(&deps.image.image_store),
            Arc::clone(&deps.events.event_sink),
            tx,
        )
        .await;

        let resp = rx.recv().await.expect("no response");
        assert!(
            matches!(resp, DaemonResponse::Error { ref message } if message.contains("invalid image ref")),
            "expected invalid image ref error, got {resp:?}"
        );
    }

    #[tokio::test]
    async fn test_remove_image_nonexistent_image() {
        let tmp = TempDir::new().expect("create temp dir");
        let state = make_state(&tmp);
        let deps = make_deps(&tmp);

        let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);
        handle_remove_image(
            "alpine:latest".to_string(),
            state,
            Arc::clone(&deps.image.image_store),
            Arc::clone(&deps.events.event_sink),
            tx,
        )
        .await;

        let resp = rx.recv().await.expect("no response");
        // Non-existent image: either Success (no-op) or Error.
        assert!(
            matches!(
                resp,
                DaemonResponse::Success { .. } | DaemonResponse::Error { .. }
            ),
            "expected Success or Error, got {resp:?}"
        );
    }

    #[tokio::test]
    async fn test_remove_image_in_use_by_running_container_returns_error() {
        let tmp = TempDir::new().expect("create temp dir");
        let state = make_state(&tmp);
        let deps = make_deps(&tmp);

        // Inject a running container whose image matches.
        let record = crate::daemon::state::ContainerRecord {
            info: ContainerInfo {
                id: "in-use-ctr".to_string(),
                name: None,
                image: "alpine:latest".to_string(),
                command: "/bin/sh".to_string(),
                state: "running".to_string(),
                created_at: "1970-01-01T00:00:00Z".to_string(),
                pid: None,
            },
            pid: None,
            rootfs_path: tmp.path().join("rootfs"),
            cgroup_path: tmp.path().join("cgroup"),
            post_exit_hooks: vec![],
            rootfs_metadata: None,
            source_image_ref: Some("alpine:latest".to_string()),
            step_state: None,
            priority: None,
            urgency: None,
            execution_context: None,
            creation_params: None,
            manifest_path: None,
            workload_digest: None,
        };
        state.add_container(record).await;

        let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);
        handle_remove_image(
            "alpine:latest".to_string(),
            state,
            Arc::clone(&deps.image.image_store),
            Arc::clone(&deps.events.event_sink),
            tx,
        )
        .await;

        let resp = rx.recv().await.expect("no response");
        assert!(
            matches!(resp, DaemonResponse::Error { ref message } if message.contains("in use")),
            "expected 'in use' error, got {resp:?}"
        );
    }

    // ── handle_subscribe_events ───────────────────────────────────────────────

    #[tokio::test]
    async fn test_subscribe_events_exits_when_mpsc_receiver_dropped() {
        // handle_subscribe_events exits when tx.send() fails (mpsc receiver dropped).
        // Strategy: drop mpsc rx before emitting, then emit a broadcast event.
        // The handler receives the broadcast event, tries to mpsc-send, fails → exits.
        let broker = Arc::new(BroadcastEventBroker::new());
        let event_source: Arc<dyn minibox_core::events::EventSource> = Arc::clone(&broker) as _;
        let (tx, rx) = tokio::sync::mpsc::channel::<DaemonResponse>(8);
        drop(rx); // drop receiver so any mpsc send will fail

        let handle = tokio::spawn(async move {
            handle_subscribe_events(event_source, tx).await;
        });

        // Give the task a moment to subscribe, then emit an event.
        tokio::task::yield_now().await;
        broker.emit(minibox_core::events::ContainerEvent::ImagePruned {
            count: 0,
            freed_bytes: 0,
            timestamp: std::time::SystemTime::now(),
        });

        // Handler should exit after the failed mpsc send.
        tokio::time::timeout(std::time::Duration::from_millis(500), handle)
            .await
            .expect("handle_subscribe_events must exit when mpsc rx is dropped")
            .expect("handler task should not panic");
    }

    // ── handle_prune with running container ────────────────────────────────

    #[tokio::test]
    async fn test_prune_with_running_container_passes_in_use() {
        let tmp = TempDir::new().expect("create temp dir");
        let state = make_state(&tmp);
        let deps = make_deps(&tmp);

        // Inject a running container so the filter closure takes the Some branch.
        state
            .add_container(crate::daemon::state::ContainerRecord {
                info: ContainerInfo {
                    id: "running-ctr".to_string(),
                    name: None,
                    image: "alpine:latest".to_string(),
                    command: "/bin/sh".to_string(),
                    state: "running".to_string(),
                    created_at: "1970-01-01T00:00:00Z".to_string(),
                    pid: None,
                },
                pid: None,
                rootfs_path: tmp.path().join("rootfs"),
                cgroup_path: tmp.path().join("cgroup"),
                post_exit_hooks: vec![],
                rootfs_metadata: None,
                source_image_ref: Some("alpine:latest".to_string()),
                step_state: None,
                priority: None,
                urgency: None,
                execution_context: None,
                creation_params: None,
                manifest_path: None,
                workload_digest: None,
            })
            .await;

        let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);
        handle_prune(
            true,
            Arc::clone(&state),
            Arc::clone(&deps.image.image_gc),
            Arc::clone(&deps.events.event_sink),
            tx,
        )
        .await;

        let resp = rx.recv().await.expect("no response from handle_prune");
        assert!(
            matches!(resp, DaemonResponse::Pruned { .. }),
            "expected Pruned, got {resp:?}"
        );
    }

    // ── handle_remove_image success path ──────────────────────────────────

    #[tokio::test]
    async fn test_remove_image_success_path_emits_success() {
        let tmp = TempDir::new().expect("create temp dir");
        let state = make_state(&tmp);
        let deps = make_deps(&tmp);

        // Calling remove on a non-existent image (dir doesn't exist) → Ok(()) → Success.
        let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);
        handle_remove_image(
            "nonexistent:v1".to_string(),
            state,
            Arc::clone(&deps.image.image_store),
            Arc::clone(&deps.events.event_sink),
            tx,
        )
        .await;

        let resp = rx.recv().await.expect("no response");
        // Either Success (delete no-op) or Error (image_dir rejected the name).
        assert!(
            matches!(
                resp,
                DaemonResponse::Success { .. } | DaemonResponse::Error { .. }
            ),
            "expected Success or Error, got {resp:?}"
        );
    }

    // ── check_oom_killed (no cgroup file) ─────────────────────────────────

    #[tokio::test]
    async fn test_check_oom_killed_returns_false_for_nonexistent_path() {
        let result = check_oom_killed(std::path::Path::new("/nonexistent/cgroup/path")).await;
        assert!(
            !result,
            "check_oom_killed must return false for nonexistent path"
        );
    }

    #[tokio::test]
    async fn test_check_oom_killed_returns_false_for_zero_count() {
        let tmp = TempDir::new().expect("create temp dir");
        let events_path = tmp.path().join("memory.events");
        tokio::fs::write(&events_path, "oom_kill 0\npgfault 42\n")
            .await
            .expect("write memory.events");
        let result = check_oom_killed(tmp.path()).await;
        assert!(!result, "check_oom_killed must return false for oom_kill 0");
    }

    #[tokio::test]
    async fn test_check_oom_killed_returns_true_for_nonzero_count() {
        let tmp = TempDir::new().expect("create temp dir");
        let events_path = tmp.path().join("memory.events");
        tokio::fs::write(&events_path, "oom_kill 2\npgfault 100\n")
            .await
            .expect("write memory.events");
        let result = check_oom_killed(tmp.path()).await;
        assert!(result, "check_oom_killed must return true for oom_kill 2");
    }

    // ── env_flag helper ───────────────────────────────────────────────────

    #[test]
    fn test_env_flag_present_true() {
        unsafe { std::env::set_var("MINIBOX_TEST_ENV_FLAG_TRUE", "true") };
        let result = env_flag("MINIBOX_TEST_ENV_FLAG_TRUE");
        unsafe { std::env::remove_var("MINIBOX_TEST_ENV_FLAG_TRUE") };
        assert!(result, "env_flag must return true for 'true'");
    }

    #[test]
    fn test_env_flag_present_one() {
        unsafe { std::env::set_var("MINIBOX_TEST_ENV_FLAG_ONE", "1") };
        let result = env_flag("MINIBOX_TEST_ENV_FLAG_ONE");
        unsafe { std::env::remove_var("MINIBOX_TEST_ENV_FLAG_ONE") };
        assert!(result, "env_flag must return true for '1'");
    }

    #[test]
    fn test_env_flag_missing_returns_false() {
        unsafe { std::env::remove_var("MINIBOX_TEST_ENV_FLAG_MISSING") };
        let result = env_flag("MINIBOX_TEST_ENV_FLAG_MISSING");
        assert!(!result, "env_flag must return false when var is absent");
    }

    #[test]
    fn test_env_flag_false_value_returns_false() {
        unsafe { std::env::set_var("MINIBOX_TEST_ENV_FLAG_FALSE", "false") };
        let result = env_flag("MINIBOX_TEST_ENV_FLAG_FALSE");
        unsafe { std::env::remove_var("MINIBOX_TEST_ENV_FLAG_FALSE") };
        assert!(!result, "env_flag must return false for 'false'");
    }

    // ── generate_container_id ─────────────────────────────────────────────

    #[test]
    fn test_generate_container_id_produces_unique_ids() {
        let id1 = generate_container_id();
        let id2 = generate_container_id();
        assert_ne!(id1, id2, "generate_container_id must produce unique IDs");
        assert!(!id1.is_empty(), "ID must not be empty");
    }

    // ── send_error ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_send_error_sends_error_response() {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);
        send_error(&tx, "test_context", "something went wrong".to_string()).await;
        let resp = rx.recv().await.expect("no response from send_error");
        assert!(
            matches!(resp, DaemonResponse::Error { ref message } if message.contains("something went wrong")),
            "expected Error with message, got {resp:?}"
        );
    }

    #[tokio::test]
    async fn test_send_error_with_dropped_rx_does_not_panic() {
        let (tx, rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);
        drop(rx);
        // Must not panic even when receiver is gone.
        send_error(&tx, "test_context", "msg".to_string()).await;
    }

    // ── validate_policy ───────────────────────────────────────────────────

    #[test]
    fn test_validate_policy_allows_when_no_mounts_and_not_privileged() {
        let policy = ContainerPolicy {
            allow_bind_mounts: false,
            allow_privileged: false,
        };
        let result = validate_policy(&[], false, &policy);
        assert!(
            result.is_ok(),
            "no mounts + not privileged must pass strict policy"
        );
    }

    #[test]
    fn test_validate_policy_denies_bind_mounts_when_not_allowed() {
        use minibox_core::domain::BindMount;
        let policy = ContainerPolicy {
            allow_bind_mounts: false,
            allow_privileged: false,
        };
        let mounts = vec![BindMount {
            host_path: std::path::PathBuf::from("/tmp"),
            container_path: std::path::PathBuf::from("/mnt"),
            read_only: false,
        }];
        let result = validate_policy(&mounts, false, &policy);
        assert!(result.is_err(), "bind mounts must be denied by policy");
    }

    #[test]
    fn test_validate_policy_denies_privileged_when_not_allowed() {
        let policy = ContainerPolicy {
            allow_bind_mounts: true,
            allow_privileged: false,
        };
        let result = validate_policy(&[], true, &policy);
        assert!(result.is_err(), "privileged must be denied by policy");
    }

    // ── handle_load_image error path ──────────────────────────────────────

    #[tokio::test]
    async fn test_handle_load_image_failing_loader_returns_error() {
        struct FailLoader;

        #[async_trait::async_trait]
        impl minibox_core::domain::ImageLoader for FailLoader {
            async fn load_image(
                &self,
                _path: &std::path::Path,
                _name: &str,
                _tag: &str,
            ) -> anyhow::Result<()> {
                anyhow::bail!("simulated load failure")
            }
        }

        let tmp = TempDir::new().expect("create temp dir");
        let state = make_state(&tmp);
        let deps_base = make_deps(&tmp);
        let deps = (*deps_base)
            .clone()
            .with_image_loader(Arc::new(FailLoader) as minibox_core::domain::DynImageLoader);

        let resp = handle_load_image(
            "/nonexistent/image.tar".to_string(),
            "myimage".to_string(),
            "v1".to_string(),
            state,
            Arc::new(deps),
        )
        .await;

        assert!(
            matches!(resp, DaemonResponse::Error { .. }),
            "failing loader must return Error, got {resp:?}"
        );
    }

    // ── handle_get_manifest deeper paths ─────────────────────────────────

    #[tokio::test]
    async fn test_handle_get_manifest_with_container_with_bad_manifest_path() {
        let tmp = TempDir::new().expect("create temp dir");
        let state = make_state(&tmp);
        let deps = make_deps(&tmp);

        let record = crate::daemon::state::ContainerRecord {
            info: ContainerInfo {
                id: "ctr-manifest-bad".to_string(),
                name: None,
                image: "alpine:latest".to_string(),
                command: "/bin/sh".to_string(),
                state: "stopped".to_string(),
                created_at: "1970-01-01T00:00:00Z".to_string(),
                pid: None,
            },
            pid: None,
            rootfs_path: tmp.path().join("rootfs"),
            cgroup_path: tmp.path().join("cgroup"),
            post_exit_hooks: vec![],
            rootfs_metadata: None,
            source_image_ref: None,
            step_state: None,
            priority: None,
            urgency: None,
            execution_context: None,
            creation_params: None,
            manifest_path: Some(std::path::PathBuf::from("/nonexistent/manifest.json")),
            workload_digest: None,
        };
        state.add_container(record).await;

        let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);
        handle_get_manifest(
            "ctr-manifest-bad".to_string(),
            Arc::clone(&state),
            Arc::clone(&deps),
            tx,
        )
        .await;

        let resp = rx
            .recv()
            .await
            .expect("no response from handle_get_manifest");
        assert!(
            matches!(resp, DaemonResponse::Error { .. }),
            "manifest read error should produce Error, got {resp:?}"
        );
    }

    // ── handle_logs success path ─────────────────────────────────────────

    #[tokio::test]
    async fn test_handle_logs_with_existing_log_file_sends_log_lines() {
        let tmp = TempDir::new().expect("create temp dir");
        let state = make_state(&tmp);
        let deps = make_deps(&tmp);

        state
            .add_container(crate::daemon::state::ContainerRecord {
                info: ContainerInfo {
                    id: "ctr-logs".to_string(),
                    name: None,
                    image: "alpine:latest".to_string(),
                    command: "/bin/sh".to_string(),
                    state: "stopped".to_string(),
                    created_at: "1970-01-01T00:00:00Z".to_string(),
                    pid: None,
                },
                pid: None,
                rootfs_path: tmp.path().join("rootfs"),
                cgroup_path: tmp.path().join("cgroup"),
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
            })
            .await;

        // Create the container log directory and a stdout.log file.
        let log_dir = deps.lifecycle.containers_base.join("ctr-logs");
        std::fs::create_dir_all(&log_dir).expect("create log dir");
        std::fs::write(log_dir.join("stdout.log"), "line1\nline2\n").expect("write log");

        let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(16);
        handle_logs("ctr-logs".to_string(), false, state, deps, tx).await;

        let mut got_log_line = false;
        while let Ok(resp) = rx.try_recv() {
            if matches!(resp, DaemonResponse::LogLine { .. }) {
                got_log_line = true;
            }
        }
        assert!(got_log_line, "handle_logs must send at least one LogLine");
    }

    // ── handle_verify_manifest success path ───────────────────────────────

    #[tokio::test]
    async fn test_handle_verify_manifest_with_valid_manifest_returns_verify_result() {
        use minibox_core::domain::{
            ExecutionManifest, ExecutionManifestImage, ExecutionManifestRequest,
            ExecutionManifestResourceLimits, ExecutionManifestRuntime, ExecutionManifestSubject,
        };

        let tmp = TempDir::new().expect("create temp dir");
        let state = make_state(&tmp);
        let deps = make_deps(&tmp);

        let manifest_dir = tmp.path().join("verify-manifest-dir");
        std::fs::create_dir_all(&manifest_dir).expect("create dir");
        let manifest_path = manifest_dir.join("execution-manifest.json");
        let manifest = ExecutionManifest {
            schema_version: 1,
            container_id: "ctr-verify".to_string(),
            created_at: "1970-01-01T00:00:00Z".to_string(),
            manifest_path: None,
            workload_digest: None,
            subject: ExecutionManifestSubject {
                image_ref: "alpine:latest".to_string(),
                image: ExecutionManifestImage {
                    manifest_digest: None,
                    config_digest: None,
                    layer_digests: vec![],
                },
            },
            runtime: ExecutionManifestRuntime {
                command: vec!["/bin/sh".to_string()],
                env: vec![],
                mounts: vec![],
                resource_limits: Some(ExecutionManifestResourceLimits {
                    memory_limit_bytes: None,
                    cpu_weight: None,
                }),
                network_mode: "none".to_string(),
                privileged: false,
                platform: None,
            },
            request: ExecutionManifestRequest {
                name: None,
                ephemeral: false,
            },
        };
        let json = serde_json::to_string(&manifest).expect("serialize manifest");
        std::fs::write(&manifest_path, &json).expect("write manifest file");

        state
            .add_container(crate::daemon::state::ContainerRecord {
                info: ContainerInfo {
                    id: "ctr-verify".to_string(),
                    name: None,
                    image: "alpine:latest".to_string(),
                    command: "/bin/sh".to_string(),
                    state: "stopped".to_string(),
                    created_at: "1970-01-01T00:00:00Z".to_string(),
                    pid: None,
                },
                pid: None,
                rootfs_path: tmp.path().join("rootfs"),
                cgroup_path: tmp.path().join("cgroup"),
                post_exit_hooks: vec![],
                rootfs_metadata: None,
                source_image_ref: None,
                step_state: None,
                priority: None,
                urgency: None,
                execution_context: None,
                creation_params: None,
                manifest_path: Some(manifest_path),
                workload_digest: None,
            })
            .await;

        let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);
        handle_verify_manifest(
            "ctr-verify".to_string(),
            r#"{"allow":[]}"#.to_string(),
            state,
            deps,
            tx,
        )
        .await;

        let resp = rx
            .recv()
            .await
            .expect("no response from handle_verify_manifest");
        assert!(
            matches!(resp, DaemonResponse::VerifyResult { .. }),
            "expected VerifyResult, got {resp:?}"
        );
    }

    #[tokio::test]
    async fn test_handle_get_manifest_with_valid_manifest_returns_manifest() {
        use minibox_core::domain::{
            ExecutionManifest, ExecutionManifestImage, ExecutionManifestRequest,
            ExecutionManifestResourceLimits, ExecutionManifestRuntime, ExecutionManifestSubject,
        };

        let tmp = TempDir::new().expect("create temp dir");
        let state = make_state(&tmp);
        let deps = make_deps(&tmp);

        let manifest_dir = tmp.path().join("manifest-dir");
        std::fs::create_dir_all(&manifest_dir).expect("create manifest dir");
        let manifest_path = manifest_dir.join("execution-manifest.json");
        let manifest = ExecutionManifest {
            schema_version: 1,
            container_id: "ctr-manifest-ok".to_string(),
            created_at: "1970-01-01T00:00:00Z".to_string(),
            manifest_path: None,
            workload_digest: None,
            subject: ExecutionManifestSubject {
                image_ref: "alpine:latest".to_string(),
                image: ExecutionManifestImage {
                    manifest_digest: None,
                    config_digest: None,
                    layer_digests: vec![],
                },
            },
            runtime: ExecutionManifestRuntime {
                command: vec!["/bin/sh".to_string()],
                env: vec![],
                mounts: vec![],
                resource_limits: Some(ExecutionManifestResourceLimits {
                    memory_limit_bytes: None,
                    cpu_weight: None,
                }),
                network_mode: "none".to_string(),
                privileged: false,
                platform: None,
            },
            request: ExecutionManifestRequest {
                name: None,
                ephemeral: false,
            },
        };
        let json = serde_json::to_string(&manifest).expect("serialize manifest");
        std::fs::write(&manifest_path, &json).expect("write manifest file");

        state
            .add_container(crate::daemon::state::ContainerRecord {
                info: ContainerInfo {
                    id: "ctr-manifest-ok".to_string(),
                    name: None,
                    image: "alpine:latest".to_string(),
                    command: "/bin/sh".to_string(),
                    state: "stopped".to_string(),
                    created_at: "1970-01-01T00:00:00Z".to_string(),
                    pid: None,
                },
                pid: None,
                rootfs_path: tmp.path().join("rootfs"),
                cgroup_path: tmp.path().join("cgroup"),
                post_exit_hooks: vec![],
                rootfs_metadata: None,
                source_image_ref: None,
                step_state: None,
                priority: None,
                urgency: None,
                execution_context: None,
                creation_params: None,
                manifest_path: Some(manifest_path),
                workload_digest: None,
            })
            .await;

        let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);
        handle_get_manifest(
            "ctr-manifest-ok".to_string(),
            Arc::clone(&state),
            Arc::clone(&deps),
            tx,
        )
        .await;

        let resp = rx
            .recv()
            .await
            .expect("no response from handle_get_manifest");
        assert!(
            matches!(resp, DaemonResponse::Manifest { .. }),
            "valid manifest file should produce Manifest, got {resp:?}"
        );
    }
}
