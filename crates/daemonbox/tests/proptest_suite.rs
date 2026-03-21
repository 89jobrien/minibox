//! Property-based tests for daemonbox state invariants and handler input safety.

use std::path::Path;
use std::sync::Arc;

use daemonbox::state::{ContainerRecord, DaemonState};
use minibox_lib::{image::ImageStore, protocol::ContainerInfo};
use proptest::prelude::*;

// ── Helpers ──────────────────────────────────────────────────────────────────

fn make_rt() -> tokio::runtime::Runtime {
    tokio::runtime::Runtime::new().expect("tokio runtime")
}

fn make_state(tmp: &Path) -> Arc<DaemonState> {
    let image_store = ImageStore::new(tmp.join("images")).expect("ImageStore::new");
    Arc::new(DaemonState::new(image_store, tmp))
}

fn make_record(id: &str) -> ContainerRecord {
    ContainerRecord {
        info: ContainerInfo {
            id: id.to_string(),
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
    }
}

// ── Strategies ───────────────────────────────────────────────────────────────

fn arb_container_id() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9]{7,31}"
}

// ── DaemonState invariants ────────────────────────────────────────────────────

proptest! {
    #[test]
    fn state_add_then_get_finds_record(id in arb_container_id()) {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = make_state(tmp.path());
        let rt = make_rt();
        let record = make_record(&id);

        rt.block_on(state.add_container(record));
        let found = rt.block_on(state.get_container(&id));

        prop_assert!(found.is_some(), "get after add returned None for id={id}");
        prop_assert_eq!(found.unwrap().info.id, id);
    }
}

proptest! {
    #[test]
    fn state_remove_after_add_returns_none(id in arb_container_id()) {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = make_state(tmp.path());
        let rt = make_rt();

        rt.block_on(state.add_container(make_record(&id)));
        rt.block_on(state.remove_container(&id));
        let found = rt.block_on(state.get_container(&id));

        prop_assert!(found.is_none(), "get after add+remove returned Some for id={id}");
    }
}

proptest! {
    #[test]
    fn state_list_count_matches_adds(
        ids in proptest::collection::hash_set(arb_container_id(), 1..=8)
    ) {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = make_state(tmp.path());
        let rt = make_rt();

        for id in &ids {
            rt.block_on(state.add_container(make_record(id)));
        }
        let list = rt.block_on(state.list_containers());

        prop_assert_eq!(list.len(), ids.len(), "list count mismatch");
    }
}

proptest! {
    #[test]
    fn state_arbitrary_sequence_no_panic(
        adds in proptest::collection::hash_set(arb_container_id(), 1..=8),
        remove_count in 0_usize..=8_usize,
    ) {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = make_state(tmp.path());
        let rt = make_rt();

        for id in &adds {
            rt.block_on(state.add_container(make_record(id)));
        }

        let ids_vec: Vec<_> = adds.iter().collect();
        let to_remove = &ids_vec[..remove_count.min(ids_vec.len())];
        let mut removed = 0;
        for id in to_remove {
            if rt.block_on(state.remove_container(id)).is_some() {
                removed += 1;
            }
        }

        let list = rt.block_on(state.list_containers());
        prop_assert_eq!(list.len(), adds.len() - removed);
    }
}
