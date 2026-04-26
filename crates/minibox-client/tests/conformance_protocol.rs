//! Conformance tests for protocol serialization boundary.
//!
//! Tests the encode/decode roundtrips for all major `DaemonRequest` and
//! `DaemonResponse` variants using the framing helpers in
//! `minibox_core::protocol`.
//!
//! Verifies:
//! - `encode_request()` appends `\n`
//! - `encode_response()` appends `\n`
//! - `decode_request()` strips trailing `\n` (or works without it)
//! - `decode_response()` strips trailing `\n` (or works without it)
//! - Serde `#[serde(default)]` fields are properly deserialized
//! - All request and response types roundtrip without data loss

use minibox_core::domain::{BindMount, NetworkMode};
use minibox_core::protocol::{
    ContainerInfo, DaemonRequest, DaemonResponse, OutputStreamKind, decode_request,
    decode_response, encode_request, encode_response,
};
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// Request encoding/decoding tests
// ---------------------------------------------------------------------------

#[test]
fn conformance_request_run_roundtrip() {
    let req = DaemonRequest::Run {
        image: "ubuntu".to_string(),
        tag: Some("22.04".to_string()),
        command: vec![
            "/bin/bash".to_string(),
            "-c".to_string(),
            "echo test".to_string(),
        ],
        memory_limit_bytes: Some(1073741824), // 1GB
        cpu_weight: Some(1000),
        ephemeral: true,
        network: Some(NetworkMode::Bridge),
        env: vec!["FOO=bar".to_string(), "BAZ=qux".to_string()],
        mounts: vec![BindMount {
            host_path: PathBuf::from("/tmp/data"),
            container_path: PathBuf::from("/data"),
            read_only: false,
        }],
        privileged: true,
        name: Some("test-container".to_string()),
        tty: false,
    };

    let encoded = encode_request(&req).expect("encode failed");
    assert_eq!(
        encoded.last(),
        Some(&b'\n'),
        "encoded request must end with newline"
    );

    let decoded = decode_request(&encoded).expect("decode failed");

    match decoded {
        DaemonRequest::Run {
            image,
            tag,
            command,
            memory_limit_bytes,
            cpu_weight,
            ephemeral,
            network,
            env,
            mounts,
            privileged,
            name,
            tty,
        } => {
            assert_eq!(image, "ubuntu");
            assert_eq!(tag, Some("22.04".to_string()));
            assert_eq!(command, vec!["/bin/bash", "-c", "echo test"]);
            assert_eq!(memory_limit_bytes, Some(1073741824));
            assert_eq!(cpu_weight, Some(1000));
            assert!(ephemeral);
            assert_eq!(network, Some(NetworkMode::Bridge));
            assert_eq!(env, vec!["FOO=bar", "BAZ=qux"]);
            assert_eq!(mounts.len(), 1);
            assert_eq!(mounts[0].host_path, PathBuf::from("/tmp/data"));
            assert_eq!(mounts[0].container_path, PathBuf::from("/data"));
            assert!(!mounts[0].read_only);
            assert!(privileged);
            assert_eq!(name, Some("test-container".to_string()));
            assert!(!tty);
        }
        _ => panic!("wrong request type after decode"),
    }
}

#[test]
fn conformance_request_list_roundtrip() {
    let req = DaemonRequest::List;

    let encoded = encode_request(&req).expect("encode failed");
    assert_eq!(
        encoded.last(),
        Some(&b'\n'),
        "encoded request must end with newline"
    );

    let decoded = decode_request(&encoded).expect("decode failed");

    match decoded {
        DaemonRequest::List => {
            // Success if we get here
        }
        _ => panic!("wrong request type"),
    }
}

#[test]
fn conformance_request_pull_roundtrip() {
    let req = DaemonRequest::Pull {
        image: "library/nginx".to_string(),
        tag: Some("1.25-alpine".to_string()),
    };

    let encoded = encode_request(&req).expect("encode failed");
    assert_eq!(
        encoded.last(),
        Some(&b'\n'),
        "encoded request must end with newline"
    );

    let decoded = decode_request(&encoded).expect("decode failed");

    match decoded {
        DaemonRequest::Pull { image, tag } => {
            assert_eq!(image, "library/nginx");
            assert_eq!(tag, Some("1.25-alpine".to_string()));
        }
        _ => panic!("wrong request type"),
    }
}

#[test]
fn conformance_request_stop_roundtrip() {
    let req = DaemonRequest::Stop {
        id: "abc123def456".to_string(),
    };

    let encoded = encode_request(&req).expect("encode failed");
    assert_eq!(
        encoded.last(),
        Some(&b'\n'),
        "encoded request must end with newline"
    );

    let decoded = decode_request(&encoded).expect("decode failed");

    match decoded {
        DaemonRequest::Stop { id } => {
            assert_eq!(id, "abc123def456");
        }
        _ => panic!("wrong request type"),
    }
}

// ---------------------------------------------------------------------------
// Response encoding/decoding tests
// ---------------------------------------------------------------------------

#[test]
fn conformance_response_success_roundtrip() {
    let resp = DaemonResponse::Success {
        message: "Operation completed successfully".to_string(),
    };

    let encoded = encode_response(&resp).expect("encode failed");
    assert_eq!(
        encoded.last(),
        Some(&b'\n'),
        "encoded response must end with newline"
    );

    let decoded = decode_response(&encoded).expect("decode failed");

    match decoded {
        DaemonResponse::Success { message } => {
            assert_eq!(message, "Operation completed successfully");
        }
        _ => panic!("wrong response type"),
    }
}

#[test]
fn conformance_response_error_roundtrip() {
    let resp = DaemonResponse::Error {
        message: "Image not found: ubuntu:99.99".to_string(),
    };

    let encoded = encode_response(&resp).expect("encode failed");
    assert_eq!(
        encoded.last(),
        Some(&b'\n'),
        "encoded response must end with newline"
    );

    let decoded = decode_response(&encoded).expect("decode failed");

    match decoded {
        DaemonResponse::Error { message } => {
            assert_eq!(message, "Image not found: ubuntu:99.99");
        }
        _ => panic!("wrong response type"),
    }
}

#[test]
fn conformance_response_container_created_roundtrip() {
    let resp = DaemonResponse::ContainerCreated {
        id: "container-uuid-1234-5678".to_string(),
    };

    let encoded = encode_response(&resp).expect("encode failed");
    assert_eq!(
        encoded.last(),
        Some(&b'\n'),
        "encoded response must end with newline"
    );

    let decoded = decode_response(&encoded).expect("decode failed");

    match decoded {
        DaemonResponse::ContainerCreated { id } => {
            assert_eq!(id, "container-uuid-1234-5678");
        }
        _ => panic!("wrong response type"),
    }
}

#[test]
fn conformance_response_container_stopped_roundtrip() {
    let resp = DaemonResponse::ContainerStopped { exit_code: 42 };

    let encoded = encode_response(&resp).expect("encode failed");
    assert_eq!(
        encoded.last(),
        Some(&b'\n'),
        "encoded response must end with newline"
    );

    let decoded = decode_response(&encoded).expect("decode failed");

    match decoded {
        DaemonResponse::ContainerStopped { exit_code } => {
            assert_eq!(exit_code, 42);
        }
        _ => panic!("wrong response type"),
    }
}

#[test]
fn conformance_response_container_output_roundtrip() {
    let resp = DaemonResponse::ContainerOutput {
        stream: OutputStreamKind::Stdout,
        data: "SGVsbG8gV29ybGQh".to_string(), // "Hello World!" in base64
    };

    let encoded = encode_response(&resp).expect("encode failed");
    assert_eq!(
        encoded.last(),
        Some(&b'\n'),
        "encoded response must end with newline"
    );

    let decoded = decode_response(&encoded).expect("decode failed");

    match decoded {
        DaemonResponse::ContainerOutput { stream, data } => {
            assert_eq!(stream, OutputStreamKind::Stdout);
            assert_eq!(data, "SGVsbG8gV29ybGQh");
        }
        _ => panic!("wrong response type"),
    }
}

#[test]
fn conformance_response_container_list_roundtrip() {
    let containers = vec![
        ContainerInfo {
            id: "abc123".to_string(),
            name: Some("web-server".to_string()),
            image: "nginx:latest".to_string(),
            command: "/usr/sbin/nginx -g daemon off;".to_string(),
            state: "running".to_string(),
            created_at: "2026-04-26T12:34:56Z".to_string(),
            pid: Some(1234),
        },
        ContainerInfo {
            id: "def456".to_string(),
            name: None,
            image: "ubuntu:22.04".to_string(),
            command: "/bin/bash".to_string(),
            state: "stopped".to_string(),
            created_at: "2026-04-25T10:00:00Z".to_string(),
            pid: None,
        },
    ];

    let resp = DaemonResponse::ContainerList {
        containers: containers.clone(),
    };

    let encoded = encode_response(&resp).expect("encode failed");
    assert_eq!(
        encoded.last(),
        Some(&b'\n'),
        "encoded response must end with newline"
    );

    let decoded = decode_response(&encoded).expect("decode failed");

    match decoded {
        DaemonResponse::ContainerList {
            containers: decoded_containers,
        } => {
            assert_eq!(decoded_containers.len(), 2);
            assert_eq!(decoded_containers[0].id, "abc123");
            assert_eq!(decoded_containers[0].name, Some("web-server".to_string()));
            assert_eq!(decoded_containers[1].id, "def456");
            assert_eq!(decoded_containers[1].name, None);
        }
        _ => panic!("wrong response type"),
    }
}

// ---------------------------------------------------------------------------
// Serde default field tests
// ---------------------------------------------------------------------------

#[test]
fn conformance_request_run_serde_default_fields() {
    // Construct a Run request JSON with only required fields, omitting defaults
    let json = r#"{
        "type": "Run",
        "image": "alpine",
        "command": ["/bin/sh"]
    }"#;

    let decoded = decode_request(json.as_bytes()).expect("decode failed");

    match decoded {
        DaemonRequest::Run {
            image,
            tag,
            command,
            ephemeral,
            network,
            env,
            mounts,
            privileged,
            name,
            tty,
            memory_limit_bytes,
            cpu_weight,
        } => {
            assert_eq!(image, "alpine");
            assert_eq!(tag, None);
            assert_eq!(command, vec!["/bin/sh"]);
            assert!(!ephemeral, "ephemeral should default to false");
            assert_eq!(network, None, "network should default to None");
            assert!(env.is_empty(), "env should default to empty vec");
            assert!(mounts.is_empty(), "mounts should default to empty vec");
            assert!(!privileged, "privileged should default to false");
            assert_eq!(name, None, "name should default to None");
            assert!(!tty, "tty should default to false");
            assert_eq!(memory_limit_bytes, None);
            assert_eq!(cpu_weight, None);
        }
        _ => panic!("wrong request type"),
    }
}

// ---------------------------------------------------------------------------
// Framing tests
// ---------------------------------------------------------------------------

#[test]
fn conformance_encoded_bytes_end_with_newline() {
    let variants = vec![
        (
            "Run",
            DaemonRequest::Run {
                image: "test".to_string(),
                tag: None,
                command: vec!["/bin/sh".to_string()],
                memory_limit_bytes: None,
                cpu_weight: None,
                ephemeral: false,
                network: None,
                env: vec![],
                mounts: vec![],
                privileged: false,
                name: None,
                tty: false,
            },
        ),
        (
            "Stop",
            DaemonRequest::Stop {
                id: "xyz".to_string(),
            },
        ),
        ("List", DaemonRequest::List),
        (
            "Pull",
            DaemonRequest::Pull {
                image: "alpine".to_string(),
                tag: None,
            },
        ),
    ];

    for (name, req) in variants {
        let encoded = encode_request(&req).expect(&format!("encode {} failed", name));
        assert_eq!(
            encoded.last(),
            Some(&b'\n'),
            "{} encoded request must end with newline",
            name
        );
    }
}

#[test]
fn conformance_decode_tolerates_missing_newline() {
    // Test that decode_request and decode_response work with or without trailing \n
    let req = DaemonRequest::List;
    let encoded_with_newline = encode_request(&req).expect("encode failed");

    // Remove the trailing newline
    let encoded_without_newline = &encoded_with_newline[..encoded_with_newline.len() - 1];

    // Decode should work in both cases
    let decoded_with = decode_request(&encoded_with_newline).expect("decode with newline failed");
    let decoded_without =
        decode_request(encoded_without_newline).expect("decode without newline failed");

    match (&decoded_with, &decoded_without) {
        (DaemonRequest::List, DaemonRequest::List) => {
            // Success
        }
        _ => panic!("decoded requests don't match expected type"),
    }
}
