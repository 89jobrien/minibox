//! Property-based tests for daemonbox state invariants and handler input safety.

use std::path::Path;
use std::sync::{Arc, OnceLock};

use daemonbox::handler::{HandlerDependencies, handle_list, handle_remove, handle_stop};
use daemonbox::state::{ContainerRecord, DaemonState};
use mbx::adapters::mocks::{MockFilesystem, MockLimiter, MockNetwork, MockRegistry, MockRuntime};
use minibox_core::{image::ImageStore, protocol::ContainerInfo, protocol::DaemonResponse};
use proptest::prelude::*;

// ── Runtime ───────────────────────────────────────────────────────────────────

static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

fn runtime() -> &'static tokio::runtime::Runtime {
    RT.get_or_init(|| tokio::runtime::Runtime::new().expect("tokio runtime"))
}

// ── Shared TempDir ────────────────────────────────────────────────────────────
//
// A single TempDir shared across all proptest iterations avoids the overhead
// of creating and destroying a temporary directory for each of the 256+
// iterations proptest runs per property. DaemonState still writes state.json
// to disk on each add/remove, but the write overwrites the same file rather
// than creating a new directory subtree each time.

static TMPDIR: OnceLock<tempfile::TempDir> = OnceLock::new();

fn shared_tmp() -> &'static Path {
    TMPDIR
        .get_or_init(|| tempfile::TempDir::new().expect("shared proptest TempDir"))
        .path()
}

// ── Helpers ──────────────────────────────────────────────────────────────────

#[test]
fn runtime_is_shared_not_per_call() {
    let rt1 = runtime();
    let rt2 = runtime();
    assert!(
        std::ptr::eq(rt1, rt2),
        "runtime() must return the same instance"
    );
}

// Mock adapters always succeed. containers_base/run_containers_base are never
// created or accessed in "unknown ID" tests because handlers return early on
// ContainerNotFound before touching the filesystem.
struct NoopImageGc;

#[async_trait::async_trait]
impl minibox_core::image::gc::ImageGarbageCollector for NoopImageGc {
    async fn prune(
        &self,
        dry_run: bool,
        _in_use: &[String],
    ) -> anyhow::Result<minibox_core::image::gc::PruneReport> {
        Ok(minibox_core::image::gc::PruneReport {
            removed: vec![],
            freed_bytes: 0,
            dry_run,
        })
    }
}

fn make_deps(tmp: &Path) -> Arc<HandlerDependencies> {
    let image_store = Arc::new(minibox_core::image::ImageStore::new(tmp.join("images2")).unwrap());
    Arc::new(HandlerDependencies {
        registry: Arc::new(MockRegistry::new()),
        ghcr_registry: Arc::new(MockRegistry::new()),
        filesystem: Arc::new(MockFilesystem::new()),
        resource_limiter: Arc::new(MockLimiter::new()),
        runtime: Arc::new(MockRuntime::new()),
        network_provider: Arc::new(MockNetwork::new()),
        containers_base: tmp.join("containers"),
        run_containers_base: tmp.join("run"),
        metrics: Arc::new(daemonbox::telemetry::NoOpMetricsRecorder::new()),
        image_loader: Arc::new(daemonbox::handler::NoopImageLoader),
        exec_runtime: None,
        image_pusher: None,
        commit_adapter: None,
        image_builder: None,
        event_sink: Arc::new(minibox_core::events::NoopEventSink),
        event_source: Arc::new(minibox_core::events::BroadcastEventBroker::new()),
        image_gc: Arc::new(NoopImageGc),
        image_store,
        policy: daemonbox::handler::ContainerPolicy {
            allow_bind_mounts: true,
            allow_privileged: true,
        },
        pty_sessions: std::sync::Arc::new(tokio::sync::Mutex::new(
            daemonbox::handler::PtySessionRegistry::default(),
        )),
    })
}

// Each proptest iteration creates a fresh DaemonState pointing at the shared
// TempDir. save_to_disk still fires on add/remove but overwrites the same
// file, avoiding per-iteration tmpdir lifecycle costs.
fn make_state(tmp: &Path) -> Arc<DaemonState> {
    let image_store = ImageStore::new(tmp.join("images")).expect("ImageStore::new");
    Arc::new(DaemonState::new(image_store, tmp))
}

fn make_record(id: &str) -> ContainerRecord {
    ContainerRecord {
        info: ContainerInfo {
            id: id.to_string(),
            name: None,
            image: "test-image".into(),
            command: String::new(),
            state: "created".into(),
            created_at: "2026-01-01T00:00:00Z".into(),
            pid: None,
        },
        pid: None,
        rootfs_path: std::path::PathBuf::from("/tmp/fake-rootfs"),
        cgroup_path: std::path::PathBuf::from("/tmp/fake-cgroup"),
        post_exit_hooks: vec![],
        overlay_upper: None,
        source_image_ref: None,
    }
}

// ── Strategies ───────────────────────────────────────────────────────────────

fn arb_container_id() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9]{7,31}"
}

// ── DaemonState invariants ────────────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig { failure_persistence: None, ..ProptestConfig::default() })]

    #[test]
    fn state_add_then_get_finds_record(id in arb_container_id()) {
        let state = make_state(shared_tmp());
        let record = make_record(&id);

        runtime().block_on(state.add_container(record));
        let found = runtime().block_on(state.get_container(&id));

        prop_assert!(found.is_some(), "get after add returned None for id={id}");
        prop_assert_eq!(found.unwrap().info.id, id);
    }
}

proptest! {
    #![proptest_config(ProptestConfig { failure_persistence: None, ..ProptestConfig::default() })]

    #[test]
    fn state_remove_after_add_returns_none(id in arb_container_id()) {
        let state = make_state(shared_tmp());

        runtime().block_on(state.add_container(make_record(&id)));
        runtime().block_on(state.remove_container(&id));
        let found = runtime().block_on(state.get_container(&id));

        prop_assert!(found.is_none(), "get after add+remove returned Some for id={id}");
    }
}

proptest! {
    #![proptest_config(ProptestConfig { failure_persistence: None, ..ProptestConfig::default() })]

    #[test]
    fn state_list_count_matches_adds(
        ids in proptest::collection::hash_set(arb_container_id(), 1..=8)
    ) {
        let state = make_state(shared_tmp());

        for id in &ids {
            runtime().block_on(state.add_container(make_record(id)));
        }
        let list = runtime().block_on(state.list_containers());

        prop_assert_eq!(list.len(), ids.len(), "list count mismatch");
    }
}

proptest! {
    #![proptest_config(ProptestConfig { failure_persistence: None, ..ProptestConfig::default() })]

    #[test]
    fn state_add_remove_sequence_list_count_is_consistent(
        adds in proptest::collection::hash_set(arb_container_id(), 1..=8),
        remove_count in 0_usize..=8_usize,
    ) {
        let state = make_state(shared_tmp());

        for id in &adds {
            runtime().block_on(state.add_container(make_record(id)));
        }

        let ids_vec: Vec<_> = adds.iter().collect();
        let to_remove = &ids_vec[..remove_count.min(ids_vec.len())];
        let mut removed = 0;
        for id in to_remove {
            if runtime().block_on(state.remove_container(id)).is_some() {
                removed += 1;
            }
        }

        let list = runtime().block_on(state.list_containers());
        prop_assert_eq!(list.len(), adds.len() - removed);
    }
}

// ── Handler input safety ──────────────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig { failure_persistence: None, ..ProptestConfig::default() })]
    #[test]
    fn handle_stop_unknown_id_is_error(id in arb_container_id()) {
        let tmp = shared_tmp();
        let state = make_state(tmp);
        let deps = make_deps(tmp);
        let resp = runtime().block_on(handle_stop(id.clone(), state, deps));

        prop_assert!(
            matches!(resp, DaemonResponse::Error { .. }),
            "expected Error for unknown id={id}, got {resp:?}"
        );
    }
}

proptest! {
    #![proptest_config(ProptestConfig { failure_persistence: None, ..ProptestConfig::default() })]
    #[test]
    fn handle_remove_unknown_id_is_error(id in arb_container_id()) {
        let tmp = shared_tmp();
        let state = make_state(tmp);
        let deps = make_deps(tmp);

        let resp = runtime().block_on(handle_remove(id.clone(), state, deps));

        prop_assert!(
            matches!(resp, DaemonResponse::Error { .. }),
            "expected Error for unknown id={id}, got {resp:?}"
        );
    }
}

proptest! {
    #![proptest_config(ProptestConfig { failure_persistence: None, ..ProptestConfig::default() })]
    #[test]
    fn handle_list_always_returns_container_list(
        ids in proptest::collection::hash_set(arb_container_id(), 0..=5)
    ) {
        let state = make_state(shared_tmp());

        for id in &ids {
            runtime().block_on(state.add_container(make_record(id)));
        }
        let resp = runtime().block_on(handle_list(state));

        prop_assert!(
            matches!(resp, DaemonResponse::ContainerList { .. }),
            "expected ContainerList, got {resp:?}"
        );
    }
}
