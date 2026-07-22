//! Conformance tests for `DaemonState` persistence contract.

use minibox::daemon::state::{ContainerRecord, DaemonState};
use minibox_core::domain::ContainerState;
use minibox_core::image::ImageStore;
use minibox_core::protocol::ContainerInfo;
use std::path::PathBuf;
use tempfile::TempDir;

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Runtime::new().expect("build Tokio runtime")
}

fn make_state(tmp: &TempDir) -> DaemonState {
    let image_store = ImageStore::new(tmp.path().join("images")).expect("ImageStore::new");
    DaemonState::new(image_store, tmp.path())
}

fn make_record(id: &str, name: Option<&str>, image: &str) -> ContainerRecord {
    ContainerRecord {
        info: ContainerInfo {
            id: id.to_string(),
            name: name.map(std::string::ToString::to_string),
            image: image.to_string(),
            command: "/bin/sh".to_string(),
            state: "Created".to_string(),
            created_at: "2026-04-27T00:00:00Z".to_string(),
            pid: None,
        },
        pid: None,
        rootfs_path: PathBuf::from("/tmp/rootfs"),
        cgroup_path: PathBuf::from("/tmp/cgroup"),
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

crate::conformance_test! {
    name: "add_then_get_round_trip",
    adapter: "state",
    category: Unit,
    |ctx| {
        let tmp = TempDir::new().unwrap();
        let state = make_state(&tmp);
        let record = make_record("aabbccdd11223344", None, "alpine:latest");
        rt().block_on(state.add_container(record));
        let retrieved = rt().block_on(state.get_container("aabbccdd11223344"));
        ctx.assert_true(retrieved.is_some(), "get returns added container");
        if let Some(r) = retrieved {
            ctx.assert_eq("alpine:latest".to_string(), r.info.image, "image preserved");
        }
        ctx.result()
    }
}

crate::conformance_test! {
    name: "remove_returns_record",
    adapter: "state",
    category: Unit,
    |ctx| {
        let tmp = TempDir::new().unwrap();
        let state = make_state(&tmp);
        rt().block_on(state.add_container(make_record("rmtest001122334455", None, "alpine")));
        let removed = rt().block_on(state.remove_container("rmtest001122334455"));
        ctx.assert_true(removed.is_some(), "remove returns the removed record");
        ctx.result()
    }
}

crate::conformance_test! {
    name: "remove_nonexistent_returns_none",
    adapter: "state",
    category: EdgeCase,
    |ctx| {
        let tmp = TempDir::new().unwrap();
        let state = make_state(&tmp);
        let result = rt().block_on(state.remove_container("doesnotexist"));
        ctx.assert_true(result.is_none(), "remove nonexistent returns None");
        ctx.result()
    }
}

crate::conformance_test! {
    name: "list_containers_returns_all",
    adapter: "state",
    category: Unit,
    |ctx| {
        let tmp = TempDir::new().unwrap();
        let state = make_state(&tmp);
        rt().block_on(state.add_container(make_record("list01aabbccdd1122", None, "alpine")));
        rt().block_on(state.add_container(make_record("list02aabbccdd1122", None, "ubuntu")));
        let list = rt().block_on(state.list_containers());
        ctx.assert_eq(2, list.len(), "list returns both containers");
        ctx.result()
    }
}

crate::conformance_test! {
    name: "update_state_changes_status",
    adapter: "state",
    category: Unit,
    |ctx| {
        let tmp = TempDir::new().unwrap();
        let state = make_state(&tmp);
        rt().block_on(state.add_container(make_record("stateupd01aabbcc11", None, "alpine")));
        let _ = rt()
            .block_on(state.update_container_state("stateupd01aabbcc11", ContainerState::Running));
        let record = rt().block_on(state.get_container("stateupd01aabbcc11"));
        ctx.assert_true(record.is_some(), "container still exists after update");
        if let Some(r) = record {
            ctx.assert_eq(
                "Running".to_string(),
                r.info.state,
                "state updated to Running",
            );
        }
        ctx.result()
    }
}

crate::conformance_test! {
    name: "persistence_round_trip",
    adapter: "state",
    category: Integration,
    |ctx| {
        let tmp = TempDir::new().unwrap();
        {
            let state = make_state(&tmp);
            rt().block_on(state.add_container(make_record("persist01aabbcc11", None, "alpine")));
        }
        // Reload from disk in a new DaemonState instance (must call load_from_disk explicitly).
        let state2 = make_state(&tmp);
        rt().block_on(state2.load_from_disk());
        let record = rt().block_on(state2.get_container("persist01aabbcc11"));
        ctx.assert_true(record.is_some(), "container survives disk round-trip");
        ctx.result()
    }
}

crate::conformance_test! {
    name: "name_in_use_detects_collision",
    adapter: "state",
    category: EdgeCase,
    |ctx| {
        let tmp = TempDir::new().unwrap();
        let state = make_state(&tmp);
        rt().block_on(state.add_container(make_record(
            "namecol01aabbcc11",
            Some("myapp"),
            "alpine",
        )));
        ctx.assert_true(
            rt().block_on(state.name_in_use("myapp")),
            "name_in_use detects existing name",
        );
        ctx.assert_false(
            rt().block_on(state.name_in_use("otherapp")),
            "name_in_use returns false for unused name",
        );
        ctx.result()
    }
}
