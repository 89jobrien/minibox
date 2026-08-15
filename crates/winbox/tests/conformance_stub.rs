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
//! Conformance tests for the winbox crate — stub validation, error types, paths.

use winbox::WinboxError;

#[test]
fn conformance_winbox_error_display_no_backend() {
    let err = WinboxError::NoBackendAvailable;
    let msg = format!("{err}");
    assert!(
        msg.contains("WSL2") || msg.contains("Windows Containers"),
        "error message should mention WSL2 or Windows Containers: {msg}"
    );
}

#[test]
fn conformance_winbox_error_is_debug() {
    let err = WinboxError::NoBackendAvailable;
    let _ = format!("{err:?}");
}

#[test]
fn conformance_start_returns_error() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(winbox::start());
    assert!(
        result.is_err(),
        "winbox::start() should return an error (stub)"
    );
}

#[test]
fn conformance_paths_pipe_name_non_empty() {
    let name = winbox::paths::pipe_name();
    assert!(
        !name.is_empty(),
        "pipe_name should return a non-empty string"
    );
}

#[test]
fn conformance_paths_pipe_name_has_prefix() {
    let name = winbox::paths::pipe_name();
    assert!(
        name.starts_with(r"\\.\pipe\"),
        "pipe_name should start with \\\\.\\pipe\\: {name}"
    );
}

#[test]
fn conformance_paths_data_dir_non_empty() {
    let p = winbox::paths::data_dir();
    assert!(!p.as_os_str().is_empty(), "data_dir should be non-empty");
}

#[test]
fn conformance_paths_run_dir_non_empty() {
    let p = winbox::paths::run_dir();
    assert!(!p.as_os_str().is_empty(), "run_dir should be non-empty");
}

#[test]
fn conformance_hcs_start_container_returns_error() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(winbox::hcs::start_container("alpine", &["/bin/sh"]));
    assert!(result.is_err(), "HCS stub should return error");
}

#[test]
fn conformance_wsl2_start_container_returns_error() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(winbox::wsl2::start_container("alpine", &["/bin/sh"]));
    assert!(result.is_err(), "WSL2 stub should return error");
}
