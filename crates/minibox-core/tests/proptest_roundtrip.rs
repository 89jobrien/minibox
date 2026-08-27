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
    clippy::struct_excessive_bools,
    clippy::type_complexity,
    clippy::float_cmp,
    clippy::diverging_sub_expression,
    clippy::single_match_else
)]
//! Property-based roundtrip tests for serializable protocol and domain types.
//!
//! Each test verifies: `deserialize(serialize(x)) == x` for arbitrary generated values.
//! Types without `PartialEq` use `serde_json::Value` comparison instead.

use minibox_core::domain::{
    BindMount, ExecutionManifest, ExecutionManifestImage, ExecutionManifestRequest,
    ExecutionManifestRuntime, ExecutionManifestSubject, NetworkMode, PhaseOutcome, StepRetry,
    StepStatus, WorkflowDef, WorkflowStep,
};
use minibox_core::image::reference::ImageRef;
use minibox_core::protocol::{
    ContainerInfo, DaemonRequest, DaemonResponse, OutputStreamKind, PushCredentials,
};
use proptest::option;
use proptest::prelude::*;
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// Strategy helpers: domain types
// ---------------------------------------------------------------------------

fn arb_network_mode() -> impl Strategy<Value = NetworkMode> {
    prop_oneof![
        Just(NetworkMode::None),
        Just(NetworkMode::Bridge),
        Just(NetworkMode::Host),
        Just(NetworkMode::Tailnet),
    ]
}

fn arb_phase_outcome() -> impl Strategy<Value = PhaseOutcome> {
    prop_oneof![
        Just(PhaseOutcome::Succeeded),
        Just(PhaseOutcome::Skipped),
        Just(PhaseOutcome::Aborted),
        Just(PhaseOutcome::Failed),
        Just(PhaseOutcome::Errored),
    ]
}

fn arb_step_status() -> impl Strategy<Value = StepStatus> {
    prop_oneof![
        Just(StepStatus::Pending),
        Just(StepStatus::Running),
        Just(StepStatus::Succeeded),
        Just(StepStatus::Failed),
        Just(StepStatus::Skipped),
        Just(StepStatus::Errored),
    ]
}

fn arb_bind_mount() -> impl Strategy<Value = BindMount> {
    ("[a-z/]{1,20}", "[a-z/]{1,20}", any::<bool>()).prop_map(|(h, c, ro)| BindMount {
        host_path: PathBuf::from(h),
        container_path: PathBuf::from(c),
        read_only: ro,
    })
}

fn arb_step_retry() -> impl Strategy<Value = StepRetry> {
    (1..100u32, option::of(1..3600u64)).prop_map(|(t, s)| StepRetry {
        error_threshold: t,
        timeout_secs: s,
    })
}

fn arb_json_value() -> impl Strategy<Value = serde_json::Value> {
    prop_oneof![
        Just(serde_json::Value::Null),
        any::<bool>().prop_map(serde_json::Value::Bool),
        any::<i64>().prop_map(|n| serde_json::Value::Number(n.into())),
        "[a-zA-Z0-9 ]{0,30}".prop_map(serde_json::Value::String),
    ]
}

fn arb_workflow_step() -> impl Strategy<Value = WorkflowStep> {
    (
        "[a-z-]{1,15}",
        "[a-z-]{1,15}",
        option::of("[a-z]+"),
        any::<bool>(),
        option::of(arb_step_retry()),
        arb_json_value(),
    )
        .prop_map(|(kind, alias, if_expr, cont, retry, config)| WorkflowStep {
            kind,
            alias,
            if_expr,
            if_guard: None,
            continue_on_error: cont,
            retry,
            vars: vec![],
            config,
        })
}

fn arb_workflow_def() -> impl Strategy<Value = WorkflowDef> {
    (
        proptest::collection::vec(arb_workflow_step(), 0..4),
        option::of("[a-z-]{1,10}"),
    )
        .prop_map(|(steps, start)| WorkflowDef {
            steps,
            state: std::collections::HashMap::new(),
            start_from_step: start,
        })
}

// ---------------------------------------------------------------------------
// Strategy helpers: ImageRef (parse/display roundtrip)
// ---------------------------------------------------------------------------

/// Generate valid image reference strings in one of three forms:
/// - `name:tag` (bare Docker Hub library image)
/// - `org/name:tag` (Docker Hub with namespace)
/// - `registry.io/org/name:tag` (custom registry, requires dot in host)
fn arb_image_ref_string() -> impl Strategy<Value = String> {
    let bare = ("[a-z]{3,10}", "[a-z0-9.]{1,8}").prop_map(|(name, tag)| format!("{name}:{tag}"));
    let org = ("[a-z]{3,10}", "[a-z]{3,10}", "[a-z0-9.]{1,8}")
        .prop_map(|(org, name, tag)| format!("{org}/{name}:{tag}"));
    let registry = (
        "[a-z]{3,8}\\.[a-z]{2,4}",
        "[a-z]{3,10}",
        "[a-z]{3,10}",
        "[a-z0-9.]{1,8}",
    )
        .prop_map(|(reg, org, name, tag)| format!("{reg}/{org}/{name}:{tag}"));
    prop_oneof![bare, org, registry]
}

// ---------------------------------------------------------------------------
// Strategy helpers: protocol types
// ---------------------------------------------------------------------------

fn arb_output_stream() -> impl Strategy<Value = OutputStreamKind> {
    prop_oneof![
        Just(OutputStreamKind::Stdout),
        Just(OutputStreamKind::Stderr),
    ]
}

fn arb_container_info() -> impl Strategy<Value = ContainerInfo> {
    (
        "[a-f0-9]{8}",
        option::of("[a-z]{3,10}"),
        "[a-z]{3,10}",
        "[a-z ]{1,20}",
        prop_oneof![
            Just("created"),
            Just("running"),
            Just("stopped"),
            Just("removed"),
        ],
        "[0-9]{4}-[0-9]{2}-[0-9]{2}",
        option::of(1..65000u32),
    )
        .prop_map(|(id, name, image, cmd, state, ts, pid)| ContainerInfo {
            id,
            name,
            image,
            command: cmd,
            state: state.to_string(),
            created_at: ts,
            pid,
        })
}

fn arb_push_credentials() -> impl Strategy<Value = PushCredentials> {
    prop_oneof![
        Just(PushCredentials::Anonymous),
        ("[a-z]{3,10}", "[a-z]{3,10}").prop_map(|(u, p)| PushCredentials::Basic {
            username: u,
            password: p
        }),
        "[a-z]{10,20}".prop_map(|t| PushCredentials::Token { token: t }),
    ]
}

/// Generate a subset of DaemonRequest variants that are safe to roundtrip.
/// Excludes variants with slashcrux types (Priority, Urgency, ExecutionContext)
/// since we cannot generate those from an external crate.
fn arb_daemon_request() -> impl Strategy<Value = DaemonRequest> {
    prop_oneof![
        // Run (minimal — no slashcrux fields; split into two tuples to stay
        // within proptest's 12-element limit)
        (
            (
                "[a-z]{3,10}",
                option::of("[a-z0-9.]{1,10}"),
                proptest::collection::vec("[a-z]{1,10}".prop_map(String::from), 0..3),
                option::of(1..1_000_000u64),
                option::of(1..10000u64),
                any::<bool>(),
                option::of(arb_network_mode()),
                proptest::collection::vec(arb_bind_mount(), 0..2),
            ),
            (
                any::<bool>(),
                proptest::collection::vec("[A-Z]+=val".prop_map(String::from), 0..2),
                option::of("[a-z]{3,10}"),
                any::<bool>(),
                option::of("[a-z]{3,10}"),
                option::of("[a-z0-9:]{3,10}"),
                any::<bool>(),
                option::of("[a-z/]{3,15}"),
            ),
        )
            .prop_map(
                |(
                    (image, tag, command, mem, cpu, eph, net, mounts),
                    (priv_, env, name, tty, ep, user, auto_rm, platform),
                )| {
                    DaemonRequest::Run {
                        image,
                        tag,
                        command,
                        memory_limit_bytes: mem,
                        cpu_weight: cpu,
                        ephemeral: eph,
                        network: net,
                        env,
                        mounts,
                        privileged: priv_,
                        shared_uid_range: false,
                        name,
                        tty,
                        entrypoint: ep,
                        user,
                        auto_remove: auto_rm,
                        priority: None,
                        urgency: None,
                        execution_context: None,
                        platform,
                        cgroup_parent: None,
                    }
                },
            ),
        "[a-f0-9]{8}".prop_map(|id| DaemonRequest::Stop { id }),
        "[a-f0-9]{8}".prop_map(|id| DaemonRequest::PauseContainer { id }),
        "[a-f0-9]{8}".prop_map(|id| DaemonRequest::ResumeContainer { id }),
        "[a-f0-9]{8}".prop_map(|id| DaemonRequest::Remove { id }),
        Just(DaemonRequest::List),
        (
            "[a-z]{3,10}",
            option::of("[a-z0-9.]{1,10}"),
            option::of("[a-z/]{3,15}")
        )
            .prop_map(|(image, tag, platform)| DaemonRequest::Pull {
                image,
                tag,
                platform
            }),
        Just(DaemonRequest::SubscribeEvents),
        any::<bool>().prop_map(|dr| DaemonRequest::Prune { dry_run: dr }),
        Just(DaemonRequest::ListImages),
        "[a-z:]{3,15}".prop_map(|r| DaemonRequest::RemoveImage { image_ref: r }),
        ("[a-f0-9]{8}", any::<bool>()).prop_map(|(id, f)| DaemonRequest::ContainerLogs {
            container_id: id,
            follow: f
        }),
        "[a-f0-9]{8}".prop_map(|id| DaemonRequest::GetManifest { id }),
        arb_workflow_def().prop_map(DaemonRequest::RunWorkflow),
    ]
}

/// Generate DaemonResponse variants that are safe to roundtrip.
/// Excludes Event (contains SystemTime) and SnapshotSaved/SnapshotList
/// (SnapshotInfo lacks PartialEq). Uses Value comparison anyway.
fn arb_daemon_response() -> impl Strategy<Value = DaemonResponse> {
    prop_oneof![
        "[a-f0-9]{8}".prop_map(|id| DaemonResponse::ContainerCreated { id }),
        "[a-z ]{5,30}".prop_map(|m| DaemonResponse::Success { message: m }),
        "[a-f0-9]{8}".prop_map(|id| DaemonResponse::ContainerPaused { id }),
        "[a-f0-9]{8}".prop_map(|id| DaemonResponse::ContainerResumed { id }),
        proptest::collection::vec(arb_container_info(), 0..3)
            .prop_map(|c| DaemonResponse::ContainerList { containers: c }),
        "[a-z:]{5,20}".prop_map(|i| DaemonResponse::ImageLoaded { image: i }),
        "[a-z ]{5,30}".prop_map(|m| DaemonResponse::Error { message: m }),
        (arb_output_stream(), "[a-zA-Z0-9+/=]{0,40}")
            .prop_map(|(s, d)| DaemonResponse::ContainerOutput { stream: s, data: d }),
        (-128..128i32).prop_map(|c| DaemonResponse::ContainerStopped { exit_code: c }),
        "[a-f0-9]{8}".prop_map(|id| DaemonResponse::ExecStarted { exec_id: id }),
        ("[a-f0-9:]{10,30}", 0..10_000u64, 1..100_000u64).prop_map(|(d, u, t)| {
            DaemonResponse::PushProgress {
                layer_digest: d,
                bytes_uploaded: u,
                total_bytes: t,
            }
        }),
        (1..20u32, 1..20u32, "[a-z ]{5,30}").prop_map(|(s, t, m)| DaemonResponse::BuildOutput {
            step: s,
            total_steps: t,
            message: m,
        }),
        ("[a-f0-9]{8}", "[a-z:]{3,10}")
            .prop_map(|(id, tag)| DaemonResponse::BuildComplete { image_id: id, tag }),
        proptest::collection::vec("[a-z:]{3,15}", 0..3)
            .prop_map(|i| DaemonResponse::ImageList { images: i }),
        (
            proptest::collection::vec("[a-z:]{3,15}", 0..3),
            0..100_000u64,
            any::<bool>(),
        )
            .prop_map(|(r, f, d)| DaemonResponse::Pruned {
                removed: r,
                freed_bytes: f,
                dry_run: d,
            }),
        (arb_output_stream(), "[a-z ]{0,40}")
            .prop_map(|(s, l)| DaemonResponse::LogLine { stream: s, line: l }),
        (any::<bool>(), option::of("[a-z ]{5,30}")).prop_map(|(a, r)| {
            DaemonResponse::VerifyResult {
                allowed: a,
                reason: r,
            }
        }),
        ("[a-z]{3,10}", arb_json_value(), arb_step_status()).prop_map(|(a, o, s)| {
            DaemonResponse::WorkflowStepComplete {
                alias: a,
                output: o,
                status: s,
            }
        }),
        arb_phase_outcome().prop_map(|p| DaemonResponse::WorkflowComplete { final_phase: p }),
    ]
}

// ---------------------------------------------------------------------------
// Roundtrip tests
// ---------------------------------------------------------------------------

proptest::proptest! {
    #![proptest_config(ProptestConfig {
        cases: 256,
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    /// DaemonRequest survives JSON encode -> decode roundtrip.
    #[test]
    fn daemon_request_json_roundtrip(req in arb_daemon_request()) {
        let encoded = minibox_core::protocol::encode_request(&req)
            .expect("encode_request must succeed");
        let decoded = minibox_core::protocol::decode_request(&encoded)
            .expect("decode_request must succeed");
        // No PartialEq on DaemonRequest — compare via Value
        let v1 = serde_json::to_value(&req).expect("to_value original");
        let v2 = serde_json::to_value(&decoded).expect("to_value decoded");
        prop_assert_eq!(v1, v2);
    }

    /// DaemonResponse survives JSON encode -> decode roundtrip.
    #[test]
    fn daemon_response_json_roundtrip(resp in arb_daemon_response()) {
        let encoded = minibox_core::protocol::encode_response(&resp)
            .expect("encode_response must succeed");
        let decoded = minibox_core::protocol::decode_response(&encoded)
            .expect("decode_response must succeed");
        let v1 = serde_json::to_value(&resp).expect("to_value original");
        let v2 = serde_json::to_value(&decoded).expect("to_value decoded");
        prop_assert_eq!(v1, v2);
    }

    /// ContainerInfo survives JSON roundtrip with PartialEq.
    #[test]
    fn container_info_roundtrip(ci in arb_container_info()) {
        let json = serde_json::to_string(&ci).expect("serialize");
        let decoded: ContainerInfo = serde_json::from_str(&json).expect("deserialize");
        // ContainerInfo derives Clone+Debug+Serialize+Deserialize but not PartialEq,
        // so compare via Value.
        let v1 = serde_json::to_value(&ci).expect("v1");
        let v2 = serde_json::to_value(&decoded).expect("v2");
        prop_assert_eq!(v1, v2);
    }

    /// BindMount survives JSON roundtrip.
    #[test]
    fn bind_mount_roundtrip(bm in arb_bind_mount()) {
        let json = serde_json::to_string(&bm).expect("serialize");
        let decoded: BindMount = serde_json::from_str(&json).expect("deserialize");
        prop_assert_eq!(bm, decoded);
    }

    /// NetworkMode survives JSON roundtrip.
    #[test]
    fn network_mode_roundtrip(nm in arb_network_mode()) {
        let json = serde_json::to_string(&nm).expect("serialize");
        let decoded: NetworkMode = serde_json::from_str(&json).expect("deserialize");
        prop_assert_eq!(nm, decoded);
    }

    /// PhaseOutcome survives JSON roundtrip.
    #[test]
    fn phase_outcome_roundtrip(po in arb_phase_outcome()) {
        let json = serde_json::to_string(&po).expect("serialize");
        let decoded: PhaseOutcome = serde_json::from_str(&json).expect("deserialize");
        prop_assert_eq!(po, decoded);
    }

    /// StepStatus survives JSON roundtrip.
    #[test]
    fn step_status_roundtrip(ss in arb_step_status()) {
        let json = serde_json::to_string(&ss).expect("serialize");
        let decoded: StepStatus = serde_json::from_str(&json).expect("deserialize");
        prop_assert_eq!(ss, decoded);
    }

    /// WorkflowDef survives JSON roundtrip.
    #[test]
    fn workflow_def_roundtrip(wd in arb_workflow_def()) {
        let json = serde_json::to_string(&wd).expect("serialize");
        let decoded: WorkflowDef = serde_json::from_str(&json).expect("deserialize");
        // WorkflowDef does not derive PartialEq — compare via Value.
        let v1 = serde_json::to_value(&wd).expect("v1");
        let v2 = serde_json::to_value(&decoded).expect("v2");
        prop_assert_eq!(v1, v2);
    }

    /// ExecutionManifest survives JSON roundtrip (additional coverage beyond inline tests).
    #[test]
    fn execution_manifest_roundtrip(
        sv in any::<u32>(),
        cid in "[a-f0-9]{8,16}",
        ts in "[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z",
        img_ref in "[a-z]{3,10}:[a-z0-9.]{1,8}",
        cmd in proptest::collection::vec("[a-z]{1,8}", 1..4),
        net_mode in "[a-z]{3,6}",
        priv_ in any::<bool>(),
        eph in any::<bool>(),
        name in option::of("[a-z]{3,10}"),
    ) {
        let m = ExecutionManifest {
            schema_version: sv,
            container_id: cid,
            created_at: ts,
            manifest_path: None,
            workload_digest: None,
            subject: ExecutionManifestSubject {
                image_ref: img_ref,
                image: ExecutionManifestImage {
                    manifest_digest: None,
                    config_digest: None,
                    layer_digests: vec![],
                },
            },
            runtime: ExecutionManifestRuntime {
                command: cmd,
                env: vec![],
                mounts: vec![],
                resource_limits: None,
                network_mode: net_mode,
                privileged: priv_,
                platform: None,
            },
            request: ExecutionManifestRequest {
                name,
                ephemeral: eph,
            },
        };
        let json = serde_json::to_string(&m).expect("serialize");
        let decoded: ExecutionManifest = serde_json::from_str(&json).expect("deserialize");
        prop_assert_eq!(m, decoded);
    }

    /// PushCredentials survives JSON roundtrip.
    #[test]
    fn push_credentials_roundtrip(pc in arb_push_credentials()) {
        let json = serde_json::to_string(&pc).expect("serialize");
        let decoded: PushCredentials = serde_json::from_str(&json).expect("deserialize");
        // No PartialEq — compare via Value.
        let v1 = serde_json::to_value(&pc).expect("v1");
        let v2 = serde_json::to_value(&decoded).expect("v2");
        prop_assert_eq!(v1, v2);
    }

    /// OutputStreamKind survives JSON roundtrip.
    #[test]
    fn output_stream_kind_roundtrip(os in arb_output_stream()) {
        let json = serde_json::to_string(&os).expect("serialize");
        let decoded: OutputStreamKind = serde_json::from_str(&json).expect("deserialize");
        prop_assert_eq!(os, decoded);
    }

    /// ImageRef survives parse(display(x)) == x roundtrip.
    ///
    /// ImageRef is not Serialize/Deserialize — it uses parse/display instead.
    /// We generate valid image references in three forms:
    /// - bare name (docker.io/library default)
    /// - org/name (docker.io with custom namespace)
    /// - registry.tld/org/name (custom registry)
    #[test]
    fn image_ref_parse_display_roundtrip(ref_str in arb_image_ref_string()) {
        let parsed = ImageRef::parse(&ref_str).expect("parse should succeed");
        let displayed = parsed.to_string();
        let reparsed = ImageRef::parse(&displayed).expect("reparse should succeed");
        prop_assert_eq!(parsed, reparsed);
    }
}
