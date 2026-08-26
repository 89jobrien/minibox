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
//! Barrier-based race tests for `DaemonState` concurrent access.
//!
//! These tests use `tokio::sync::Barrier` to force simultaneous access from
//! multiple tasks, reliably exposing missing lock guards or TOCTOU issues.

use minibox::daemon::state::{ContainerRecord, DaemonState};
use minibox::image::ImageStore;
use minibox_core::protocol::ContainerInfo;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Barrier;

/// Build a minimal `ContainerRecord` for testing.
fn test_record(id: &str) -> ContainerRecord {
    ContainerRecord {
        info: ContainerInfo {
            id: id.to_string(),
            name: None,
            image: "test:latest".to_string(),
            command: "/bin/true".to_string(),
            state: "Created".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            pid: None,
        },
        pid: None,
        runtime_id: None,
        rootfs_path: PathBuf::from("/tmp/fake"),
        cgroup_path: PathBuf::from("/tmp/fake-cg"),
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

/// Create a `DaemonState` backed by a temp directory.
fn make_state(tmp: &tempfile::TempDir) -> DaemonState {
    let image_store =
        ImageStore::new(tmp.path().join("images")).expect("create ImageStore in test");
    DaemonState::new(image_store, tmp.path())
}

/// N tasks all insert distinct containers simultaneously, then all read.
/// After completion every inserted container must be present.
#[tokio::test]
async fn concurrent_insert_and_lookup() {
    let tmp = tempfile::TempDir::with_prefix("race-insert-").expect("tempdir");
    let state = Arc::new(make_state(&tmp));
    let n: usize = 20;
    let barrier = Arc::new(Barrier::new(n));

    // Phase 1: concurrent inserts
    let mut handles = Vec::with_capacity(n);
    for i in 0..n {
        let state = Arc::clone(&state);
        let barrier = Arc::clone(&barrier);
        handles.push(tokio::spawn(async move {
            let id = format!("ctr-{i:04}");
            barrier.wait().await;
            state.add_container(test_record(&id)).await;
            id
        }));
    }

    let mut ids = Vec::with_capacity(n);
    for h in handles {
        ids.push(h.await.expect("task panicked"));
    }

    // Phase 2: concurrent lookups -- every ID must resolve
    let barrier2 = Arc::new(Barrier::new(n));
    let mut lookup_handles = Vec::with_capacity(n);
    for id in &ids {
        let state = Arc::clone(&state);
        let barrier = Arc::clone(&barrier2);
        let id = id.clone();
        lookup_handles.push(tokio::spawn(async move {
            barrier.wait().await;
            state
                .get_container(&id)
                .await
                .unwrap_or_else(|| panic!("container {id} missing after concurrent insert"))
        }));
    }
    for h in lookup_handles {
        h.await.expect("lookup task panicked");
    }

    // Final consistency: list must contain exactly N containers
    let listed = state.list_containers().await;
    assert_eq!(
        listed.len(),
        n,
        "expected {n} containers after concurrent insert"
    );
}

/// A single writer updates container state while multiple readers list.
/// No reader should ever observe a partially-updated record.
#[tokio::test]
async fn concurrent_status_update_and_list() {
    let tmp = tempfile::TempDir::with_prefix("race-status-").expect("tempdir");
    let state = Arc::new(make_state(&tmp));

    // Pre-populate one container in Created state
    let id = "status-target".to_string();
    state.add_container(test_record(&id)).await;

    let readers = 10_usize;
    let barrier = Arc::new(Barrier::new(readers + 1)); // +1 for writer

    // Spawn readers that list containers after the barrier
    let mut reader_handles = Vec::with_capacity(readers);
    for _ in 0..readers {
        let state = Arc::clone(&state);
        let barrier = Arc::clone(&barrier);
        let id = id.clone();
        reader_handles.push(tokio::spawn(async move {
            barrier.wait().await;
            // Read multiple times to increase overlap window
            for _ in 0..50 {
                let list = state.list_containers().await;
                // Every entry must have a valid state string
                for info in &list {
                    if info.id == id {
                        let s = info.state.as_str();
                        assert!(s == "Created" || s == "Running", "unexpected state: {s}");
                    }
                }
                tokio::task::yield_now().await;
            }
        }));
    }

    // Writer: transition Created -> Running via set_container_pid
    {
        let state = Arc::clone(&state);
        let barrier = Arc::clone(&barrier);
        let id = id.clone();
        tokio::spawn(async move {
            barrier.wait().await;
            state.set_container_pid(&id, 12345).await;
        })
        .await
        .expect("writer task panicked");
    }

    for h in reader_handles {
        h.await.expect("reader task panicked");
    }

    // Final state must be Running
    let record = state.get_container(&id).await.expect("container missing");
    assert_eq!(record.info.state, "Running");
    assert_eq!(record.pid, Some(12345));
}

/// Two tasks attempt to remove the same container ID simultaneously.
/// Exactly one must succeed (return Some), the other must get None.
#[tokio::test]
async fn concurrent_double_remove() {
    let tmp = tempfile::TempDir::with_prefix("race-remove-").expect("tempdir");
    let state = Arc::new(make_state(&tmp));

    let id = "remove-me".to_string();
    state.add_container(test_record(&id)).await;

    let barrier = Arc::new(Barrier::new(2));

    let h1 = {
        let state = Arc::clone(&state);
        let barrier = Arc::clone(&barrier);
        let id = id.clone();
        tokio::spawn(async move {
            barrier.wait().await;
            state.remove_container(&id).await
        })
    };

    let h2 = {
        let state = Arc::clone(&state);
        let barrier = Arc::clone(&barrier);
        let id = id.clone();
        tokio::spawn(async move {
            barrier.wait().await;
            state.remove_container(&id).await
        })
    };

    let r1 = h1.await.expect("task 1 panicked");
    let r2 = h2.await.expect("task 2 panicked");

    // Exactly one remove should return the record
    let some_count = [&r1, &r2].iter().filter(|r| r.is_some()).count();
    assert_eq!(
        some_count,
        1,
        "expected exactly one successful remove, got {some_count} (r1={}, r2={})",
        r1.is_some(),
        r2.is_some()
    );

    // Container should be gone
    assert!(
        state.get_container(&id).await.is_none(),
        "container should not exist after removal"
    );
    assert!(
        state.list_containers().await.is_empty(),
        "list should be empty after removing only container"
    );
}
