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
//! Property-based tests for minibox's public API.
//!
//! Invariants tested:
//! - Protocol encode→decode roundtrip is lossless (re-encode produces same bytes)
//! - Arbitrary valid DaemonRequest / DaemonResponse survive the round-trip
//! - image/tag strings of any content survive protocol encode→decode
//!
//! Note: `ImageRef` is not a public type; image ref string safety is covered
//! by the `DaemonRequest::Pull` roundtrip which exercises arbitrary image/tag
//! strings through the full protocol layer.

use minibox::domain::SessionId;
use minibox::protocol::{
    ContainerInfo, DaemonRequest, DaemonResponse, OutputStreamKind, PushCredentials,
    decode_request, decode_response, encode_request, encode_response,
};
use minibox_core::events::ContainerEvent;
use proptest::option;
use proptest::prelude::*;
use std::time::SystemTime;

// ---------------------------------------------------------------------------
// Strategies
// ---------------------------------------------------------------------------

fn arb_session_id() -> impl Strategy<Value = SessionId> {
    any::<String>().prop_map(SessionId::from)
}

fn arb_push_credentials() -> impl Strategy<Value = PushCredentials> {
    prop_oneof![
        Just(PushCredentials::Anonymous),
        (any::<String>(), any::<String>())
            .prop_map(|(username, password)| PushCredentials::Basic { username, password }),
        any::<String>().prop_map(|token| PushCredentials::Token { token }),
    ]
}

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
                        env: vec![],
                        name: None,
                        tty: false,
                        entrypoint: None,
                        user: None,
                        auto_remove: false,
                        priority: None,
                        urgency: None,
                        execution_context: None,
                        platform: None,
                        cgroup_parent: None,
                    }
                }
            ),
        any::<String>().prop_map(|id| DaemonRequest::Stop { id }),
        any::<String>().prop_map(|id| DaemonRequest::PauseContainer { id }),
        any::<String>().prop_map(|id| DaemonRequest::ResumeContainer { id }),
        any::<String>().prop_map(|id| DaemonRequest::Remove { id }),
        Just(DaemonRequest::List),
        // Exercises arbitrary image ref strings (the spec's ImageRef invariant —
        // ImageRef is not public, but Pull carries the same data through the wire).
        (any::<String>(), option::of(any::<String>())).prop_map(|(image, tag)| {
            DaemonRequest::Pull {
                image,
                tag,
                platform: None,
            }
        }),
        (any::<String>(), any::<String>(), any::<String>())
            .prop_map(|(path, name, tag)| DaemonRequest::LoadImage { path, name, tag }),
        (
            any::<String>(),
            prop::collection::vec(any::<String>(), 0..8),
            prop::collection::vec(any::<String>(), 0..4),
            option::of(any::<String>()),
            any::<bool>(),
        )
            .prop_map(
                |(container_id, cmd, env, working_dir, tty)| DaemonRequest::Exec {
                    container_id,
                    cmd,
                    env,
                    working_dir,
                    tty,
                    user: None,
                }
            ),
        (arb_session_id(), any::<String>())
            .prop_map(|(session_id, data)| DaemonRequest::SendInput { session_id, data }),
        (arb_session_id(), any::<u16>(), any::<u16>()).prop_map(|(session_id, cols, rows)| {
            DaemonRequest::ResizePty {
                session_id,
                cols,
                rows,
            }
        }),
        (any::<String>(), arb_push_credentials()).prop_map(|(image_ref, credentials)| {
            DaemonRequest::Push {
                image_ref,
                credentials,
            }
        }),
        (
            any::<String>(),
            any::<String>(),
            option::of(any::<String>()),
            option::of(any::<String>()),
            prop::collection::vec(any::<String>(), 0..4),
            option::of(prop::collection::vec(any::<String>(), 0..4)),
            any::<bool>(),
        )
            .prop_map(
                |(
                    container_id,
                    target_image,
                    author,
                    message,
                    env_overrides,
                    cmd_override,
                    include_volumes,
                )| {
                    DaemonRequest::Commit {
                        container_id,
                        target_image,
                        author,
                        message,
                        env_overrides,
                        cmd_override,
                        include_volumes,
                    }
                },
            ),
        (
            any::<String>(),
            any::<String>(),
            any::<String>(),
            prop::collection::vec((any::<String>(), any::<String>()), 0..4),
            any::<bool>(),
        )
            .prop_map(|(dockerfile, context_path, tag, build_args, no_cache)| {
                DaemonRequest::Build {
                    dockerfile,
                    context_path,
                    tag,
                    build_args,
                    no_cache,
                }
            }),
        Just(DaemonRequest::SubscribeEvents),
        any::<bool>().prop_map(|dry_run| DaemonRequest::Prune { dry_run }),
        any::<String>().prop_map(|image_ref| DaemonRequest::RemoveImage { image_ref }),
        (any::<String>(), any::<bool>()).prop_map(|(container_id, follow)| {
            DaemonRequest::ContainerLogs {
                container_id,
                follow,
            }
        }),
        (
            any::<String>(),
            option::of(any::<String>()),
            prop::collection::vec((any::<String>(), any::<String>()), 0..4),
            any::<u32>(),
        )
            .prop_map(|(pipeline_path, image, env, max_depth)| {
                DaemonRequest::RunPipeline {
                    pipeline_path,
                    input: None,
                    image,
                    budget: None,
                    env,
                    max_depth,
                    priority: None,
                    urgency: None,
                    execution_context: None,
                }
            }),
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
                name: None,
                image,
                command,
                state,
                created_at,
                pid,
            },
        )
}

fn arb_container_event() -> impl Strategy<Value = ContainerEvent> {
    prop_oneof![
        (any::<String>(), any::<String>()).prop_map(|(id, image)| ContainerEvent::Created {
            id,
            image,
            timestamp: SystemTime::UNIX_EPOCH,
        }),
        (any::<String>(), any::<u32>()).prop_map(|(id, pid)| ContainerEvent::Started {
            id,
            pid,
            timestamp: SystemTime::UNIX_EPOCH,
        }),
        (any::<String>(), any::<i32>()).prop_map(|(id, exit_code)| ContainerEvent::Stopped {
            id,
            exit_code,
            timestamp: SystemTime::UNIX_EPOCH,
        }),
        any::<String>().prop_map(|id| ContainerEvent::Paused {
            id,
            timestamp: SystemTime::UNIX_EPOCH,
        }),
        any::<String>().prop_map(|id| ContainerEvent::Resumed {
            id,
            timestamp: SystemTime::UNIX_EPOCH,
        }),
        any::<String>().prop_map(|id| ContainerEvent::OomKilled {
            id,
            timestamp: SystemTime::UNIX_EPOCH,
        }),
    ]
}

fn arb_response() -> impl Strategy<Value = DaemonResponse> {
    prop_oneof![
        any::<String>().prop_map(|id| DaemonResponse::ContainerCreated { id }),
        any::<String>().prop_map(|message| DaemonResponse::Success { message }),
        any::<String>().prop_map(|id| DaemonResponse::ContainerPaused { id }),
        any::<String>().prop_map(|id| DaemonResponse::ContainerResumed { id }),
        prop::collection::vec(arb_container_info(), 0..8)
            .prop_map(|containers| DaemonResponse::ContainerList { containers }),
        any::<String>().prop_map(|image| DaemonResponse::ImageLoaded { image }),
        any::<String>().prop_map(|message| DaemonResponse::Error { message }),
        (arb_stream_kind(), any::<String>())
            .prop_map(|(stream, data)| DaemonResponse::ContainerOutput { stream, data }),
        any::<i32>().prop_map(|exit_code| DaemonResponse::ContainerStopped { exit_code }),
        any::<String>().prop_map(|exec_id| DaemonResponse::ExecStarted { exec_id }),
        (any::<String>(), any::<u64>(), any::<u64>()).prop_map(
            |(layer_digest, bytes_uploaded, total_bytes)| DaemonResponse::PushProgress {
                layer_digest,
                bytes_uploaded,
                total_bytes,
            }
        ),
        (any::<u32>(), any::<u32>(), any::<String>()).prop_map(|(step, total_steps, message)| {
            DaemonResponse::BuildOutput {
                step,
                total_steps,
                message,
            }
        }),
        (any::<String>(), any::<String>())
            .prop_map(|(image_id, tag)| DaemonResponse::BuildComplete { image_id, tag }),
        arb_container_event().prop_map(|event| DaemonResponse::Event { event }),
        (
            prop::collection::vec(any::<String>(), 0..4),
            any::<u64>(),
            any::<bool>()
        )
            .prop_map(|(removed, freed_bytes, dry_run)| DaemonResponse::Pruned {
                removed,
                freed_bytes,
                dry_run,
            }),
        (arb_stream_kind(), any::<String>())
            .prop_map(|(stream, line)| DaemonResponse::LogLine { stream, line }),
        (any::<i32>(), any::<String>()).prop_map(|(exit_code, container_id)| {
            DaemonResponse::PipelineComplete {
                trace: serde_json::Value::Null,
                container_id,
                exit_code,
            }
        }),
    ]
}

// ---------------------------------------------------------------------------
// Supplementary strategies
// ---------------------------------------------------------------------------

fn arb_env_entry() -> impl Strategy<Value = String> {
    ("[A-Z_]{1,8}", any::<String>()).prop_map(|(k, v)| format!("{k}={v}"))
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

    /// DaemonRequest::Run with all optional fields populated must roundtrip.
    ///
    /// The base arb_request() leaves env, name, platform, tty, and auto_remove
    /// at their defaults. This property exercises the full field matrix so that
    /// adding a new field without a serde default is caught immediately.
    #[test]
    fn run_request_full_fields_roundtrip(
        image in any::<String>(),
        tag in option::of(any::<String>()),
        command in proptest::collection::vec(any::<String>(), 0..8),
        memory_limit_bytes in option::of(any::<u64>()),
        cpu_weight in option::of(1u64..=10_000u64),
        ephemeral in any::<bool>(),
        env in proptest::collection::vec(arb_env_entry(), 0..6),
        name in option::of("[a-zA-Z0-9_-]{1,20}"),
        platform in option::of(prop_oneof![
            Just("linux/amd64".to_string()),
            Just("linux/arm64".to_string()),
        ]),
        tty in any::<bool>(),
        auto_remove in any::<bool>(),
    ) {
        let req = DaemonRequest::Run {
            image,
            tag,
            command,
            memory_limit_bytes,
            cpu_weight,
            ephemeral,
            network: None,
            mounts: vec![],
            privileged: false,
            env,
            name,
            tty,
            entrypoint: None,
            user: None,
            auto_remove,
            priority: None,
            urgency: None,
            execution_context: None,
            platform,
            cgroup_parent: None,
        };
        let encoded = encode_request(&req).expect("encode must succeed");
        let decoded = decode_request(&encoded).expect("decode must succeed");
        let re_encoded = encode_request(&decoded).expect("re-encode must succeed");
        prop_assert_eq!(encoded, re_encoded);
    }
}

// ── CgroupConfig boundary validation (Linux + root only) ─────────────────────

#[cfg(target_os = "linux")]
mod cgroup_props {
    use minibox::container::cgroups::{CgroupConfig, CgroupManager};
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
