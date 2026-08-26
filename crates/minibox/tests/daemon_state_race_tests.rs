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
//! Barrier-based concurrent stress tests for `DaemonState`.
//!
//! These tests use `tokio::sync::Barrier` to force simultaneous access from
//! multiple tasks, exposing missing lock guards or TOCTOU issues without
//! sleep-based synchronisation.

use minibox::daemon::state::{ContainerRecord, DaemonState, StateRepository};
use minibox_core::image::ImageStore;
use minibox_core::protocol::ContainerInfo;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;
use tokio::sync::Barrier;

// ---------------------------------------------------------------------------
// In-memory StateRepository double
// ---------------------------------------------------------------------------

struct NoopRepository;

impl StateRepository for NoopRepository {
    fn load_containers(&self) -> anyhow::Result<HashMap<String, ContainerRecord>> {
        Ok(HashMap::new())
    }
    fn save_containers(
        &self,
        _containers: &HashMap<String, ContainerRecord>,
    ) -> anyhow::Result<()> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_state(tmp: &TempDir) -> DaemonState {
    let image_store = ImageStore::new(tmp.path().join("images")).expect("ImageStore::new in test");
    let repo: Arc<dyn StateRepository> = Arc::new(NoopRepository);
    DaemonState::with_repository(image_store, repo)
}

fn make_record(id: &str) -> ContainerRecord {
    ContainerRecord {
        info: ContainerInfo {
            id: id.to_string(),
            name: None,
            image: "alpine:latest".to_string(),
            command: "/bin/sh".to_string(),
            state: "Created".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            pid: None,
        },
        pid: None,
        runtime_id: None,
        rootfs_path: PathBuf::from("/tmp/fake-rootfs"),
        cgroup_path: PathBuf::from("/tmp/fake-cgroup"),
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
// Test 1: Concurrent container insertion and lookup
// ---------------------------------------------------------------------------

/// N tasks all insert distinct containers behind a barrier, then all read
/// the full list and verify every container is present.
#[tokio::test]
async fn daemon_state_race_concurrent_insert_and_lookup() {
    const N: usize = 32;
    let tmp = TempDir::new().expect("TempDir in test");
    let state = make_state(&tmp);

    let insert_barrier = Arc::new(Barrier::new(N));
    let read_barrier = Arc::new(Barrier::new(N));

    let mut handles = Vec::with_capacity(N);
    for i in 0..N {
        let s = state.clone();
        let ib = Arc::clone(&insert_barrier);
        let rb = Arc::clone(&read_barrier);
        handles.push(tokio::spawn(async move {
            let id = format!("ctr-{i}");

            // All tasks wait here, then insert simultaneously.
            ib.wait().await;
            s.add_container(make_record(&id)).await;

            // All tasks wait here, then read simultaneously.
            rb.wait().await;
            let found = s.get_container(&id).await;
            assert!(
                found.is_some(),
                "container {id} must be visible after concurrent insert"
            );
        }));
    }

    for h in handles {
        h.await.expect("task must not panic");
    }

    let all = state.list_containers().await;
    assert_eq!(all.len(), N, "all {N} containers must be present");
}

// ---------------------------------------------------------------------------
// Test 2: Concurrent status update racing with multiple readers
// ---------------------------------------------------------------------------

/// One writer repeatedly transitions a container's state while multiple
/// readers concurrently list containers. No task should observe an invalid
/// or partially-written state string.
#[tokio::test]
async fn daemon_state_race_status_update_vs_list() {
    const READERS: usize = 8;
    const WRITER_ROUNDS: usize = 50;

    let tmp = TempDir::new().expect("TempDir in test");
    let state = make_state(&tmp);

    // Seed a container in Created state.
    let id = "race-ctr".to_string();
    state.add_container(make_record(&id)).await;

    // Barrier: writer + all readers start together.
    let barrier = Arc::new(Barrier::new(READERS + 1));

    // Writer task: Created -> Running -> Stopped, repeated.
    let writer_state = state.clone();
    let writer_barrier = Arc::clone(&barrier);
    let writer_id = id.clone();
    let writer = tokio::spawn(async move {
        writer_barrier.wait().await;
        for _ in 0..WRITER_ROUNDS {
            // Created -> Running
            let _ = writer_state
                .update_container_state(&writer_id, minibox::daemon::state::ContainerState::Running)
                .await;
            // Running -> Stopped
            let _ = writer_state
                .update_container_state(&writer_id, minibox::daemon::state::ContainerState::Stopped)
                .await;
            // Re-insert as Created for next round.
            writer_state.add_container(make_record(&writer_id)).await;
        }
    });

    // Reader tasks: list containers and check state strings are valid.
    let valid_states = ["Created", "Running", "Stopped"];
    let mut readers = Vec::with_capacity(READERS);
    for _ in 0..READERS {
        let s = state.clone();
        let b = Arc::clone(&barrier);
        let rid = id.clone();
        readers.push(tokio::spawn(async move {
            b.wait().await;
            for _ in 0..(WRITER_ROUNDS * 3) {
                if let Some(record) = s.get_container(&rid).await {
                    let st = record.info.state.as_str();
                    assert!(valid_states.contains(&st), "observed invalid state: {st}");
                }
                // No sleep -- tight loop.
            }
        }));
    }

    writer.await.expect("writer must not panic");
    for r in readers {
        r.await.expect("reader must not panic");
    }
}

// ---------------------------------------------------------------------------
// Test 3: Concurrent removal of the same container ID
// ---------------------------------------------------------------------------

/// Two tasks race to remove the same container. Exactly one must get
/// `Some(record)` back and the other must get `None`.
#[tokio::test]
async fn daemon_state_race_concurrent_removal() {
    const ROUNDS: usize = 50;
    let tmp = TempDir::new().expect("TempDir in test");
    let state = make_state(&tmp);

    for round in 0..ROUNDS {
        let id = format!("rm-{round}");
        state.add_container(make_record(&id)).await;

        let barrier = Arc::new(Barrier::new(2));
        let s1 = state.clone();
        let s2 = state.clone();
        let b1 = Arc::clone(&barrier);
        let b2 = Arc::clone(&barrier);
        let id1 = id.clone();
        let id2 = id.clone();

        let h1 = tokio::spawn(async move {
            b1.wait().await;
            s1.remove_container(&id1).await
        });
        let h2 = tokio::spawn(async move {
            b2.wait().await;
            s2.remove_container(&id2).await
        });

        let r1 = h1.await.expect("task 1 must not panic");
        let r2 = h2.await.expect("task 2 must not panic");

        let some_count = [&r1, &r2].iter().filter(|r| r.is_some()).count();
        assert_eq!(
            some_count, 1,
            "round {round}: exactly one remover must succeed, got {some_count} Some values"
        );

        // Container must be gone.
        assert!(
            state.get_container(&id).await.is_none(),
            "round {round}: container must be absent after removal"
        );
    }
}
