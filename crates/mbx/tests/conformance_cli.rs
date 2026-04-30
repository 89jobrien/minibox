//! Conformance tests for the mbx CLI crate.
//!
//! Since mbx is a binary crate, these tests verify CLI behavior via
//! `assert_cmd` and protocol types from minibox-core.

use assert_cmd::Command;
use minibox_core::protocol::{DaemonRequest, DaemonResponse};
use predicates::prelude::*;

#[test]
fn conformance_cli_no_args_shows_help() {
    Command::cargo_bin("mbx")
        .unwrap()
        .assert()
        .failure()
        .stderr(predicate::str::contains("Usage"));
}

#[test]
fn conformance_cli_help_flag() {
    Command::cargo_bin("mbx")
        .unwrap()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("minibox"));
}

#[test]
fn conformance_cli_version_flag() {
    Command::cargo_bin("mbx")
        .unwrap()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("mbx"));
}

#[test]
fn conformance_cli_unknown_subcommand_fails() {
    Command::cargo_bin("mbx")
        .unwrap()
        .arg("nonexistent-command")
        .assert()
        .failure();
}

#[test]
fn conformance_protocol_request_variants_serialize() {
    // Verify key request variants round-trip through serde.
    let requests = vec![
        DaemonRequest::List,
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
fn conformance_protocol_response_variants_serialize() {
    let responses = vec![
        DaemonResponse::Success {
            message: "ok".to_string(),
        },
        DaemonResponse::Error {
            message: "fail".to_string(),
        },
        DaemonResponse::ContainerList { containers: vec![] },
    ];

    for resp in &responses {
        let json = serde_json::to_string(resp).expect("serialize response");
        let _: DaemonResponse = serde_json::from_str(&json).expect("deserialize response");
    }
}
