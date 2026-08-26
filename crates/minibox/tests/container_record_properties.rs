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
//! Property-based tests for ContainerRecord serde roundtrip.
//!
//! ContainerRecord is persisted to disk as JSON in state.json.  Any field
//! that fails to roundtrip cleanly would corrupt daemon state across restarts.

use minibox::daemon::state::{ContainerRecord, RunCreationParams};
use minibox_core::domain::BackendRootfsMetadata;
use minibox_core::protocol::ContainerInfo;
use proptest::option;
use proptest::prelude::*;
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// Strategies
// ---------------------------------------------------------------------------

fn arb_container_info() -> impl Strategy<Value = ContainerInfo> {
    (
        "[a-zA-Z0-9]{1,32}",
        option::of("[a-zA-Z0-9_-]{1,16}"),
        any::<String>(),
        any::<String>(),
        any::<String>(),
        any::<String>(),
        option::of(any::<u32>()),
    )
        .prop_map(
            |(id, name, image, command, state, created_at, pid)| ContainerInfo {
                id,
                name,
                image,
                command,
                state,
                created_at,
                pid,
            },
        )
}

fn arb_path() -> impl Strategy<Value = PathBuf> {
    "[a-z]{1,8}(/[a-z]{1,8}){0,4}".prop_map(PathBuf::from)
}

fn arb_rootfs_metadata() -> impl Strategy<Value = Option<BackendRootfsMetadata>> {
    option::of(arb_path().prop_map(|upper| BackendRootfsMetadata::Overlay {
        upper_dir: upper.into(),
        metadata: std::collections::HashMap::new(),
    }))
}

fn arb_creation_params() -> impl Strategy<Value = RunCreationParams> {
    (
        any::<String>(),
        option::of(any::<String>()),
        proptest::collection::vec(any::<String>(), 0..4),
        option::of(any::<u64>()),
        option::of(1u64..=10_000u64),
        proptest::collection::vec(any::<String>(), 0..4),
        any::<bool>(),
        option::of(any::<String>()),
        option::of(any::<String>()),
    )
        .prop_map(
            |(
                image,
                tag,
                command,
                memory_limit_bytes,
                cpu_weight,
                env,
                privileged,
                name,
                platform,
            )| {
                RunCreationParams {
                    image,
                    tag,
                    command,
                    memory_limit_bytes,
                    cpu_weight,
                    network: None,
                    env,
                    mounts: vec![],
                    privileged,
                    name,
                    tty: false,
                    entrypoint: None,
                    user: None,
                    platform,
                    cgroup_parent: None,
                }
            },
        )
}

fn arb_container_record() -> impl Strategy<Value = ContainerRecord> {
    (
        arb_container_info(),
        option::of(any::<u32>()),
        option::of(any::<String>()),
        arb_path(),
        arb_path(),
        option::of(any::<String>()),
        option::of(arb_creation_params()),
        option::of(any::<String>()),
        arb_rootfs_metadata(),
    )
        .prop_map(
            |(
                info,
                pid,
                runtime_id,
                rootfs_path,
                cgroup_path,
                source_image_ref,
                creation_params,
                workload_digest,
                rootfs_metadata,
            )| {
                let upper_dir = rootfs_metadata
                    .as_ref()
                    .map(|m| m.overlay_upper_dir().clone().into_inner());
                let merged_dir = Some(rootfs_path.clone());
                ContainerRecord {
                    info,
                    pid,
                    runtime_id,
                    rootfs_path,
                    cgroup_path,
                    post_exit_hooks: vec![],
                    rootfs_metadata,
                    source_image_ref,
                    upper_dir,
                    merged_dir,
                    step_state: None,
                    priority: None,
                    urgency: None,
                    execution_context: None,
                    creation_params,
                    manifest_path: None,
                    workload_digest,
                }
            },
        )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    /// ContainerRecord must survive a JSON serialize → deserialize roundtrip.
    ///
    /// This is the exact path taken by DaemonState when persisting and loading
    /// state.json on disk. Failure here means daemon state is silently corrupted
    /// across restarts.
    #[test]
    fn container_record_json_roundtrip(record in arb_container_record()) {
        let json = serde_json::to_string(&record).expect("serialize ContainerRecord");
        let decoded: ContainerRecord =
            serde_json::from_str(&json).expect("deserialize ContainerRecord");
        let re_encoded = serde_json::to_string(&decoded).expect("re-serialize ContainerRecord");
        prop_assert_eq!(
            json,
            re_encoded,
            "roundtrip produced different JSON — field data was lost or mutated"
        );
    }

    /// Optional fields (pid, runtime_id, source_image_ref, creation_params,
    /// workload_digest) set to None must round-trip to None — not to a
    /// default value.
    #[test]
    fn container_record_none_fields_stay_none(info in arb_container_info()) {
        let record = ContainerRecord {
            info,
            pid: None,
            runtime_id: None,
            rootfs_path: PathBuf::from("/tmp/rootfs"),
            cgroup_path: PathBuf::from("/tmp/cgroup"),
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
        };
        let json = serde_json::to_string(&record).expect("serialize");
        let decoded: ContainerRecord = serde_json::from_str(&json).expect("deserialize");
        prop_assert!(decoded.pid.is_none(), "pid round-tripped to Some");
        prop_assert!(decoded.source_image_ref.is_none(), "source_image_ref round-tripped to Some");
        prop_assert!(decoded.creation_params.is_none(), "creation_params round-tripped to Some");
        prop_assert!(decoded.workload_digest.is_none(), "workload_digest round-tripped to Some");
    }

    /// A populated upper_dir/merged_dir/rootfs_metadata must survive the
    /// state.json roundtrip intact — regression guard for the field wiring
    /// added in 1ae7528e, which `mbx commit` depends on after a daemon restart.
    #[test]
    fn container_record_rootfs_metadata_survives_roundtrip(record in arb_container_record()) {
        let json = serde_json::to_string(&record).expect("serialize ContainerRecord");
        let decoded: ContainerRecord =
            serde_json::from_str(&json).expect("deserialize ContainerRecord");
        prop_assert_eq!(decoded.upper_dir, record.upper_dir);
        prop_assert_eq!(decoded.merged_dir, record.merged_dir);
        prop_assert_eq!(decoded.rootfs_metadata, record.rootfs_metadata);
    }
}
