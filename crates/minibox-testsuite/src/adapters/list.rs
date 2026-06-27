//! Conformance tests for the `handle_list` contract.
//!
//! Tests exercise both `DaemonState::list_containers` (state layer) and the
//! `handle_list` handler directly, verifying that the correct
//! `DaemonResponse::ContainerList` is returned under all container-state
//! combinations.

use minibox::daemon::handler::handle_list;
use minibox::daemon::state::{ContainerRecord, DaemonState};
use minibox_core::domain::ContainerState;
use minibox_core::image::ImageStore;
use minibox_core::protocol::{ContainerInfo, DaemonResponse};
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;

use crate::harness::{ConformanceTest, TestCategory, TestContext, TestResult};

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Runtime::new().expect("build Tokio runtime")
}

fn make_state(tmp: &TempDir) -> DaemonState {
    let image_store = ImageStore::new(tmp.path().join("images")).expect("ImageStore::new");
    DaemonState::new(image_store, tmp.path())
}

fn make_arc_state(tmp: &TempDir) -> Arc<DaemonState> {
    Arc::new(make_state(tmp))
}

fn make_record(id: &str, image: &str, state_str: &str) -> ContainerRecord {
    ContainerRecord {
        info: ContainerInfo {
            id: id.to_string(),
            name: None,
            image: image.to_string(),
            command: "/bin/sh".to_string(),
            state: state_str.to_string(),
            created_at: "2026-04-27T00:00:00Z".to_string(),
            pid: None,
        },
        pid: None,
        rootfs_path: PathBuf::from("/tmp/rootfs"),
        cgroup_path: PathBuf::from("/tmp/cgroup"),
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
// Test structs
// ---------------------------------------------------------------------------

pub struct ListEmptyReturnsEmptyVec;
impl ConformanceTest for ListEmptyReturnsEmptyVec {
    fn name(&self) -> &'static str {
        "list_empty_returns_empty_vec"
    }
    fn adapter(&self) -> &'static str {
        "list"
    }
    fn category(&self) -> TestCategory {
        TestCategory::EdgeCase
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let tmp = TempDir::new().expect("TempDir::new");
        let state = make_state(&tmp);
        let containers = rt().block_on(state.list_containers());
        ctx.assert_eq(0, containers.len(), "empty daemon returns empty list");
        ctx.result()
    }
}

pub struct ListAfterRunShowsContainer;
impl ConformanceTest for ListAfterRunShowsContainer {
    fn name(&self) -> &'static str {
        "list_after_run_shows_container"
    }
    fn adapter(&self) -> &'static str {
        "list"
    }
    fn category(&self) -> TestCategory {
        TestCategory::Unit
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let tmp = TempDir::new().expect("TempDir::new");
        let state = make_state(&tmp);
        rt().block_on(state.add_container(make_record(
            "listarun01aabb1122",
            "alpine:latest",
            "Created",
        )));
        let containers = rt().block_on(state.list_containers());
        ctx.assert_eq(1, containers.len(), "list returns added container");
        if let Some(info) = containers.first() {
            ctx.assert_eq(
                "listarun01aabb1122".to_string(),
                info.id.clone(),
                "container id matches",
            );
        }
        ctx.result()
    }
}

pub struct ListShowsCorrectState;
impl ConformanceTest for ListShowsCorrectState {
    fn name(&self) -> &'static str {
        "list_shows_correct_state"
    }
    fn adapter(&self) -> &'static str {
        "list"
    }
    fn category(&self) -> TestCategory {
        TestCategory::Unit
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let tmp = TempDir::new().expect("TempDir::new");
        let state = make_state(&tmp);
        rt().block_on(state.add_container(make_record(
            "liststate01aabb11",
            "alpine:latest",
            "Created",
        )));
        let _ = rt()
            .block_on(state.update_container_state("liststate01aabb11", ContainerState::Running));
        let containers = rt().block_on(state.list_containers());
        ctx.assert_eq(1, containers.len(), "one container returned");
        if let Some(info) = containers.first() {
            ctx.assert_eq(
                "Running".to_string(),
                info.state.clone(),
                "state reflects Running after update",
            );
        }
        ctx.result()
    }
}

pub struct ListMultipleContainers;
impl ConformanceTest for ListMultipleContainers {
    fn name(&self) -> &'static str {
        "list_multiple_containers"
    }
    fn adapter(&self) -> &'static str {
        "list"
    }
    fn category(&self) -> TestCategory {
        TestCategory::Unit
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let tmp = TempDir::new().expect("TempDir::new");
        let state = make_state(&tmp);
        rt().block_on(state.add_container(make_record(
            "listmulti01aabb11",
            "alpine:latest",
            "Created",
        )));
        rt().block_on(state.add_container(make_record(
            "listmulti02aabb11",
            "ubuntu:22.04",
            "Created",
        )));
        let containers = rt().block_on(state.list_containers());
        ctx.assert_eq(2, containers.len(), "list returns all 2 containers");
        let ids: Vec<&str> = containers.iter().map(|c| c.id.as_str()).collect();
        ctx.assert_true(
            ids.contains(&"listmulti01aabb11"),
            "first container present",
        );
        ctx.assert_true(
            ids.contains(&"listmulti02aabb11"),
            "second container present",
        );
        ctx.result()
    }
}

pub struct ListAfterRemoveExcludesContainer;
impl ConformanceTest for ListAfterRemoveExcludesContainer {
    fn name(&self) -> &'static str {
        "list_after_remove_excludes_container"
    }
    fn adapter(&self) -> &'static str {
        "list"
    }
    fn category(&self) -> TestCategory {
        TestCategory::Unit
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let tmp = TempDir::new().expect("TempDir::new");
        let state = make_state(&tmp);
        rt().block_on(state.add_container(make_record(
            "listrm01aabb1122",
            "alpine:latest",
            "Created",
        )));
        rt().block_on(state.add_container(make_record(
            "listrm02aabb1122",
            "ubuntu:22.04",
            "Created",
        )));
        rt().block_on(state.remove_container("listrm01aabb1122"));
        let containers = rt().block_on(state.list_containers());
        ctx.assert_eq(1, containers.len(), "one container remains after remove");
        if let Some(info) = containers.first() {
            ctx.assert_eq(
                "listrm02aabb1122".to_string(),
                info.id.clone(),
                "remaining container is the one not removed",
            );
        }
        ctx.result()
    }
}

// ---------------------------------------------------------------------------
// handle_list handler conformance tests
// ---------------------------------------------------------------------------

/// `handle_list` on an empty daemon returns `ContainerList` with no entries.
pub struct HandleListEmptyReturnsContainerList;
impl ConformanceTest for HandleListEmptyReturnsContainerList {
    fn name(&self) -> &'static str {
        "handle_list_empty_returns_container_list"
    }
    fn adapter(&self) -> &'static str {
        "list"
    }
    fn category(&self) -> TestCategory {
        TestCategory::Unit
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let tmp = TempDir::new().expect("TempDir::new");
        let state = make_arc_state(&tmp);
        let resp = rt().block_on(handle_list(state));
        ctx.assert_true(
            matches!(resp, DaemonResponse::ContainerList { .. }),
            "handle_list returns ContainerList variant",
        );
        if let DaemonResponse::ContainerList { containers } = resp {
            ctx.assert_eq(0, containers.len(), "empty daemon has no containers");
        }
        ctx.result()
    }
}

/// `handle_list` with a single Running container returns it in the list.
pub struct HandleListWithRunningContainer;
impl ConformanceTest for HandleListWithRunningContainer {
    fn name(&self) -> &'static str {
        "handle_list_with_running_container"
    }
    fn adapter(&self) -> &'static str {
        "list"
    }
    fn category(&self) -> TestCategory {
        TestCategory::Unit
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let tmp = TempDir::new().expect("TempDir::new");
        let state = make_arc_state(&tmp);
        let id = "hlrunning01aabb1";
        rt().block_on(state.add_container(make_record(id, "alpine:latest", "Created")));
        let _ = rt().block_on(state.update_container_state(id, ContainerState::Running));

        let resp = rt().block_on(handle_list(state));
        ctx.assert_true(
            matches!(resp, DaemonResponse::ContainerList { .. }),
            "handle_list returns ContainerList",
        );
        if let DaemonResponse::ContainerList { containers } = resp {
            ctx.assert_eq(1, containers.len(), "one running container returned");
            if let Some(info) = containers.first() {
                ctx.assert_eq(id.to_string(), info.id.clone(), "container id matches");
                ctx.assert_eq(
                    "Running".to_string(),
                    info.state.clone(),
                    "container state is Running",
                );
            }
        }
        ctx.result()
    }
}

/// `handle_list` with a single Stopped container returns it in the list.
pub struct HandleListWithStoppedContainer;
impl ConformanceTest for HandleListWithStoppedContainer {
    fn name(&self) -> &'static str {
        "handle_list_with_stopped_container"
    }
    fn adapter(&self) -> &'static str {
        "list"
    }
    fn category(&self) -> TestCategory {
        TestCategory::Unit
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let tmp = TempDir::new().expect("TempDir::new");
        let state = make_arc_state(&tmp);
        let id = "hlstopped01aabb1";
        rt().block_on(state.add_container(make_record(id, "ubuntu:22.04", "Stopped")));

        let resp = rt().block_on(handle_list(state));
        ctx.assert_true(
            matches!(resp, DaemonResponse::ContainerList { .. }),
            "handle_list returns ContainerList",
        );
        if let DaemonResponse::ContainerList { containers } = resp {
            ctx.assert_eq(1, containers.len(), "one stopped container returned");
            if let Some(info) = containers.first() {
                ctx.assert_eq(id.to_string(), info.id.clone(), "container id matches");
                ctx.assert_eq(
                    "Stopped".to_string(),
                    info.state.clone(),
                    "container state is Stopped",
                );
            }
        }
        ctx.result()
    }
}

/// `handle_list` with mixed Running and Stopped containers returns all of them.
pub struct HandleListWithMixedStates;
impl ConformanceTest for HandleListWithMixedStates {
    fn name(&self) -> &'static str {
        "handle_list_with_mixed_states"
    }
    fn adapter(&self) -> &'static str {
        "list"
    }
    fn category(&self) -> TestCategory {
        TestCategory::Unit
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let tmp = TempDir::new().expect("TempDir::new");
        let state = make_arc_state(&tmp);

        let running_id = "hlmixed01running1";
        let stopped_id = "hlmixed02stopped1";

        rt().block_on(state.add_container(make_record(running_id, "alpine:latest", "Created")));
        let _ = rt().block_on(state.update_container_state(running_id, ContainerState::Running));

        rt().block_on(state.add_container(make_record(stopped_id, "ubuntu:22.04", "Stopped")));

        let resp = rt().block_on(handle_list(state));
        ctx.assert_true(
            matches!(resp, DaemonResponse::ContainerList { .. }),
            "handle_list returns ContainerList",
        );
        if let DaemonResponse::ContainerList { containers } = resp {
            ctx.assert_eq(2, containers.len(), "both containers returned");
            let ids: Vec<&str> = containers.iter().map(|c| c.id.as_str()).collect();
            ctx.assert_true(ids.contains(&running_id), "running container present");
            ctx.assert_true(ids.contains(&stopped_id), "stopped container present");

            let states: std::collections::HashMap<&str, &str> = containers
                .iter()
                .map(|c| (c.id.as_str(), c.state.as_str()))
                .collect();
            ctx.assert_eq(
                "Running",
                *states.get(running_id).unwrap_or(&""),
                "running container has Running state",
            );
            ctx.assert_eq(
                "Stopped",
                *states.get(stopped_id).unwrap_or(&""),
                "stopped container has Stopped state",
            );
        }
        ctx.result()
    }
}

/// Return all list conformance tests.
#[must_use]
pub fn all() -> Vec<Box<dyn ConformanceTest>> {
    vec![
        Box::new(ListEmptyReturnsEmptyVec),
        Box::new(ListAfterRunShowsContainer),
        Box::new(ListShowsCorrectState),
        Box::new(ListMultipleContainers),
        Box::new(ListAfterRemoveExcludesContainer),
        Box::new(HandleListEmptyReturnsContainerList),
        Box::new(HandleListWithRunningContainer),
        Box::new(HandleListWithStoppedContainer),
        Box::new(HandleListWithMixedStates),
    ]
}
