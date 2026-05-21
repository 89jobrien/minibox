//! Proptest roundtrip tests for serializable protocol and domain types.
//!
//! Each test verifies: `deserialize(serialize(x)) == x` (compared via JSON
//! Value equality when the type lacks `PartialEq`).
//!
//! Types tested:
//! - `DaemonRequest` (protocol)
//! - `DaemonResponse` (protocol)
//! - `ExecutionManifest` (domain)
//!
//! Types from the issue that are skipped:
//! - `ImageReference` / `ImageRef` — not Serialize/Deserialize
//! - `ContainerConfig` — does not exist in minibox-core
//! - `BackendDescriptor` — contains `Box<dyn Fn()>`, not serializable

use minibox_core::domain::execution_manifest::*;
use minibox_core::domain::{
    BindMount, ExprVar, NetworkMode, PhaseOutcome, SessionId, StepRetry, StepStatus, WorkflowDef,
    WorkflowStep,
};
use minibox_core::protocol::*;
use proptest::prelude::*;
use std::path::PathBuf;
use std::time::SystemTime;

// ---------------------------------------------------------------------------
// Strategy helpers
// ---------------------------------------------------------------------------

fn arb_nonempty_string() -> impl Strategy<Value = String> {
    "[a-zA-Z0-9_-]{1,30}"
}

fn arb_network_mode() -> impl Strategy<Value = NetworkMode> {
    prop_oneof![
        Just(NetworkMode::None),
        Just(NetworkMode::Bridge),
        Just(NetworkMode::Host),
        Just(NetworkMode::Tailnet),
    ]
}

fn arb_bind_mount() -> impl Strategy<Value = BindMount> {
    (arb_nonempty_string(), arb_nonempty_string(), any::<bool>()).prop_map(|(h, c, ro)| BindMount {
        host_path: PathBuf::from(format!("/host/{h}")),
        container_path: PathBuf::from(format!("/container/{c}")),
        read_only: ro,
    })
}

fn arb_session_id() -> impl Strategy<Value = SessionId> {
    arb_nonempty_string().prop_map(SessionId::new)
}

fn arb_priority() -> impl Strategy<Value = slashcrux::Priority> {
    prop_oneof![
        Just(slashcrux::Priority::Critical),
        Just(slashcrux::Priority::High),
        Just(slashcrux::Priority::Medium),
        Just(slashcrux::Priority::Low),
        Just(slashcrux::Priority::Deferred),
    ]
}

fn arb_urgency() -> impl Strategy<Value = slashcrux::Urgency> {
    prop_oneof![
        Just(slashcrux::Urgency::Immediate),
        Just(slashcrux::Urgency::Soon),
        Just(slashcrux::Urgency::Whenever),
        Just(slashcrux::Urgency::Never),
    ]
}

fn arb_output_stream_kind() -> impl Strategy<Value = OutputStreamKind> {
    prop_oneof![
        Just(OutputStreamKind::Stdout),
        Just(OutputStreamKind::Stderr),
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

fn arb_push_credentials() -> impl Strategy<Value = PushCredentials> {
    prop_oneof![
        Just(PushCredentials::Anonymous),
        (arb_nonempty_string(), arb_nonempty_string()).prop_map(|(u, p)| PushCredentials::Basic {
            username: u,
            password: p,
        }),
        arb_nonempty_string().prop_map(|t| PushCredentials::Token { token: t }),
    ]
}

fn arb_step_retry() -> impl Strategy<Value = StepRetry> {
    (1..100u32, proptest::option::of(1..3600u64)).prop_map(|(et, ts)| StepRetry {
        error_threshold: et,
        timeout_secs: ts,
    })
}

fn arb_expr_var() -> impl Strategy<Value = ExprVar> {
    (arb_nonempty_string(), arb_nonempty_string()).prop_map(|(n, v)| ExprVar { name: n, value: v })
}

fn arb_workflow_step() -> impl Strategy<Value = WorkflowStep> {
    (
        arb_nonempty_string(),
        arb_nonempty_string(),
        proptest::option::of(arb_nonempty_string()),
        any::<bool>(),
        proptest::option::of(arb_step_retry()),
        proptest::collection::vec(arb_expr_var(), 0..3),
    )
        .prop_map(|(kind, alias, if_expr, coe, retry, vars)| WorkflowStep {
            kind,
            alias,
            if_expr,
            continue_on_error: coe,
            retry,
            vars,
            config: serde_json::Value::Null,
        })
}

fn arb_workflow_def() -> impl Strategy<Value = WorkflowDef> {
    (
        proptest::collection::vec(arb_workflow_step(), 1..4),
        proptest::option::of(arb_nonempty_string()),
    )
        .prop_map(|(steps, start)| WorkflowDef {
            steps,
            state: std::collections::HashMap::new(),
            start_from_step: start,
        })
}

fn arb_container_event() -> impl Strategy<Value = minibox_core::events::ContainerEvent> {
    use minibox_core::events::ContainerEvent;
    let ts = SystemTime::UNIX_EPOCH;
    prop_oneof![
        (arb_nonempty_string(), arb_nonempty_string()).prop_map(move |(id, image)| {
            ContainerEvent::Created {
                id,
                image,
                timestamp: ts,
            }
        }),
        (arb_nonempty_string(), 1..65000u32).prop_map(move |(id, pid)| {
            ContainerEvent::Started {
                id,
                pid,
                timestamp: ts,
            }
        }),
        (arb_nonempty_string(), -128..128i32).prop_map(move |(id, exit_code)| {
            ContainerEvent::Stopped {
                id,
                exit_code,
                timestamp: ts,
            }
        }),
        arb_nonempty_string().prop_map(move |id| ContainerEvent::Paused { id, timestamp: ts }),
        arb_nonempty_string().prop_map(move |id| ContainerEvent::Resumed { id, timestamp: ts }),
    ]
}

fn arb_snapshot_info() -> impl Strategy<Value = minibox_core::domain::SnapshotInfo> {
    (
        arb_nonempty_string(),
        arb_nonempty_string(),
        arb_nonempty_string(),
        arb_nonempty_string(),
        0..1_000_000u64,
    )
        .prop_map(|(cid, name, adapter, image, size_bytes)| {
            minibox_core::domain::SnapshotInfo {
                container_id: cid,
                name,
                created_at: "2026-01-01T00:00:00Z".to_string(),
                adapter,
                image,
                size_bytes,
            }
        })
}

fn arb_container_info() -> impl Strategy<Value = ContainerInfo> {
    (
        arb_nonempty_string(),
        proptest::option::of(arb_nonempty_string()),
        arb_nonempty_string(),
        arb_nonempty_string(),
        arb_nonempty_string(),
        proptest::option::of(1..65000u32),
    )
        .prop_map(|(id, name, image, cmd, state, pid)| ContainerInfo {
            id,
            name,
            image,
            command: cmd,
            state,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            pid,
        })
}

// ---------------------------------------------------------------------------
// DaemonRequest strategy
// ---------------------------------------------------------------------------

fn arb_daemon_request() -> impl Strategy<Value = DaemonRequest> {
    prop_oneof![
        // Run — split into two nested tuples to stay within proptest's 12-arity limit.
        (
            (
                arb_nonempty_string(),
                proptest::option::of(arb_nonempty_string()),
                proptest::collection::vec(arb_nonempty_string(), 0..3),
                proptest::option::of(1..1_000_000u64),
                proptest::option::of(1..10000u64),
                any::<bool>(),
                proptest::option::of(arb_network_mode()),
                proptest::collection::vec(arb_nonempty_string(), 0..3),
                proptest::collection::vec(arb_bind_mount(), 0..2),
            ),
            (
                any::<bool>(),
                proptest::option::of(arb_nonempty_string()),
                any::<bool>(),
                proptest::option::of(arb_nonempty_string()),
                proptest::option::of(arb_nonempty_string()),
                any::<bool>(),
                proptest::option::of(arb_priority()),
                proptest::option::of(arb_urgency()),
                proptest::option::of(arb_nonempty_string()),
            ),
        )
            .prop_map(
                |(
                    (image, tag, command, mem, cpu, eph, net, env, mounts),
                    (priv_, name, tty, ep, user, auto_remove, priority, urgency, platform),
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
                        name,
                        tty,
                        entrypoint: ep,
                        user,
                        auto_remove,
                        priority,
                        urgency,
                        execution_context: None,
                        platform,
                    }
                },
            ),
        // Stop
        arb_nonempty_string().prop_map(|id| DaemonRequest::Stop { id }),
        // PauseContainer
        arb_nonempty_string().prop_map(|id| DaemonRequest::PauseContainer { id }),
        // ResumeContainer
        arb_nonempty_string().prop_map(|id| DaemonRequest::ResumeContainer { id }),
        // Remove
        arb_nonempty_string().prop_map(|id| DaemonRequest::Remove { id }),
        // List
        Just(DaemonRequest::List),
        // Pull
        (
            arb_nonempty_string(),
            proptest::option::of(arb_nonempty_string()),
            proptest::option::of(arb_nonempty_string()),
        )
            .prop_map(|(image, tag, platform)| DaemonRequest::Pull {
                image,
                tag,
                platform,
            }),
        // LoadImage
        (
            arb_nonempty_string(),
            arb_nonempty_string(),
            arb_nonempty_string(),
        )
            .prop_map(|(path, name, tag)| DaemonRequest::LoadImage { path, name, tag }),
        // Exec
        (
            arb_nonempty_string(),
            proptest::collection::vec(arb_nonempty_string(), 1..3),
            proptest::collection::vec(arb_nonempty_string(), 0..2),
            proptest::option::of(arb_nonempty_string()),
            any::<bool>(),
            proptest::option::of(arb_nonempty_string()),
        )
            .prop_map(|(cid, cmd, env, wd, tty, user)| DaemonRequest::Exec {
                container_id: cid,
                cmd,
                env,
                working_dir: wd,
                tty,
                user,
            }),
        // SendInput
        (arb_session_id(), arb_nonempty_string()).prop_map(|(sid, data)| {
            DaemonRequest::SendInput {
                session_id: sid,
                data,
            }
        }),
        // ResizePty
        (arb_session_id(), 1..300u16, 1..100u16).prop_map(|(sid, cols, rows)| {
            DaemonRequest::ResizePty {
                session_id: sid,
                cols,
                rows,
            }
        }),
        // Push
        (arb_nonempty_string(), arb_push_credentials()).prop_map(|(ir, creds)| {
            DaemonRequest::Push {
                image_ref: ir,
                credentials: creds,
            }
        }),
        // SubscribeEvents
        Just(DaemonRequest::SubscribeEvents),
        // Prune
        any::<bool>().prop_map(|dr| DaemonRequest::Prune { dry_run: dr }),
        // ListImages
        Just(DaemonRequest::ListImages),
        // RemoveImage
        arb_nonempty_string().prop_map(|ir| DaemonRequest::RemoveImage { image_ref: ir }),
        // ContainerLogs
        (arb_nonempty_string(), any::<bool>()).prop_map(|(cid, follow)| {
            DaemonRequest::ContainerLogs {
                container_id: cid,
                follow,
            }
        }),
        // RunWorkflow
        arb_workflow_def().prop_map(DaemonRequest::RunWorkflow),
        // Update
        (
            proptest::collection::vec(arb_nonempty_string(), 0..3),
            any::<bool>(),
            any::<bool>(),
            any::<bool>(),
        )
            .prop_map(|(images, all, containers, restart)| DaemonRequest::Update {
                images,
                all,
                containers,
                restart,
            }),
        // GetManifest
        arb_nonempty_string().prop_map(|id| DaemonRequest::GetManifest { id }),
        // VerifyManifest
        (arb_nonempty_string(), arb_nonempty_string()).prop_map(|(id, pj)| {
            DaemonRequest::VerifyManifest {
                id,
                policy_json: pj,
            }
        }),
    ]
}

// ---------------------------------------------------------------------------
// DaemonResponse strategy
// ---------------------------------------------------------------------------

fn arb_daemon_response() -> impl Strategy<Value = DaemonResponse> {
    prop_oneof![
        arb_nonempty_string().prop_map(|id| DaemonResponse::ContainerCreated { id }),
        arb_nonempty_string().prop_map(|msg| DaemonResponse::Success { message: msg }),
        arb_nonempty_string().prop_map(|id| DaemonResponse::ContainerPaused { id }),
        arb_nonempty_string().prop_map(|id| DaemonResponse::ContainerResumed { id }),
        proptest::collection::vec(arb_container_info(), 0..3)
            .prop_map(|cs| DaemonResponse::ContainerList { containers: cs }),
        arb_nonempty_string().prop_map(|img| DaemonResponse::ImageLoaded { image: img }),
        arb_nonempty_string().prop_map(|msg| DaemonResponse::Error { message: msg }),
        (arb_output_stream_kind(), arb_nonempty_string())
            .prop_map(|(s, d)| DaemonResponse::ContainerOutput { stream: s, data: d }),
        (-128..128i32).prop_map(|ec| DaemonResponse::ContainerStopped { exit_code: ec }),
        arb_nonempty_string().prop_map(|eid| DaemonResponse::ExecStarted { exec_id: eid }),
        (arb_nonempty_string(), 0..1_000_000u64, 0..1_000_000u64).prop_map(|(ld, bu, tb)| {
            DaemonResponse::PushProgress {
                layer_digest: ld,
                bytes_uploaded: bu,
                total_bytes: tb,
            }
        }),
        (1..20u32, 1..20u32, arb_nonempty_string()).prop_map(|(s, ts, msg)| {
            DaemonResponse::BuildOutput {
                step: s,
                total_steps: ts,
                message: msg,
            }
        }),
        (arb_nonempty_string(), arb_nonempty_string())
            .prop_map(|(iid, tag)| { DaemonResponse::BuildComplete { image_id: iid, tag } }),
        arb_container_event().prop_map(|e| DaemonResponse::Event { event: e }),
        proptest::collection::vec(arb_nonempty_string(), 0..5)
            .prop_map(|imgs| DaemonResponse::ImageList { images: imgs }),
        (
            proptest::collection::vec(arb_nonempty_string(), 0..3),
            0..1_000_000u64,
            any::<bool>(),
        )
            .prop_map(|(r, fb, dr)| DaemonResponse::Pruned {
                removed: r,
                freed_bytes: fb,
                dry_run: dr,
            }),
        (arb_output_stream_kind(), arb_nonempty_string())
            .prop_map(|(s, l)| DaemonResponse::LogLine { stream: s, line: l }),
        arb_snapshot_info().prop_map(|info| DaemonResponse::SnapshotSaved { info }),
        (arb_nonempty_string(), arb_nonempty_string())
            .prop_map(|(id, name)| DaemonResponse::SnapshotRestored { id, name }),
        (
            arb_nonempty_string(),
            proptest::collection::vec(arb_snapshot_info(), 0..3),
        )
            .prop_map(|(id, snapshots)| DaemonResponse::SnapshotList { id, snapshots }),
        (arb_nonempty_string(), arb_nonempty_string())
            .prop_map(|(img, status)| DaemonResponse::UpdateProgress { image: img, status }),
        Just(DaemonResponse::Manifest {
            manifest: serde_json::json!({"test": true}),
        }),
        (any::<bool>(), proptest::option::of(arb_nonempty_string())).prop_map(|(a, r)| {
            DaemonResponse::VerifyResult {
                allowed: a,
                reason: r,
            }
        }),
        (arb_nonempty_string(), arb_step_status()).prop_map(|(alias, status)| {
            DaemonResponse::WorkflowStepComplete {
                alias,
                output: serde_json::Value::Null,
                status,
            }
        }),
        arb_phase_outcome().prop_map(|fp| DaemonResponse::WorkflowComplete { final_phase: fp }),
    ]
}

// ---------------------------------------------------------------------------
// ExecutionManifest strategy
// ---------------------------------------------------------------------------

fn arb_execution_manifest() -> impl Strategy<Value = ExecutionManifest> {
    (
        arb_nonempty_string(),
        arb_nonempty_string(),
        proptest::collection::vec(arb_nonempty_string(), 1..3),
        any::<bool>(),
        proptest::option::of(arb_nonempty_string()),
        any::<bool>(),
    )
        .prop_map(
            |(cid, image_ref, cmd, privileged, name, ephemeral)| ExecutionManifest {
                schema_version: 1,
                container_id: cid,
                created_at: "2026-01-01T00:00:00Z".to_string(),
                manifest_path: None,
                workload_digest: None,
                subject: ExecutionManifestSubject {
                    image_ref,
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
                    network_mode: "none".to_string(),
                    privileged,
                    platform: None,
                },
                request: ExecutionManifestRequest { name, ephemeral },
            },
        )
}

// ---------------------------------------------------------------------------
// Roundtrip property tests
// ---------------------------------------------------------------------------

/// Compare via JSON Value since DaemonRequest does not derive PartialEq.
fn json_roundtrip_eq<T: serde::Serialize + serde::de::DeserializeOwned>(val: &T) -> bool {
    let json = serde_json::to_string(val).expect("serialize should succeed");
    let back: T = serde_json::from_str(&json).expect("deserialize should succeed");
    let json2 = serde_json::to_string(&back).expect("re-serialize should succeed");
    json == json2
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    #[test]
    fn roundtrip_daemon_request(req in arb_daemon_request()) {
        prop_assert!(json_roundtrip_eq(&req));
    }

    #[test]
    fn roundtrip_daemon_response(resp in arb_daemon_response()) {
        prop_assert!(json_roundtrip_eq(&resp));
    }

    #[test]
    fn roundtrip_execution_manifest(manifest in arb_execution_manifest()) {
        // ExecutionManifest derives PartialEq, so compare directly.
        let json = serde_json::to_string(&manifest).expect("serialize");
        let back: ExecutionManifest = serde_json::from_str(&json).expect("deserialize");
        prop_assert_eq!(manifest, back);
    }
}
