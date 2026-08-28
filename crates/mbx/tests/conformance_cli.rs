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
//! Conformance tests for the mbx CLI crate.
//!
//! Since mbx is a binary crate, these tests verify CLI behavior via
//! `assert_cmd` and protocol types from minibox-core.

use assert_cmd::Command;
use minibox_core::protocol::{DaemonRequest, DaemonResponse};
use predicates::prelude::*;

#[test]
fn conformance_mbx_cli_no_args_shows_help() {
    Command::cargo_bin("mbx")
        .unwrap()
        .assert()
        .failure()
        .stderr(predicate::str::contains("Usage"));
}

#[test]
fn conformance_mbx_cli_help_flag() {
    Command::cargo_bin("mbx")
        .unwrap()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("minibox"));
}

#[test]
fn conformance_mbx_cli_version_flag() {
    Command::cargo_bin("mbx")
        .unwrap()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("mbx"));
}

#[test]
fn conformance_mbx_cli_unknown_subcommand_fails() {
    Command::cargo_bin("mbx")
        .unwrap()
        .arg("nonexistent-command")
        .assert()
        .failure();
}

#[test]
fn conformance_daemon_request_variants_serialize() {
    // Verify key request variants round-trip through serde.
    let requests = vec![
        DaemonRequest::List,
        DaemonRequest::GetCapabilities,
        DaemonRequest::Pull {
            image: "alpine".to_string(),
            tag: Some("latest".to_string()),
            platform: None,
        },
        DaemonRequest::Stop {
            id: "abc123".to_string(),
        },
        DaemonRequest::Remove {
            id: "abc123".to_string(),
        },
    ];

    for req in &requests {
        let json = serde_json::to_string(req).expect("serialize request");
        let _: DaemonRequest = serde_json::from_str(&json).expect("deserialize request");
    }
}

#[test]
fn conformance_daemon_response_variants_serialize() {
    let responses = vec![
        DaemonResponse::Success {
            message: "ok".to_string(),
        },
        DaemonResponse::Error {
            message: "fail".to_string(),
        },
        DaemonResponse::ContainerList { containers: vec![] },
        DaemonResponse::CapabilityMatrix {
            matrix: minibox_core::domain::capability_matrix(),
        },
    ];

    for resp in &responses {
        let json = serde_json::to_string(resp).expect("serialize response");
        let _: DaemonResponse = serde_json::from_str(&json).expect("deserialize response");
    }
}
