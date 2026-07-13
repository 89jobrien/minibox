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
//! Property-based tests for miniboxd handler and state layer.
//!
//! Invariants that must hold for arbitrary inputs:
//! - `handle_list` never panics and always returns `ContainerList`.
//! - `handle_stop` / `handle_remove` on unknown IDs always return `Error`.
//! - `handle_pull` with a failing registry always returns `Error`.
//! - `DaemonState::list_containers` count matches the number of records added.
//!
//! No daemon process, no root, no network.

use minibox::testing::helpers::{
    make_mock_deps, make_mock_state, make_mock_state_with_n_containers,
};
use minibox::testing::mocks::MockRegistry;
use minibox_core::protocol::DaemonResponse;
use proptest::prelude::*;
use tempfile::TempDir;

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: None,
        cases: 64,
        ..ProptestConfig::default()
    })]

    /// `handle_list` must always return `ContainerList`, never panic.
    #[test]
    fn handle_list_always_returns_container_list(_unused in Just(())) {
        let rt = tokio::runtime::Runtime::new().expect("build tokio runtime");
        rt.block_on(async {
            let tmp = TempDir::new().expect("create temp dir");
            let state = make_mock_state(tmp.path());
            let response = miniboxd::handler::handle_list(state).await;
            prop_assert!(
                matches!(response, DaemonResponse::ContainerList { .. }),
                "expected ContainerList, got {response:?}"
            );
            Ok(())
        })?;
    }

    /// `handle_stop` with any arbitrary ID on empty state must return `Error`.
    #[test]
    fn handle_stop_arbitrary_id_returns_error(id in "[a-z0-9]{1,64}") {
        let rt = tokio::runtime::Runtime::new().expect("build tokio runtime");
        rt.block_on(async {
            let tmp = TempDir::new().expect("create temp dir");
            let state = make_mock_state(tmp.path());
            let deps = make_mock_deps(&tmp);
            let response = miniboxd::handler::handle_stop(id.clone(), state, deps).await;
            prop_assert!(
                matches!(response, DaemonResponse::Error { .. }),
                "stop of '{id}' must return Error, got {response:?}"
            );
            Ok(())
        })?;
    }

    /// `handle_remove` with any arbitrary ID on empty state must return `Error`.
    #[test]
    fn handle_remove_arbitrary_id_returns_error(id in "[a-z0-9]{1,64}") {
        let rt = tokio::runtime::Runtime::new().expect("build tokio runtime");
        rt.block_on(async {
            let tmp = TempDir::new().expect("create temp dir");
            let state = make_mock_state(tmp.path());
            let deps = make_mock_deps(&tmp);
            let response = miniboxd::handler::handle_remove(id.clone(), state, deps).await;
            prop_assert!(
                matches!(response, DaemonResponse::Error { .. }),
                "remove of '{id}' must return Error, got {response:?}"
            );
            Ok(())
        })?;
    }

    /// `handle_pull` with a failing registry always returns `Error`, for any
    /// image name.
    #[test]
    fn handle_pull_failing_registry_always_returns_error(
        image in "[a-z][a-z0-9_/-]{0,30}",
        tag   in "[a-z0-9._-]{1,20}",
    ) {
        let rt = tokio::runtime::Runtime::new().expect("build tokio runtime");
        rt.block_on(async {
            let tmp = TempDir::new().expect("create temp dir");
            let state = make_mock_state(tmp.path());
            let deps = minibox::testing::helpers::make_mock_deps_with_registry(
                MockRegistry::new().with_pull_failure(),
                &tmp,
            );
            let response =
                miniboxd::handler::handle_pull(image.clone(), Some(tag), None, state, deps).await;
            prop_assert!(
                matches!(response, DaemonResponse::Error { .. }),
                "pull of '{image}' with failing registry must return Error, got {response:?}"
            );
            Ok(())
        })?;
    }

    /// `list_containers` count equals the number of records added.
    #[test]
    fn list_count_matches_records_added(n in 0usize..=16usize) {
        let tmp = TempDir::new().expect("create temp dir");
        let state = make_mock_state_with_n_containers(tmp.path(), n);
        let rt = tokio::runtime::Runtime::new().expect("build tokio runtime");
        let count = rt.block_on(async { state.list_containers().await.len() });
        prop_assert_eq!(count, n, "list count {} != expected {}", count, n);
    }
}
