//! Persistent container state tracking.
//!
//! `DaemonState` is the single shared data structure held behind an
//! `Arc<DaemonState>`.  All mutable access is gated behind a tokio
//! `RwLock` so many readers can proceed concurrently while writes are
//!
//! exclusive.
//!
//! State is persisted to a JSON file after every mutation so that
//! container records survive daemon restarts.

use minibox_core::domain::{BindMount, HookSpec, NetworkMode};
use minibox_core::image::ImageStore;
use minibox_core::protocol::ContainerInfo;
use minibox_core::trace::TraceStore;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::{RwLock, Semaphore};
use tracing::{debug, info, warn};

// ---------------------------------------------------------------------------
// ProcessChecker port (process liveness port)
// ---------------------------------------------------------------------------

/// Port for checking whether a host PID is still alive.
///
/// The default adapter uses `kill(pid, 0)`.  Tests supply in-memory doubles.
pub trait ProcessChecker: Send + Sync {
    /// Returns `true` if the process with the given PID exists on the host.
    fn is_alive(&self, pid: u32) -> bool;
}

/// Default adapter: probes process existence via `kill(pid, 0)`.
#[cfg(unix)]
pub struct KillProcessChecker;

#[cfg(unix)]
impl ProcessChecker for KillProcessChecker {
    fn is_alive(&self, pid: u32) -> bool {
        // SAFETY: `kill(pid, 0)` sends no signal; it only checks existence.
        // A non-zero PID is always valid to probe.
        unsafe { nix::libc::kill(pid as nix::libc::pid_t, 0) == 0 }
    }
}

// ---------------------------------------------------------------------------
// CgroupFreezeChecker port (cgroup freezer inspection)
// ---------------------------------------------------------------------------

/// Port for checking whether a container's cgroup is frozen.
///
/// Decouples `reconcile_paused` from direct filesystem access so tests can
/// inject a mock without requiring a real cgroup hierarchy.
pub trait CgroupFreezeChecker: Send + Sync {
    /// Returns `true` if the cgroup at `cgroup_path` has `cgroup.freeze` set
    /// to `1` (frozen).
    fn is_frozen(&self, cgroup_path: &std::path::Path) -> bool;
}

/// Default adapter: reads `cgroup.freeze` from the filesystem.
pub struct FsCgroupFreezeChecker;

impl CgroupFreezeChecker for FsCgroupFreezeChecker {
    fn is_frozen(&self, cgroup_path: &std::path::Path) -> bool {
        let freeze_path = cgroup_path.join("cgroup.freeze");
        std::fs::read_to_string(&freeze_path).is_ok_and(|s| s.trim() == "1")
    }
}

// ---------------------------------------------------------------------------
// StateRepository port (persistence port)
// ---------------------------------------------------------------------------

/// Port for persisting and loading the container state map.
///
/// The primary adapter is [`JsonFileRepository`].  Tests may supply an
/// in-memory double.  `DaemonState` depends only on this trait.
pub trait StateRepository: Send + Sync + 'static {
    /// Load all persisted container records.
    ///
    /// Returns an empty map when no persisted state exists.
    fn load_containers(&self) -> anyhow::Result<HashMap<String, ContainerRecord>>;

    /// Persist the current container map.
    fn save_containers(&self, containers: &HashMap<String, ContainerRecord>) -> anyhow::Result<()>;
}

// ---------------------------------------------------------------------------
// JsonFileRepository — default adapter
// ---------------------------------------------------------------------------

/// Persists container state as pretty-printed JSON using an atomic rename.
///
/// Atomic rename ensures readers never see a partially-written file on POSIX
/// filesystems.  Permission `0o600` is applied to restrict state visibility
/// to the daemon owner.
pub struct JsonFileRepository {
    path: PathBuf,
}

impl JsonFileRepository {
    /// Create a new repository that reads/writes `path`.
    #[must_use]
    pub const fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

impl StateRepository for JsonFileRepository {
    fn load_containers(&self) -> anyhow::Result<HashMap<String, ContainerRecord>> {
        let data = match std::fs::read_to_string(&self.path) {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                debug!("no state file at {}, starting fresh", self.path.display());
                return Ok(HashMap::new());
            }
            Err(e) => {
                warn!("failed to read state file {}: {}", self.path.display(), e);
                return Ok(HashMap::new());
            }
        };

        let records: HashMap<String, ContainerRecord> =
            serde_json::from_str(&data).map_err(|e| {
                anyhow::anyhow!("failed to parse state file {}: {}", self.path.display(), e)
            })?;
        Ok(records)
    }

    fn save_containers(&self, containers: &HashMap<String, ContainerRecord>) -> anyhow::Result<()> {
        let json = serde_json::to_string_pretty(containers)
            .map_err(|e| anyhow::anyhow!("failed to serialise state: {e}"))?;

        let tmp_path = self.path.with_extension("json.tmp");
        std::fs::write(&tmp_path, &json).map_err(|e| {
            anyhow::anyhow!("failed to write state file {}: {}", tmp_path.display(), e)
        })?;

        // SECURITY: Restrict state file to owner-only.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            const OWNER_RW_PERMS: u32 = 0o600;
            let permissions = std::fs::Permissions::from_mode(OWNER_RW_PERMS);
            if let Err(e) = std::fs::set_permissions(&tmp_path, permissions) {
                warn!("failed to set state file permissions: {}", e);
            }
        }

        std::fs::rename(&tmp_path, &self.path).map_err(|e| {
            anyhow::anyhow!(
                "failed to rename {} → {}: {}",
                tmp_path.display(),
                self.path.display(),
                e
            )
        })?;
        Ok(())
    }
}

/// Typed container state for use with [`DaemonState::update_container_state`].
///
/// Re-exported from `minibox_core::domain` — use `minibox_core::domain::ContainerState`
/// directly in new code; this alias keeps existing call sites compiling unchanged.
pub use minibox_core::domain::ContainerState;

// SECURITY: Maximum concurrent container spawn operations to prevent fork bombs
const MAX_CONCURRENT_SPAWNS: usize = 100;

/// Default state file name within the data directory.
const STATE_FILENAME: &str = "state.json";

/// Snapshot of the `DaemonRequest::Run` parameters used to create a container.
///
/// Stored inside [`ContainerRecord`] so the daemon can replay or inspect the
/// original creation request (e.g. for container restart support).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunCreationParams {
    /// Source image name or reference.
    pub image: String,
    /// Optional image tag.
    pub tag: Option<String>,
    /// Command and arguments used at creation.
    pub command: Vec<String>,
    /// Optional memory limit in bytes.
    pub memory_limit_bytes: Option<u64>,
    /// Optional relative CPU scheduling weight.
    pub cpu_weight: Option<u64>,
    /// Requested network mode.
    pub network: Option<NetworkMode>,
    /// Environment variables in `KEY=VALUE` form.
    #[serde(default)]
    pub env: Vec<String>,
    /// Requested host bind mounts.
    #[serde(default)]
    pub mounts: Vec<BindMount>,
    /// Whether privileged execution was requested.
    #[serde(default)]
    pub privileged: bool,
    /// Optional human-readable container name.
    #[serde(default)]
    pub name: Option<String>,
    /// Whether a pseudo-terminal was requested.
    #[serde(default)]
    pub tty: bool,
    /// Optional entrypoint override.
    #[serde(default)]
    pub entrypoint: Option<String>,
    /// Optional container user override.
    #[serde(default)]
    pub user: Option<String>,
    /// Optional image platform override.
    #[serde(default)]
    pub platform: Option<String>,
    /// Optional parent cgroup path.
    #[serde(default)]
    pub cgroup_parent: Option<String>,
}

/// A complete record for a container tracked by the daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerRecord {
    /// Serialisable snapshot shared with the CLI.
    pub info: ContainerInfo,
    /// Host-namespace PID, or `None` if the process has not started yet
    /// or has exited.
    pub pid: Option<u32>,
    /// Adapter-managed handle for containers whose lifecycle isn't a plain
    /// host PID (e.g. a persistent smolvm/krun VM name). `None` for native
    /// containers and for ephemeral VM-backed runs that already completed.
    #[serde(default)]
    pub runtime_id: Option<String>,
    /// Path to the merged overlay directory used as the container rootfs.
    pub rootfs_path: PathBuf,
    /// Path to the container's cgroup directory.
    pub cgroup_path: PathBuf,
    /// Host-side commands to run after the container process exits.
    #[serde(default)]
    pub post_exit_hooks: Vec<HookSpec>,
    /// Typed backend metadata for the writable layer.
    /// `None` for adapters that don't expose an overlay filesystem (GKE, VZ).
    #[serde(default)]
    pub rootfs_metadata: Option<minibox_core::domain::BackendRootfsMetadata>,
    /// Image reference used to create this container (e.g. `"alpine:latest"`).
    #[serde(default)]
    pub source_image_ref: Option<String>,
    /// Host-visible writable-layer (overlay upper) directory for this
    /// container's rootfs, when the backend exposes one. Mirrors
    /// [`minibox_core::domain::BackendRootfsMetadata::overlay_upper_dir`] but is kept as a
    /// top-level field so callers don't need to match on `rootfs_metadata`
    /// to locate the writable layer. `None` for adapters without an
    /// overlay filesystem (GKE, VZ) or when not yet populated.
    #[serde(default)]
    pub upper_dir: Option<PathBuf>,
    /// Path to the merged/mounted rootfs presented to the container.
    /// Currently duplicates `rootfs_path` for backends that use an overlay
    /// mount; kept as a distinct field so future backends can report a
    /// merged view that differs from `rootfs_path`. `None` when not yet
    /// populated.
    #[serde(default)]
    pub merged_dir: Option<PathBuf>,
    /// Slashcrux step state — mirrors container lifecycle for agentic pipelines.
    #[serde(default)]
    pub step_state: Option<slashcrux::StepState>,
    /// Scheduling priority set at creation time.
    #[serde(default)]
    pub priority: Option<slashcrux::Priority>,
    /// Scheduling urgency set at creation time.
    #[serde(default)]
    pub urgency: Option<slashcrux::Urgency>,
    /// Execution context passed from the pipeline runner.
    #[serde(default)]
    pub execution_context: Option<slashcrux::ExecutionContext>,
    /// Original creation parameters, enabling container restart.
    #[serde(default)]
    pub creation_params: Option<RunCreationParams>,
    /// Path to the persisted execution manifest JSON file.
    #[serde(default)]
    pub manifest_path: Option<PathBuf>,
    /// Sealed workload digest from the execution manifest.
    #[serde(default)]
    pub workload_digest: Option<String>,
}

impl ContainerRecord {
    /// Returns the container's current state string (e.g. `"Running"`, `"Stopped"`).
    pub fn state_str(&self) -> &str {
        &self.info.state
    }
}

/// Shared daemon state, cheap to clone because it wraps `Arc`s internally.
#[derive(Clone)]
pub struct DaemonState {
    /// All containers known to the daemon.
    containers: Arc<RwLock<HashMap<String, ContainerRecord>>>,
    /// Image cache / pull facility.
    pub image_store: Arc<ImageStore>,
    /// SECURITY: Semaphore limiting concurrent container spawn operations
    pub spawn_semaphore: Arc<Semaphore>,
    /// Path to the state file on disk (used when no repository is injected).
    state_file: PathBuf,
    /// Injected persistence port.  When `Some`, all load/save operations
    /// go through this port instead of the raw `state_file` path.
    repository: Option<Arc<dyn StateRepository>>,
    /// IP addresses currently allocated by bridge network, keyed by `container_id`.
    pub allocated_ips: Arc<RwLock<HashMap<String, std::net::IpAddr>>>,
    /// Pipeline trace persistence adapter.
    pub trace_store: Arc<dyn TraceStore>,
}

impl DaemonState {
    /// Create a fresh `DaemonState` using the given image store.
    ///
    /// `data_dir` is the base directory where `state.json` will be written
    /// (e.g. `/var/lib/minibox`). A [`FileTraceStore`] is created under
    /// `data_dir/traces/` by default.
    ///
    /// [`FileTraceStore`]: minibox_core::trace::FileTraceStore
    #[must_use]
    pub fn new(image_store: ImageStore, data_dir: &Path) -> Self {
        let trace_store: Arc<dyn TraceStore> =
            minibox_core::trace::FileTraceStore::new(data_dir.join("traces")).map_or_else(
                |e| {
                    warn!("trace store: failed to create FileTraceStore: {e}, using noop");
                    Arc::new(minibox_core::trace::NoopTraceStore) as Arc<dyn TraceStore>
                },
                |s| Arc::new(s) as Arc<dyn TraceStore>,
            );

        Self {
            containers: Arc::new(RwLock::new(HashMap::new())),
            image_store: Arc::new(image_store),
            spawn_semaphore: Arc::new(Semaphore::new(MAX_CONCURRENT_SPAWNS)),
            state_file: data_dir.join(STATE_FILENAME),
            repository: None,
            allocated_ips: Arc::new(RwLock::new(HashMap::new())),
            trace_store,
        }
    }

    /// Create a `DaemonState` with an explicit [`StateRepository`] port.
    ///
    /// All `load_from_disk` and `save_to_disk` operations are delegated to
    /// `repository`.  The raw file-based path is not used when a repository
    /// is injected.  This is the preferred constructor for tests.
    pub fn with_repository(image_store: ImageStore, repository: Arc<dyn StateRepository>) -> Self {
        Self {
            containers: Arc::new(RwLock::new(HashMap::new())),
            image_store: Arc::new(image_store),
            spawn_semaphore: Arc::new(Semaphore::new(MAX_CONCURRENT_SPAWNS)),
            state_file: PathBuf::new(),
            repository: Some(repository),
            allocated_ips: Arc::new(RwLock::new(HashMap::new())),
            trace_store: Arc::new(minibox_core::trace::NoopTraceStore),
        }
    }

    /// Load previously persisted state from disk.
    ///
    /// Any containers that were "Running" when the daemon last exited are
    /// marked "Stopped" since the processes are no longer alive.
    ///
    /// Returns silently if the state file does not exist or is unreadable.
    pub async fn load_from_disk(&self) {
        let mut records: HashMap<String, ContainerRecord> = if let Some(repo) = &self.repository {
            match repo.load_containers() {
                Ok(r) => r,
                Err(e) => {
                    warn!(error = %e, "state: repository load failed, starting fresh");
                    return;
                }
            }
        } else {
            let path = &self.state_file;
            let data = match std::fs::read_to_string(path) {
                Ok(d) => d,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    debug!("no state file at {}, starting fresh", path.display());
                    return;
                }
                Err(e) => {
                    warn!("failed to read state file {}: {}", path.display(), e);
                    return;
                }
            };

            match serde_json::from_str(&data) {
                Ok(r) => r,
                Err(e) => {
                    warn!(
                        "failed to parse state file {} (starting fresh): {}",
                        path.display(),
                        e
                    );
                    return;
                }
            }
        };

        // Created containers from a previous session cannot be recovered —
        // mark them Stopped immediately.  Running and Paused containers are
        // left as-is so that `reconcile_on_startup` can probe their PIDs
        // (and cgroup.freeze for Paused) to distinguish truly orphaned
        // processes from those still alive.
        for record in records.values_mut() {
            if record.info.state == "Created" {
                debug!(
                    "marking stale container {} as Stopped (was Created)",
                    record.info.id
                );
                record.info.state = "Stopped".to_string();
                record.info.pid = None;
                record.pid = None;
            }
        }

        let count = records.len();
        *self.containers.write().await = records;
        debug!("loaded {} container records from disk", count);
    }

    /// Reconcile container state after loading from disk.
    ///
    /// For each container still marked `"Running"`, probe the host PID via the
    /// supplied [`ProcessChecker`].  If the process is gone, transition the
    /// record to `"Orphaned"` and clear the PID fields.
    ///
    /// For each container still marked `"Paused"`, verify the PID is alive and
    /// `cgroup.freeze` contains `1`.  If either check fails, mark `"Orphaned"`.
    ///
    /// Call this **after** [`Self::load_from_disk`] on daemon startup.
    pub async fn reconcile_on_startup(
        &self,
        checker: &dyn ProcessChecker,
        freeze_checker: &dyn CgroupFreezeChecker,
    ) {
        let mut map = self.containers.write().await;
        let mut orphaned_count: u32 = 0;

        for record in map.values_mut() {
            match record.info.state.as_str() {
                "Running" => {
                    reconcile_running(record, checker, &mut orphaned_count);
                }
                "Paused" => {
                    reconcile_paused(record, checker, freeze_checker, &mut orphaned_count);
                }
                _ => continue,
            }
        }

        drop(map);

        if orphaned_count > 0 {
            self.save_to_disk().await;
        }
    }

    /// Persist the current state to disk using an atomic write.
    ///
    /// When a [`StateRepository`] was injected via [`with_repository`], all
    /// writes go through `save_containers` on that port.  Otherwise the
    /// default file-based path is used: serialise to pretty-printed JSON,
    /// write to a `.json.tmp` sibling, then atomically rename.  Failures are
    /// logged as warnings but do not propagate — state writes are best-effort
    /// and must not crash the daemon.
    async fn save_to_disk(&self) {
        let map = self.containers.read().await;

        if let Some(repo) = &self.repository {
            if let Err(e) = repo.save_containers(&map) {
                warn!(error = %e, "state: repository save failed");
            }
            return;
        }

        let json = match serde_json::to_string_pretty(&*map) {
            Ok(j) => j,
            Err(e) => {
                warn!("failed to serialise state: {}", e);
                return;
            }
        };
        drop(map); // release lock before I/O

        let tmp_path = self.state_file.with_extension("json.tmp");
        if let Err(e) = std::fs::write(&tmp_path, &json) {
            warn!("failed to write state file {}: {}", tmp_path.display(), e);
            return;
        }
        // SECURITY: Restrict state file to owner-only. Contains PIDs and
        // rootfs paths that should not be world-readable.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            const OWNER_RW_PERMS: u32 = 0o600;
            let permissions = std::fs::Permissions::from_mode(OWNER_RW_PERMS);
            if let Err(e) = std::fs::set_permissions(&tmp_path, permissions) {
                warn!("failed to set state file permissions: {}", e);
            }
        }
        if let Err(e) = std::fs::rename(&tmp_path, &self.state_file) {
            warn!(
                "failed to rename {} → {}: {}",
                tmp_path.display(),
                self.state_file.display(),
                e
            );
        }
    }

    /// Register a new container record and persist state to disk.
    ///
    /// The caller is expected to create the record in `"Created"` state before
    /// the container process is forked. Use [`Self::set_container_pid`] to transition
    /// the record to `"Running"` once the PID is known.
    pub async fn add_container(&self, record: ContainerRecord) {
        debug!("adding container {}", record.info.id);
        let mut map = self.containers.write().await;
        map.insert(record.info.id.clone(), record);
        drop(map);
        self.save_to_disk().await;
    }

    /// Remove a container record from the in-memory map and persist the updated
    /// state to disk.
    ///
    /// Returns the removed record, or `None` if no container with `id` exists.
    /// Callers should ensure the container is in `"Stopped"` state before
    /// removing it; no state check is performed here.
    pub async fn remove_container(&self, id: &str) -> Option<ContainerRecord> {
        debug!("removing container {}", id);
        let mut map = self.containers.write().await;
        let removed = map.remove(id);
        drop(map);
        self.save_to_disk().await;
        removed
    }

    /// Look up a container by its ID and return a cloned snapshot.
    ///
    /// Returns `None` if no container with that ID is tracked. Because the
    /// return value is a clone, callers see the state at the moment of the call;
    /// concurrent mutations are not visible after the lock is released.
    pub async fn get_container(&self, id: &str) -> Option<ContainerRecord> {
        let map = self.containers.read().await;
        map.get(id).cloned()
    }

    /// Resolve a name-or-ID string to a container ID.
    ///
    /// First tries an exact ID match, then falls back to a name match.
    /// Returns `None` if no container with that ID or name exists.
    pub async fn resolve_id(&self, name_or_id: &str) -> Option<String> {
        let map = self.containers.read().await;
        // Exact ID match first.
        if map.contains_key(name_or_id) {
            return Some(name_or_id.to_string());
        }
        // Name match: find the first container whose info.name == Some(name_or_id).
        map.values()
            .find(|r| r.info.name.as_deref() == Some(name_or_id))
            .map(|r| r.info.id.clone())
    }

    /// Check whether a container name is already in use.
    pub async fn name_in_use(&self, name: &str) -> bool {
        let map = self.containers.read().await;
        map.values().any(|r| r.info.name.as_deref() == Some(name))
    }

    /// Return `ContainerInfo` snapshots for every tracked container, ordered
    /// by creation time (oldest first, ties broken by `id`).
    ///
    /// The returned vec is a point-in-time snapshot. `containers` is a
    /// `HashMap`, whose iteration order is unspecified and effectively
    /// randomized per-process (`RandomState` hasher) — callers that infer
    /// "the container that just appeared" from table position (e.g. `mbx
    /// ps`'s last row, or test harnesses polling for a freshly-created
    /// container) need a deterministic, creation-stable order rather than
    /// raw map iteration. `created_at` is an ISO 8601 string, so a plain
    /// lexicographic sort is also chronological.
    pub async fn list_containers(&self) -> Vec<ContainerInfo> {
        let map = self.containers.read().await;
        let mut containers: Vec<ContainerInfo> = map.values().map(|r| r.info.clone()).collect();
        containers.sort_by(|a, b| {
            a.created_at
                .cmp(&b.created_at)
                .then_with(|| a.id.cmp(&b.id))
        });
        containers
    }

    /// Change the `state` field of a container using the typed [`ContainerState`] enum.
    ///
    /// Enforces valid transitions:
    /// - `Running → Paused` (freeze)
    /// - `Paused → Running` (resume)
    /// - `Running → Stopped` / `Running → Failed` / `Created → Running`
    ///
    /// Returns an error if the transition is not permitted.
    pub async fn update_container_state(
        &self,
        id: &str,
        new_state: ContainerState,
    ) -> anyhow::Result<()> {
        let mut map = self.containers.write().await;
        let record = map
            .get_mut(id)
            .ok_or_else(|| anyhow::anyhow!("container {id} not found"))?;

        let current = record.info.state.as_str();
        match (current, new_state) {
            // Pause: Running → Paused
            ("Running", ContainerState::Paused) => {
                record.info.state = "Paused".to_string();
            }
            // Resume: Paused → Running
            ("Paused", ContainerState::Running) => {
                record.info.state = "Running".to_string();
            }
            // Standard forward transitions
            ("Created", ContainerState::Running | ContainerState::Failed)
            | ("Running" | "Paused", ContainerState::Stopped)
            | ("Running", ContainerState::Failed) => {
                if new_state == ContainerState::Stopped {
                    record.info.pid = None;
                    record.pid = None;
                }
                record.info.state = new_state.as_str().to_string();
            }
            _ => {
                anyhow::bail!(
                    "invalid transition: {} → {:?}",
                    record.info.state,
                    new_state
                );
            }
        }

        debug!(
            container_id = id,
            to = new_state.as_str(),
            "state: container state transition"
        );
        drop(map);
        self.save_to_disk().await;
        Ok(())
    }

    /// Update the manifest path and workload digest on a container record.
    ///
    /// Called after `prepare_run` persists the execution manifest to disk.
    pub async fn set_manifest_info(&self, id: &str, path: PathBuf, digest: String) {
        let mut map = self.containers.write().await;
        if let Some(record) = map.get_mut(id) {
            record.manifest_path = Some(path);
            record.workload_digest = Some(digest);
        }
        drop(map);
        self.save_to_disk().await;
    }

    /// Record the host-namespace PID after the container process is successfully
    /// forked and advance the container state from `"Created"` to `"Running"`.
    ///
    /// Both the `ContainerRecord.pid` field (used for signal delivery) and the
    /// `ContainerInfo.pid` field (returned to the CLI via `List`) are updated.
    pub async fn set_container_pid(&self, id: &str, pid: u32) {
        let mut map = self.containers.write().await;
        if let Some(record) = map.get_mut(id) {
            record.pid = Some(pid);
            record.info.pid = Some(pid);
            record.info.state = "Running".to_string();
        }
        drop(map);
        self.save_to_disk().await;
    }

    /// Record the adapter-managed runtime handle for a container (e.g. a
    /// persistent smolvm machine name), so later `Exec`/`Stop`/`Remove`
    /// requests can look it back up. No-op if the container isn't tracked.
    pub async fn set_container_runtime_id(&self, id: &str, runtime_id: Option<String>) {
        if runtime_id.is_none() {
            return;
        }
        let mut map = self.containers.write().await;
        if let Some(record) = map.get_mut(id) {
            record.runtime_id = runtime_id;
        }
        drop(map);
        self.save_to_disk().await;
    }
}

// ---------------------------------------------------------------------------
// Startup reconciliation helpers
// ---------------------------------------------------------------------------

/// Reconcile a container that was `"Running"` when the daemon last exited.
fn reconcile_running(
    record: &mut ContainerRecord,
    checker: &dyn ProcessChecker,
    orphaned_count: &mut u32,
) {
    let pid = if let Some(p) = record.pid {
        p
    } else {
        warn!(
            container_id = %record.info.id,
            stale_pid = 0_u32,
            "reconcile: container marked Running but has no PID — marking Orphaned"
        );
        record.info.state = "Orphaned".to_string();
        record.info.pid = None;
        *orphaned_count += 1;
        return;
    };

    if checker.is_alive(pid) {
        warn!(
            container_id = %record.info.id,
            pid = pid,
            "reconcile: container PID alive but unmonitored after restart — marking Orphaned"
        );
        record.info.state = "Orphaned".to_string();
        record.pid = None;
        *orphaned_count += 1;
    } else {
        warn!(
            container_id = %record.info.id,
            stale_pid = pid,
            "reconcile: stale container detected — PID gone, marking Orphaned"
        );
        record.info.state = "Orphaned".to_string();
        record.info.pid = None;
        record.pid = None;
        *orphaned_count += 1;
    }
}

/// Reconcile a container that was `"Paused"` when the daemon last exited.
///
/// A paused container is recoverable only if its PID is still alive and the
/// cgroup freezer is still engaged (`cgroup.freeze` contains `1`).  If either
/// condition fails, the container is marked `"Orphaned"`.
// NOTE: std::fs::read_to_string is intentionally synchronous here. This runs
// once at daemon startup while holding the write lock — no concurrent work is
// possible and the file is a single-digit-byte cgroup control file.
fn reconcile_paused(
    record: &mut ContainerRecord,
    checker: &dyn ProcessChecker,
    freeze_checker: &dyn CgroupFreezeChecker,
    orphaned_count: &mut u32,
) {
    let pid = if let Some(p) = record.pid {
        p
    } else {
        warn!(
            container_id = %record.info.id,
            "reconcile: container marked Paused but has no PID — marking Orphaned"
        );
        record.info.state = "Orphaned".to_string();
        record.info.pid = None;
        *orphaned_count += 1;
        return;
    };

    if !checker.is_alive(pid) {
        warn!(
            container_id = %record.info.id,
            stale_pid = pid,
            "reconcile: paused container PID gone — marking Orphaned"
        );
        record.info.state = "Orphaned".to_string();
        record.info.pid = None;
        record.pid = None;
        *orphaned_count += 1;
        return;
    }

    // PID alive — verify the cgroup freezer is still engaged.
    let frozen = freeze_checker.is_frozen(&record.cgroup_path);

    if frozen {
        info!(
            container_id = %record.info.id,
            pid = pid,
            "reconcile: paused container recovered — PID alive and cgroup frozen"
        );
        // Keep Paused state; clear daemon PID tracking (no reaper attached).
        record.pid = None;
    } else {
        warn!(
            container_id = %record.info.id,
            pid = pid,
            cgroup_path = %record.cgroup_path.display(),
            "reconcile: paused container cgroup not frozen — marking Orphaned"
        );
        record.info.state = "Orphaned".to_string();
        record.pid = None;
        *orphaned_count += 1;
    }
}

// ---------------------------------------------------------------------------
// ContainerStateAccess implementation
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
impl crate::container_state::ContainerStateAccess for DaemonState {
    async fn get_container_pid(&self, container_id: &str) -> anyhow::Result<u32> {
        let map = self.containers.read().await;
        let record = map
            .get(container_id)
            .ok_or_else(|| anyhow::anyhow!("container {container_id} not found"))?;
        record
            .pid
            .ok_or_else(|| anyhow::anyhow!("container {container_id} has no pid (not running)"))
    }

    async fn get_overlay_upper(&self, container_id: &str) -> anyhow::Result<std::path::PathBuf> {
        let map = self.containers.read().await;
        let record = map
            .get(container_id)
            .ok_or_else(|| anyhow::anyhow!("container {container_id} not found"))?;
        record
            .rootfs_metadata
            .as_ref()
            .map(|m| std::path::Path::to_path_buf(m.overlay_upper_dir()))
            .ok_or_else(|| anyhow::anyhow!("container {container_id} has no overlay upper dir"))
    }

    async fn get_source_image_ref(&self, container_id: &str) -> anyhow::Result<String> {
        let map = self.containers.read().await;
        let record = map
            .get(container_id)
            .ok_or_else(|| anyhow::anyhow!("container {container_id} not found"))?;
        Ok(record.source_image_ref.clone().unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use minibox_core::protocol::ContainerInfo;
    use tempfile::TempDir;

    fn make_test_record() -> ContainerRecord {
        make_record_with_name("test-container-id", None)
    }

    fn make_record_with_name(id: &str, name: Option<&str>) -> ContainerRecord {
        ContainerRecord {
            info: ContainerInfo {
                id: id.to_string(),
                name: name.map(|s| s.to_string()),
                image: "alpine:latest".to_string(),
                command: "/bin/sh".to_string(),
                state: "Created".to_string(),
                created_at: "2026-01-01T00:00:00Z".to_string(),
                pid: None,
            },
            pid: None,
            runtime_id: None,
            rootfs_path: std::path::PathBuf::from("/tmp/fake-rootfs"),
            cgroup_path: std::path::PathBuf::from("/tmp/fake-cgroup"),
            post_exit_hooks: vec![],
            rootfs_metadata: None,
            source_image_ref: None,
            upper_dir: None,
            merged_dir: None,
            step_state: None,
            priority: None,
            urgency: None,
            execution_context: None,
            creation_params: None,
            manifest_path: None,
            workload_digest: None,
        }
    }

    fn make_state_in(tmp: &TempDir) -> DaemonState {
        let image_store = ImageStore::new(tmp.path().join("images")).expect("ImageStore::new");
        DaemonState::new(image_store, tmp.path())
    }

    fn record_with_overlay(id: &str, upper: &str) -> ContainerRecord {
        let mut record = make_record_with_name(id, None);
        record.rootfs_metadata = Some(minibox_core::domain::BackendRootfsMetadata::Overlay {
            upper_dir: std::path::PathBuf::from(upper).into(),
            metadata: std::collections::HashMap::new(),
        });
        record.upper_dir = Some(std::path::PathBuf::from(upper));
        record
    }

    /// `get_overlay_upper` derives from `rootfs_metadata`; the persisted
    /// `upper_dir` field is written from the same source in
    /// `build_container_record`, so the two must agree (issue #80).
    #[tokio::test]
    async fn get_overlay_upper_agrees_with_record_upper_dir() {
        use crate::container_state::ContainerStateAccess as _;
        let tmp = TempDir::new().unwrap();
        let state = make_state_in(&tmp);
        let record = record_with_overlay("abc123", "/var/lib/minibox/containers/abc123/upper");
        let expected = record.upper_dir.clone().expect("record has upper_dir");
        state.add_container(record).await;
        assert_eq!(
            state
                .get_overlay_upper("abc123")
                .await
                .expect("overlay upper resolves"),
            expected
        );
    }

    #[tokio::test]
    async fn get_overlay_upper_errors_when_metadata_absent() {
        use crate::container_state::ContainerStateAccess as _;
        let tmp = TempDir::new().unwrap();
        let state = make_state_in(&tmp);
        state
            .add_container(make_record_with_name("abc123", None))
            .await;
        assert!(state.get_overlay_upper("abc123").await.is_err());
    }

    #[tokio::test]
    async fn resolve_id_finds_by_exact_id() {
        let tmp = TempDir::new().unwrap();
        let state = make_state_in(&tmp);
        state
            .add_container(make_record_with_name("abc123", None))
            .await;
        assert_eq!(state.resolve_id("abc123").await, Some("abc123".to_string()));
    }

    #[tokio::test]
    async fn resolve_id_finds_by_name() {
        let tmp = TempDir::new().unwrap();
        let state = make_state_in(&tmp);
        state
            .add_container(make_record_with_name("abc123", Some("my-container")))
            .await;
        assert_eq!(
            state.resolve_id("my-container").await,
            Some("abc123".to_string())
        );
    }

    #[tokio::test]
    async fn resolve_id_returns_none_for_unknown() {
        let tmp = TempDir::new().unwrap();
        let state = make_state_in(&tmp);
        assert_eq!(state.resolve_id("nonexistent").await, None);
    }

    #[tokio::test]
    async fn name_in_use_detects_duplicate() {
        let tmp = TempDir::new().unwrap();
        let state = make_state_in(&tmp);
        state
            .add_container(make_record_with_name("abc123", Some("web")))
            .await;
        assert!(state.name_in_use("web").await);
        assert!(!state.name_in_use("db").await);
    }

    #[tokio::test]
    async fn test_pause_resume_state_transitions() {
        let tmp = TempDir::new().unwrap();
        let state = make_state_in(&tmp);

        // Add a running container
        let mut record = make_test_record();
        record.info.state = "Running".to_string();
        state.add_container(record.clone()).await;
        let id = record.info.id.clone();

        // Pause it
        state
            .update_container_state(&id, ContainerState::Paused)
            .await
            .expect("pause transition");
        let c = state.get_container(&id).await.unwrap();
        assert_eq!(c.info.state, "Paused");

        // Resume it
        state
            .update_container_state(&id, ContainerState::Running)
            .await
            .expect("resume transition");
        let c = state.get_container(&id).await.unwrap();
        assert_eq!(c.info.state, "Running");
    }

    // ── Persistence semantics — Issue #134 ──────────────────────────────────

    /// Issue #134: container records must survive a daemon restart.
    ///
    /// After `save_to_disk` (triggered by `add_container`), a new `DaemonState`
    /// backed by the same directory must load the record via `load_from_disk`.
    ///
    /// Guards the documented contract in `docs/STATE_MODEL.md`.
    #[tokio::test]
    async fn container_records_survive_restart() {
        let tmp = TempDir::new().unwrap();

        // First "daemon session" — add a container and implicitly save.
        {
            let state = make_state_in(&tmp);
            let mut record = make_test_record();
            record.info.state = "Stopped".to_string();
            state.add_container(record).await;
        }

        // Second "daemon session" — load state from the same directory.
        let state2 = make_state_in(&tmp);
        state2.load_from_disk().await;

        let containers = state2.list_containers().await;
        assert_eq!(
            containers.len(),
            1,
            "container record must survive daemon restart"
        );
        assert_eq!(containers[0].id, "test-container-id");
    }

    /// Issue #134 (updated by #160): containers that were "Running" when the
    /// daemon stopped are marked "Orphaned" after `load_from_disk` +
    /// `reconcile_on_startup` with a checker that reports the PID as dead.
    #[tokio::test]
    async fn running_containers_marked_orphaned_on_reload() {
        let tmp = TempDir::new().unwrap();

        {
            let state = make_state_in(&tmp);
            let mut record = make_test_record();
            record.info.state = "Running".to_string();
            record.info.pid = Some(99999);
            record.pid = Some(99999);
            state.add_container(record).await;
        }

        let state2 = make_state_in(&tmp);
        state2.load_from_disk().await;
        state2
            .reconcile_on_startup(&NeverAliveChecker, &NeverFrozenChecker)
            .await;

        let containers = state2.list_containers().await;
        assert_eq!(containers.len(), 1);
        assert_eq!(
            containers[0].state, "Orphaned",
            "Running containers with dead PIDs must be Orphaned after reconcile"
        );
        assert_eq!(
            containers[0].pid, None,
            "pid must be cleared — process cannot be reattached"
        );
    }

    /// Issue #134: "Created" containers must also be marked "Stopped" on reload.
    #[tokio::test]
    async fn created_containers_marked_stopped_on_reload() {
        let tmp = TempDir::new().unwrap();

        {
            let state = make_state_in(&tmp);
            let mut record = make_test_record();
            record.info.state = "Created".to_string();
            state.add_container(record).await;
        }

        let state2 = make_state_in(&tmp);
        state2.load_from_disk().await;

        let containers = state2.list_containers().await;
        assert_eq!(containers.len(), 1);
        assert_eq!(
            containers[0].state, "Stopped",
            "Created containers must be marked Stopped on reload"
        );
    }

    // ── Orphaned state reconciliation — Issue #160 ──────────────────────

    /// Issue #160: `ContainerState::Orphaned` variant exists and can be
    /// represented as the string `"Orphaned"`.
    #[test]
    fn orphaned_state_has_correct_string_repr() {
        assert_eq!(ContainerState::Orphaned.as_str(), "Orphaned");
    }

    /// Issue #160: `reconcile_on_startup` marks Running containers whose PID
    /// is gone as Orphaned (not Stopped).
    #[tokio::test]
    async fn reconcile_marks_stale_running_as_orphaned() {
        let tmp = TempDir::new().expect("tempdir");

        // Session 1: persist a "Running" container with a non-existent PID.
        {
            let state = make_state_in(&tmp);
            let mut record = make_test_record();
            record.info.state = "Running".to_string();
            record.info.pid = Some(99999);
            record.pid = Some(99999);
            state.add_container(record).await;
        }

        // Session 2: load + reconcile with a checker that says "no such PID".
        let state2 = make_state_in(&tmp);
        state2.load_from_disk().await;
        state2
            .reconcile_on_startup(&NeverAliveChecker, &NeverFrozenChecker)
            .await;

        let containers = state2.list_containers().await;
        assert_eq!(containers.len(), 1);
        assert_eq!(
            containers[0].state, "Orphaned",
            "Running containers with dead PIDs must become Orphaned after reconcile"
        );
    }

    /// Issue #160: reconcile marks Running containers with a live PID as Orphaned.
    ///
    /// A PID that is alive after restart means the process is running without a
    /// daemon reaper.  The daemon cannot detect its eventual exit, so the honest
    /// state is Orphaned — not Running.
    #[tokio::test]
    async fn reconcile_marks_live_pid_as_orphaned() {
        let tmp = TempDir::new().expect("tempdir");

        {
            let state = make_state_in(&tmp);
            let mut record = make_test_record();
            record.info.state = "Running".to_string();
            record.info.pid = Some(std::process::id());
            record.pid = Some(std::process::id());
            state.add_container(record).await;
        }

        let state2 = make_state_in(&tmp);
        state2.load_from_disk().await;
        state2
            .reconcile_on_startup(&AlwaysAliveChecker, &NeverFrozenChecker)
            .await;

        let containers = state2.list_containers().await;
        assert_eq!(
            containers[0].state, "Orphaned",
            "Running containers with live-but-unmonitored PIDs must become Orphaned after reconcile"
        );
    }

    /// Issue #160: reconcile marks Running containers with a dead PID as Orphaned.
    #[tokio::test]
    async fn reconcile_marks_dead_pid_as_orphaned() {
        let tmp = TempDir::new().expect("tempdir");

        {
            let state = make_state_in(&tmp);
            let mut record = make_test_record();
            record.info.state = "Running".to_string();
            record.info.pid = Some(99999999);
            record.pid = Some(99999999);
            state.add_container(record).await;
        }

        let state2 = make_state_in(&tmp);
        state2.load_from_disk().await;
        state2
            .reconcile_on_startup(&NeverAliveChecker, &NeverFrozenChecker)
            .await;

        let containers = state2.list_containers().await;
        assert_eq!(
            containers[0].state, "Orphaned",
            "Running containers with dead PIDs must become Orphaned after reconcile"
        );
    }

    /// Issue #160: orphaned containers appear in `list_containers` (surfaced by `mbx ps`).
    #[tokio::test]
    async fn orphaned_containers_visible_in_list() {
        let tmp = TempDir::new().expect("tempdir");
        let state = make_state_in(&tmp);

        let mut record = make_test_record();
        record.info.state = "Orphaned".to_string();
        record.pid = None;
        state.add_container(record).await;

        let list = state.list_containers().await;
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].state, "Orphaned");
    }

    // ── Test doubles for ProcessChecker ──────────────────────────────────

    /// Always reports PIDs as dead.
    struct NeverAliveChecker;
    impl super::ProcessChecker for NeverAliveChecker {
        fn is_alive(&self, _pid: u32) -> bool {
            false
        }
    }

    /// Always reports PIDs as alive.
    struct AlwaysAliveChecker;
    impl super::ProcessChecker for AlwaysAliveChecker {
        fn is_alive(&self, _pid: u32) -> bool {
            true
        }
    }

    /// Always reports cgroups as not frozen (default for most tests).
    struct NeverFrozenChecker;
    impl super::CgroupFreezeChecker for NeverFrozenChecker {
        fn is_frozen(&self, _cgroup_path: &std::path::Path) -> bool {
            false
        }
    }

    /// Always reports cgroups as frozen.
    struct AlwaysFrozenChecker;
    impl super::CgroupFreezeChecker for AlwaysFrozenChecker {
        fn is_frozen(&self, _cgroup_path: &std::path::Path) -> bool {
            true
        }
    }

    /// Issue #134: "Stopped" containers must be preserved as-is on reload.
    #[tokio::test]
    async fn stopped_containers_preserved_on_reload() {
        let tmp = TempDir::new().unwrap();

        {
            let state = make_state_in(&tmp);
            let mut record = make_test_record();
            record.info.state = "Stopped".to_string();
            state.add_container(record).await;
        }

        let state2 = make_state_in(&tmp);
        state2.load_from_disk().await;

        let containers = state2.list_containers().await;
        assert_eq!(containers.len(), 1);
        assert_eq!(
            containers[0].state, "Stopped",
            "Stopped containers must remain Stopped — not double-reset"
        );
    }

    #[test]
    fn container_record_deserialize_without_creation_params() {
        let json = r#"{
            "info": {
                "id": "abc123",
                "name": null,
                "image": "alpine:latest",
                "command": "/bin/sh",
                "state": "Stopped",
                "created_at": "2026-01-01T00:00:00Z",
                "pid": null
            },
            "pid": null,
            "rootfs_path": "/tmp/rootfs",
            "cgroup_path": "/tmp/cgroup",
            "post_exit_hooks": [],
            "rootfs_metadata": null,
            "source_image_ref": null,
            "step_state": null,
            "priority": null,
            "urgency": null,
            "execution_context": null
        }"#;
        let record: ContainerRecord =
            serde_json::from_str(json).expect("must deserialize without creation_params");
        // Exercise ContainerRecord methods — the SUT struct under test.
        assert_eq!(record.info.id, "abc123");
        assert_eq!(record.info.image, "alpine:latest");
        assert_eq!(record.state_str(), "Stopped");
        assert!(
            record.creation_params.is_none(),
            "missing creation_params must deserialize as None"
        );
    }

    // ── StateRepository injection — Issue #315 ───────────────────────────────

    use std::sync::Mutex as StdMutex;

    /// In-memory StateRepository double that tracks call counts.
    struct SpyRepository {
        data: StdMutex<HashMap<String, ContainerRecord>>,
        save_count: StdMutex<u32>,
        load_count: StdMutex<u32>,
    }

    impl SpyRepository {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                data: StdMutex::new(HashMap::new()),
                save_count: StdMutex::new(0),
                load_count: StdMutex::new(0),
            })
        }

        fn save_count(&self) -> u32 {
            *self.save_count.lock().unwrap()
        }

        fn load_count(&self) -> u32 {
            *self.load_count.lock().unwrap()
        }
    }

    impl StateRepository for SpyRepository {
        fn load_containers(&self) -> anyhow::Result<HashMap<String, ContainerRecord>> {
            *self.load_count.lock().unwrap() += 1;
            Ok(self.data.lock().unwrap().clone())
        }

        fn save_containers(
            &self,
            containers: &HashMap<String, ContainerRecord>,
        ) -> anyhow::Result<()> {
            *self.save_count.lock().unwrap() += 1;
            *self.data.lock().unwrap() = containers.clone();
            Ok(())
        }
    }

    fn make_state_with_spy(spy: Arc<SpyRepository>) -> DaemonState {
        let image_store =
            ImageStore::new(std::env::temp_dir().join("spy-images")).expect("ImageStore::new");
        DaemonState::with_repository(image_store, spy as Arc<dyn StateRepository>)
    }

    /// Issue #315: injected repository save_containers is called on add_container.
    #[tokio::test]
    async fn with_repository_delegates_save_on_add_container() {
        let spy = SpyRepository::new();
        let state = make_state_with_spy(spy.clone());

        state.add_container(make_test_record()).await;

        assert_eq!(
            spy.save_count(),
            1,
            "save_containers must be called once after add_container"
        );
    }

    /// Issue #315: injected repository load_containers is called on load_from_disk.
    #[tokio::test]
    async fn with_repository_delegates_load_from_disk() {
        let spy = SpyRepository::new();
        let state = make_state_with_spy(spy.clone());

        state.load_from_disk().await;

        assert_eq!(
            spy.load_count(),
            1,
            "load_containers must be called once during load_from_disk"
        );
    }

    /// Issue #315: records saved through injected repository survive load_from_disk.
    #[tokio::test]
    async fn with_repository_roundtrips_container_records() {
        let spy = SpyRepository::new();

        {
            let state = make_state_with_spy(spy.clone());
            state.add_container(make_test_record()).await;
        }

        let state2 = make_state_with_spy(spy.clone());
        state2.load_from_disk().await;

        let containers = state2.list_containers().await;
        assert_eq!(
            containers.len(),
            1,
            "container record must be visible after round-trip through injected repository"
        );
        assert_eq!(containers[0].id, "test-container-id");
    }

    #[test]
    fn container_record_roundtrips_creation_params() {
        use minibox_core::domain::{BindMount, NetworkMode};
        let params = RunCreationParams {
            image: "alpine".to_string(),
            tag: Some("latest".to_string()),
            command: vec!["/bin/sh".to_string()],
            memory_limit_bytes: Some(134_217_728),
            cpu_weight: Some(512),
            network: Some(NetworkMode::Bridge),
            env: vec!["FOO=bar".to_string()],
            mounts: vec![BindMount {
                host_path: std::path::PathBuf::from("/tmp/host"),
                container_path: std::path::PathBuf::from("/tmp/guest"),
                read_only: false,
            }],
            privileged: false,
            name: Some("my-container".to_string()),
            tty: true,
            entrypoint: Some("/bin/bash".to_string()),
            user: Some("root".to_string()),
            platform: Some("linux/amd64".to_string()),
            cgroup_parent: None,
        };
        let mut record = make_test_record();
        record.creation_params = Some(params.clone());

        let json = serde_json::to_string(&record).expect("serialize");
        let back: ContainerRecord = serde_json::from_str(&json).expect("deserialize");
        let cp = back.creation_params.expect("creation_params must be Some");

        assert_eq!(cp.image, "alpine");
        assert_eq!(cp.tag, Some("latest".to_string()));
        assert_eq!(cp.command, vec!["/bin/sh"]);
        assert_eq!(cp.memory_limit_bytes, Some(134_217_728));
        assert_eq!(cp.cpu_weight, Some(512));
        assert_eq!(cp.network, Some(NetworkMode::Bridge));
        assert_eq!(cp.env, vec!["FOO=bar"]);
        assert_eq!(cp.mounts.len(), 1);
        assert_eq!(cp.mounts[0].host_path, std::path::Path::new("/tmp/host"));
        assert!(!cp.privileged);
        assert_eq!(cp.name, Some("my-container".to_string()));
        assert!(cp.tty);
        assert_eq!(cp.entrypoint, Some("/bin/bash".to_string()));
        assert_eq!(cp.user, Some("root".to_string()));
        assert_eq!(cp.platform, Some("linux/amd64".to_string()));
    }

    // ── Paused state persistence — Issue #263 ────────────────────────────

    /// Issue #263: `load_from_disk` preserves `"Paused"` state (no longer
    /// downgrades to `"Stopped"`).
    #[tokio::test]
    async fn paused_containers_preserved_on_load() {
        let tmp = TempDir::new().expect("tempdir");

        {
            let state = make_state_in(&tmp);
            let mut record = make_test_record();
            record.info.state = "Paused".to_string();
            record.info.pid = Some(12345);
            record.pid = Some(12345);
            state.add_container(record).await;
        }

        let state2 = make_state_in(&tmp);
        state2.load_from_disk().await;

        let containers = state2.list_containers().await;
        assert_eq!(containers.len(), 1);
        assert_eq!(
            containers[0].state, "Paused",
            "Paused containers must be preserved through load_from_disk"
        );
    }

    /// Issue #263: paused container with alive PID and frozen cgroup survives
    /// reconciliation.
    #[tokio::test]
    async fn reconcile_preserves_paused_with_frozen_cgroup() {
        let tmp = TempDir::new().expect("tempdir");
        let cgroup_dir = tmp.path().join("cgroup-paused");
        std::fs::create_dir_all(&cgroup_dir).expect("create cgroup dir");
        std::fs::write(cgroup_dir.join("cgroup.freeze"), "1\n").expect("write freeze");

        {
            let state = make_state_in(&tmp);
            let mut record = make_test_record();
            record.info.state = "Paused".to_string();
            record.info.pid = Some(12345);
            record.pid = Some(12345);
            record.cgroup_path = cgroup_dir.clone();
            state.add_container(record).await;
        }

        let state2 = make_state_in(&tmp);
        state2.load_from_disk().await;
        state2
            .reconcile_on_startup(&AlwaysAliveChecker, &AlwaysFrozenChecker)
            .await;

        let containers = state2.list_containers().await;
        assert_eq!(
            containers[0].state, "Paused",
            "Paused container with alive PID + frozen cgroup must stay Paused"
        );
    }

    /// Issue #263: paused container with dead PID becomes Orphaned.
    #[tokio::test]
    async fn reconcile_marks_paused_dead_pid_as_orphaned() {
        let tmp = TempDir::new().expect("tempdir");

        {
            let state = make_state_in(&tmp);
            let mut record = make_test_record();
            record.info.state = "Paused".to_string();
            record.info.pid = Some(99999);
            record.pid = Some(99999);
            state.add_container(record).await;
        }

        let state2 = make_state_in(&tmp);
        state2.load_from_disk().await;
        state2
            .reconcile_on_startup(&NeverAliveChecker, &NeverFrozenChecker)
            .await;

        let containers = state2.list_containers().await;
        assert_eq!(
            containers[0].state, "Orphaned",
            "Paused container with dead PID must become Orphaned"
        );
    }

    /// Issue #263: paused container with alive PID but unfrozen cgroup becomes
    /// Orphaned (inconsistent state — process running unmonitored).
    #[tokio::test]
    async fn reconcile_marks_paused_unfrozen_as_orphaned() {
        let tmp = TempDir::new().expect("tempdir");
        let cgroup_dir = tmp.path().join("cgroup-unfrozen");
        std::fs::create_dir_all(&cgroup_dir).expect("create cgroup dir");
        std::fs::write(cgroup_dir.join("cgroup.freeze"), "0\n").expect("write freeze");

        {
            let state = make_state_in(&tmp);
            let mut record = make_test_record();
            record.info.state = "Paused".to_string();
            record.info.pid = Some(12345);
            record.pid = Some(12345);
            record.cgroup_path = cgroup_dir.clone();
            state.add_container(record).await;
        }

        let state2 = make_state_in(&tmp);
        state2.load_from_disk().await;
        state2
            .reconcile_on_startup(&AlwaysAliveChecker, &NeverFrozenChecker)
            .await;

        let containers = state2.list_containers().await;
        assert_eq!(
            containers[0].state, "Orphaned",
            "Paused container with alive PID but unfrozen cgroup must become Orphaned"
        );
    }

    /// Issue #263: paused container with alive PID but missing cgroup.freeze
    /// file becomes Orphaned.
    #[tokio::test]
    async fn reconcile_marks_paused_missing_cgroup_as_orphaned() {
        let tmp = TempDir::new().expect("tempdir");

        {
            let state = make_state_in(&tmp);
            let mut record = make_test_record();
            record.info.state = "Paused".to_string();
            record.info.pid = Some(12345);
            record.pid = Some(12345);
            // cgroup_path points to a nonexistent directory
            record.cgroup_path = tmp.path().join("nonexistent-cgroup");
            state.add_container(record).await;
        }

        let state2 = make_state_in(&tmp);
        state2.load_from_disk().await;
        state2
            .reconcile_on_startup(&AlwaysAliveChecker, &NeverFrozenChecker)
            .await;

        let containers = state2.list_containers().await;
        assert_eq!(
            containers[0].state, "Orphaned",
            "Paused container with missing cgroup must become Orphaned"
        );
    }

    /// Issue #263: paused container with no PID becomes Orphaned.
    #[tokio::test]
    async fn reconcile_marks_paused_no_pid_as_orphaned() {
        let tmp = TempDir::new().expect("tempdir");

        {
            let state = make_state_in(&tmp);
            let mut record = make_test_record();
            record.info.state = "Paused".to_string();
            record.pid = None;
            state.add_container(record).await;
        }

        let state2 = make_state_in(&tmp);
        state2.load_from_disk().await;
        state2
            .reconcile_on_startup(&AlwaysAliveChecker, &NeverFrozenChecker)
            .await;

        let containers = state2.list_containers().await;
        assert_eq!(
            containers[0].state, "Orphaned",
            "Paused container with no PID must become Orphaned"
        );
    }
}
