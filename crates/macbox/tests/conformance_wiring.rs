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
//! Conformance tests for the macbox crate — Colima/krun adapter wiring,
//! error types, and macOS-specific path defaults.

use macbox::MacboxError;
use macbox::paths;

#[test]
fn conformance_macbox_error_display_no_backend() {
    let err = MacboxError::NoBackendAvailable;
    let msg = format!("{err}");
    assert!(
        msg.contains("Colima"),
        "error message should mention Colima: {msg}"
    );
}

#[test]
fn conformance_macbox_error_is_debug() {
    let err = MacboxError::NoBackendAvailable;
    let _ = format!("{err:?}");
}

#[test]
fn conformance_paths_data_dir_non_empty() {
    let p = paths::data_dir();
    assert!(
        !p.as_os_str().is_empty(),
        "data_dir should return a non-empty path"
    );
}

#[test]
fn conformance_paths_run_dir_non_empty() {
    let p = paths::run_dir();
    assert!(
        !p.as_os_str().is_empty(),
        "run_dir should return a non-empty path"
    );
}

#[test]
fn conformance_paths_socket_path_non_empty() {
    let p = paths::socket_path();
    assert!(
        !p.as_os_str().is_empty(),
        "socket_path should return a non-empty path"
    );
}

#[test]
fn conformance_paths_socket_ends_with_sock() {
    let p = paths::socket_path();
    let s = p.to_string_lossy();
    assert!(
        s.ends_with(".sock"),
        "socket_path should end with .sock: {s}"
    );
}

#[tokio::test]
async fn conformance_build_colima_handler_dependencies_succeeds() {
    use macbox::build_colima_handler_dependencies;
    use minibox::adapters::{LimaExecutor, LimaSpawner};
    use minibox::daemon::state::DaemonState;
    use minibox_core::image::ImageStore;
    use minibox_core::image::gc::{ImageGarbageCollector, ImageGc};
    use minibox_core::image::lease::DiskLeaseService;
    use std::sync::Arc;
    use tempfile::TempDir;

    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let containers_dir = data_dir.join("containers");
    let run_containers_dir = tmp.path().join("run").join("containers");
    std::fs::create_dir_all(&containers_dir).unwrap();
    std::fs::create_dir_all(&run_containers_dir).unwrap();

    let image_store = ImageStore::new(data_dir.join("images")).expect("image store");
    let state = Arc::new(DaemonState::new(image_store, &data_dir));
    let lease_service = Arc::new(
        DiskLeaseService::new(data_dir.join("leases.json"))
            .await
            .expect("lease service"),
    );
    let image_gc: Arc<dyn ImageGarbageCollector> =
        Arc::new(ImageGc::new(Arc::clone(&state.image_store), lease_service));

    let executor: LimaExecutor = Arc::new(|_args: &[&str]| Ok(String::new()));
    let spawner: LimaSpawner = Arc::new(|_args: &[&str]| Err(anyhow::anyhow!("spawner stub")));

    let deps = build_colima_handler_dependencies(
        Arc::clone(&state),
        data_dir,
        containers_dir,
        run_containers_dir,
        image_gc,
        executor,
        spawner,
    );

    assert!(
        deps.is_ok(),
        "build_colima_handler_dependencies should succeed"
    );
    let deps = deps.unwrap();
    assert!(deps.build.commit_adapter.is_some());
    assert!(deps.build.image_builder.is_some());
    assert!(deps.build.image_pusher.is_some());
}
