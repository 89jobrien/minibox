//! Property-based tests for linuxbox's public API.
//!
//! Invariants tested:
//! - Protocol encode→decode roundtrip is lossless (re-encode produces same bytes)
//! - Arbitrary valid DaemonRequest / DaemonResponse survive the round-trip
//! - image/tag strings of any content survive protocol encode→decode
//!
//! Note: `ImageRef` is not a public type; image ref string safety is covered
//! by the `DaemonRequest::Pull` roundtrip which exercises arbitrary image/tag
//! strings through the full protocol layer.

use linuxbox::protocol::{
    ContainerInfo, DaemonRequest, DaemonResponse, OutputStreamKind, decode_request,
    decode_response, encode_request, encode_response,
};
use proptest::option;
use proptest::prelude::*;

// ---------------------------------------------------------------------------
// Strategies
// ---------------------------------------------------------------------------

fn arb_request() -> impl Strategy<Value = DaemonRequest> {
    prop_oneof![
        (
            any::<String>(),
            option::of(any::<String>()),
            prop::collection::vec(any::<String>(), 0..8),
            option::of(any::<u64>()),
            option::of(1u64..=10000u64),
            any::<bool>(),
        )
            .prop_map(
                |(image, tag, command, memory_limit_bytes, cpu_weight, ephemeral)| {
                    DaemonRequest::Run {
                        image,
                        tag,
                        command,
                        memory_limit_bytes,
                        cpu_weight,
                        ephemeral,
                        network: None,
                        mounts: vec![],
                        privileged: false,
                    }
                }
            ),
        any::<String>().prop_map(|id| DaemonRequest::Stop { id }),
        any::<String>().prop_map(|id| DaemonRequest::Remove { id }),
        Just(DaemonRequest::List),
        // Exercises arbitrary image ref strings (the spec's ImageRef invariant —
        // ImageRef is not public, but Pull carries the same data through the wire).
        (any::<String>(), option::of(any::<String>()))
            .prop_map(|(image, tag)| DaemonRequest::Pull { image, tag }),
    ]
}

fn arb_stream_kind() -> impl Strategy<Value = OutputStreamKind> {
    prop_oneof![
        Just(OutputStreamKind::Stdout),
        Just(OutputStreamKind::Stderr)
    ]
}

fn arb_container_info() -> impl Strategy<Value = ContainerInfo> {
    (
        any::<String>(),
        any::<String>(),
        any::<String>(),
        any::<String>(),
        any::<String>(),
        option::of(any::<u32>()),
    )
        .prop_map(
            |(id, image, command, state, created_at, pid)| ContainerInfo {
                id,
                image,
                command,
                state,
                created_at,
                pid,
            },
        )
}

fn arb_response() -> impl Strategy<Value = DaemonResponse> {
    prop_oneof![
        any::<String>().prop_map(|id| DaemonResponse::ContainerCreated { id }),
        any::<String>().prop_map(|message| DaemonResponse::Success { message }),
        prop::collection::vec(arb_container_info(), 0..8)
            .prop_map(|containers| DaemonResponse::ContainerList { containers }),
        any::<String>().prop_map(|message| DaemonResponse::Error { message }),
        (arb_stream_kind(), any::<String>())
            .prop_map(|(stream, data)| DaemonResponse::ContainerOutput { stream, data }),
        any::<i32>().prop_map(|exit_code| DaemonResponse::ContainerStopped { exit_code }),
    ]
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

proptest! {
    /// Encoding a DaemonRequest and decoding it back, then re-encoding,
    /// must produce identical bytes — guarantees no data is lost in transit.
    #[test]
    fn request_encode_decode_roundtrip(req in arb_request()) {
        let encoded = encode_request(&req).expect("encode must succeed");
        let decoded = decode_request(&encoded).expect("decode must succeed");
        let re_encoded = encode_request(&decoded).expect("re-encode must succeed");
        prop_assert_eq!(encoded, re_encoded);
    }

    /// Same invariant for DaemonResponse (all six variants including ContainerList).
    #[test]
    fn response_encode_decode_roundtrip(resp in arb_response()) {
        let encoded = encode_response(&resp).expect("encode must succeed");
        let decoded = decode_response(&encoded).expect("decode must succeed");
        let re_encoded = encode_response(&decoded).expect("re-encode must succeed");
        prop_assert_eq!(encoded, re_encoded);
    }
}

// ── CgroupConfig boundary validation (Linux + root only) ─────────────────────

#[cfg(target_os = "linux")]
mod cgroup_props {
    use linuxbox::container::cgroups::{CgroupConfig, CgroupManager};
    use proptest::prelude::*;

    // NOTE: On unprivileged Linux (no root / no cgroup2 mount), `create_dir_all` will
    // fail with EACCES before the bounds check runs. The `prop_assert!(is_err())` still
    // passes, but for the wrong reason. These tests only exercise validation logic
    // correctly under `just test-integration` where `MINIBOX_CGROUP_ROOT` points to
    // a writable cgroup2 path with root privileges.
    proptest! {
        #![proptest_config(ProptestConfig { failure_persistence: None, ..ProptestConfig::default() })]
        #[test]
        fn memory_below_4096_always_rejected(
            mem in 0_u64..4096_u64,
            id in "[a-z]{8,16}",
        ) {
            let config = CgroupConfig {
                memory_limit_bytes: Some(mem),
                ..Default::default()
            };
            let mgr = CgroupManager::new(&id, config);
            prop_assert!(
                mgr.create().is_err(),
                "expected Err for memory={mem}, got Ok"
            );
        }

        #[test]
        fn cpu_weight_out_of_range_rejected(
            weight in prop_oneof![Just(0_u64), (10_001_u64..=u64::MAX)],
            id in "[a-z]{8,16}",
        ) {
            let config = CgroupConfig {
                cpu_weight: Some(weight),
                ..Default::default()
            };
            let mgr = CgroupManager::new(&id, config);
            prop_assert!(
                mgr.create().is_err(),
                "expected Err for cpu_weight={weight}, got Ok"
            );
        }
    }
}
