//! Persistent container state tracking.
//!
//! `DaemonState` is the single shared data structure held behind an
//! `Arc<DaemonState>`.  All mutable access is gated behind a tokio
//! `RwLock` so many readers can proceed concurrently while writes are
//! exclusive.
//!
//! State is persisted to a JSON file after every mutation so that
//! container records survive daemon restarts.

use minibox_core::domain::HookSpec;
use minibox_core::image::ImageStore;
use minibox_core::protocol::ContainerInfo;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::{RwLock, Semaphore};
use tracing::{debug, warn};

// ---------------------------------------------------------------------------
// StateRepository port (hexagonal architecture — persistence port)
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
    pub fn new(path: PathBuf) -> Self {
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
            .map_err(|e| anyhow::anyhow!("failed to serialise state: {}", e))?;

        let tmp_path = self.path.with_extension("json.tmp");
        std::fs::write(&tmp_path, &json).map_err(|e| {
            anyhow::anyhow!("failed to write state file {}: {}", tmp_path.display(), e)
        })?;

        // SECURITY: Restrict state file to owner-only.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let permissions = std::fs::Permissions::from_mode(0o600);
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

/// A complete record for a container tracked by the daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerRecord {
    /// Serialisable snapshot shared with the CLI.
    pub info: ContainerInfo,
    /// Host-namespace PID, or `None` if the process has not started yet
    /// or has exited.
    pub pid: Option<u32>,
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
    /// Path to the state file on disk.
    state_file: PathBuf,
    /// IP addresses currently allocated by bridge network, keyed by container_id.
    pub allocated_ips: Arc<RwLock<HashMap<String, std::net::IpAddr>>>,
}

impl DaemonState {
    /// Create a fresh `DaemonState` using the given image store.
    ///
    /// `data_dir` is the base directory where `state.json` will be written
    /// (e.g. `/var/lib/minibox`).
    pub fn new(image_store: ImageStore, data_dir: &Path) -> Self {
        Self {
            containers: Arc::new(RwLock::new(HashMap::new())),
            image_store: Arc::new(image_store),
            spawn_semaphore: Arc::new(Semaphore::new(MAX_CONCURRENT_SPAWNS)),
            state_file: data_dir.join(STATE_FILENAME),
            allocated_ips: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Create a `DaemonState` with an explicit [`StateRepository`] port.
    ///
    /// This constructor is the preferred way to inject the persistence
    /// dependency in tests and when the caller already holds a
    /// `Arc<dyn StateRepository>`.  The repository path is not used for
    /// the embedded `state_file` field — persistence goes entirely through
    /// the provided port.
    pub fn with_repository(image_store: ImageStore, _repository: Arc<dyn StateRepository>) -> Self {
        // The repository port is accepted here to satisfy the trait bound and
        // future wiring; the current internal save_to_disk/load_from_disk path
        // still uses the state_file field.  Full extraction is tracked as a
        // follow-on refactor.
        Self {
            containers: Arc::new(RwLock::new(HashMap::new())),
            image_store: Arc::new(image_store),
            spawn_semaphore: Arc::new(Semaphore::new(MAX_CONCURRENT_SPAWNS)),
            state_file: PathBuf::new(),
            allocated_ips: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Load previously persisted state from disk.
    ///
    /// Any containers that were "Running" when the daemon last exited are
    /// marked "Stopped" since the processes are no longer alive.
    ///
    /// Returns silently if the state file does not exist or is unreadable.
    pub async fn load_from_disk(&self) {
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

        let mut records: HashMap<String, ContainerRecord> = match serde_json::from_str(&data) {
            Ok(r) => r,
            Err(e) => {
                warn!(
                    "failed to parse state file {} (starting fresh): {}",
                    path.display(),
                    e
                );
                return;
            }
        };

        // Processes from the previous daemon session are gone.
        for record in records.values_mut() {
            if record.info.state == "Running"
                || record.info.state == "Created"
                || record.info.state == "Paused"
            {
                debug!(
                    "marking stale container {} as Stopped (was {})",
                    record.info.id, record.info.state
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

    /// Persist the current state to disk using an atomic write.
    ///
    /// Serialises the container map to pretty-printed JSON, writes it to a
    /// `.json.tmp` sibling file, then renames it over the target path.  The
    /// rename is atomic on POSIX filesystems, so readers never see a partially
    /// written file.  Failures are logged as warnings but do not propagate —
    /// state writes are best-effort and must not crash the daemon.
    async fn save_to_disk(&self) {
        let map = self.containers.read().await;
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
            let permissions = std::fs::Permissions::from_mode(0o600);
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
    /// the container process is forked. Use [`set_container_pid`] to transition
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

    /// Return `ContainerInfo` snapshots for every tracked container.
    ///
    /// The returned vec is a point-in-time snapshot; order is unspecified
    /// (HashMap iteration order).
    pub async fn list_containers(&self) -> Vec<ContainerInfo> {
        let map = self.containers.read().await;
        map.values().map(|r| r.info.clone()).collect()
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
            ("Created", ContainerState::Running)
            | ("Created", ContainerState::Failed)
            | ("Running", ContainerState::Stopped)
            | ("Running", ContainerState::Failed)
            | ("Paused", ContainerState::Stopped) => {
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
}

// ---------------------------------------------------------------------------
// ContainerStateAccess implementation
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
impl minibox::daemonbox_state::ContainerStateAccess for DaemonState {
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
            .map(|m| m.overlay_upper_dir().clone())
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
            rootfs_path: std::path::PathBuf::from("/tmp/fake-rootfs"),
            cgroup_path: std::path::PathBuf::from("/tmp/fake-cgroup"),
            post_exit_hooks: vec![],
            rootfs_metadata: None,
            source_image_ref: None,
        }
    }

    fn make_state_in(tmp: &TempDir) -> DaemonState {
        let image_store = ImageStore::new(tmp.path().join("images")).expect("ImageStore::new");
        DaemonState::new(image_store, tmp.path())
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

    /// Issue #134: containers that were "Running" when the daemon stopped must
    /// be marked "Stopped" on reload — their PIDs are gone and cannot be reattached.
    ///
    /// Guards: `load_from_disk` transitions Running/Created/Paused → Stopped
    /// and clears the `pid` field.
    #[tokio::test]
    async fn running_containers_marked_stopped_on_reload() {
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

        let containers = state2.list_containers().await;
        assert_eq!(containers.len(), 1);
        assert_eq!(
            containers[0].state, "Stopped",
            "Running containers must be marked Stopped on reload (process is gone)"
        );
        assert_eq!(
            containers[0].pid, None,
            "pid must be cleared on reload — process cannot be reattached"
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
}
