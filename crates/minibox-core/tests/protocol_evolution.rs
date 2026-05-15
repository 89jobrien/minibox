//! Protocol evolution guard tests.
//!
//! These tests enforce invariants that are easy to violate when adding new
//! protocol variants:
//!
//! 1. Every `DaemonResponse` variant round-trips through serde without loss.
//! 2. Every `DaemonRequest` variant with `#[serde(default)]` fields tolerates
//!    omitted keys in JSON (backward-compatible deserialization).
//! 3. Non-streaming `DaemonResponse` variants are explicitly classified as
//!    terminal here — when `is_terminal_response` in `minibox/src/daemon/server.rs` is
//!    updated, this test must be updated in the same commit to keep the two in
//!    sync.
//!
//! # How to extend
//!
//! When adding a new `DaemonResponse` variant:
//! - Add it to `all_response_variants()`.
//! - Decide terminal vs non-terminal and add it to the appropriate list in
//!   `test_terminal_classification_is_exhaustive`.
//!
//! When adding a new `DaemonRequest` variant with optional/defaulted fields:
//! - Add a case to `test_request_serde_default_fields`.

use minibox_core::domain::SnapshotInfo;
use minibox_core::events::ContainerEvent;
use minibox_core::protocol::{ContainerInfo, DaemonRequest, DaemonResponse, OutputStreamKind};
use std::time::SystemTime;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build one representative of every `DaemonResponse` variant.
///
/// This list is the source of truth for the test suite.  Keep it in sync with
/// the actual enum definition in `minibox-core/src/protocol.rs`.
fn all_response_variants() -> Vec<DaemonResponse> {
    vec![
        // --- terminal variants ---
        DaemonResponse::ContainerCreated {
            id: "abc123".to_string(),
        },
        DaemonResponse::Success {
            message: "ok".to_string(),
        },
        DaemonResponse::Error {
            message: "something went wrong".to_string(),
        },
        DaemonResponse::ContainerList {
            containers: vec![ContainerInfo {
                id: "abc123".to_string(),
                name: Some("test".to_string()),
                image: "alpine".to_string(),
                command: "/bin/sh".to_string(),
                state: "running".to_string(),
                created_at: "2026-01-01T00:00:00Z".to_string(),
                pid: Some(42),
            }],
        },
        DaemonResponse::ContainerStopped { exit_code: 0 },
        DaemonResponse::ImageLoaded {
            image: "minibox-tester:latest".to_string(),
        },
        DaemonResponse::BuildComplete {
            image_id: "sha256:deadbeef".to_string(),
            tag: "my-image:latest".to_string(),
        },
        DaemonResponse::ContainerPaused {
            id: "abc123".to_string(),
        },
        DaemonResponse::ContainerResumed {
            id: "abc123".to_string(),
        },
        DaemonResponse::Pruned {
            removed: vec!["alpine:latest".to_string()],
            freed_bytes: 1024,
            dry_run: false,
        },
        // --- non-terminal (streaming) variants ---
        DaemonResponse::ContainerOutput {
            stream: OutputStreamKind::Stdout,
            data: "aGVsbG8=".to_string(),
        },
        DaemonResponse::ExecStarted {
            exec_id: "exec-abc".to_string(),
        },
        DaemonResponse::PushProgress {
            layer_digest: "sha256:deadbeef".to_string(),
            bytes_uploaded: 512,
            total_bytes: 1024,
        },
        DaemonResponse::BuildOutput {
            step: 1,
            total_steps: 3,
            message: "RUN echo hello".to_string(),
        },
        DaemonResponse::Event {
            event: ContainerEvent::Started {
                id: "abc123".to_string(),
                pid: 42,
                timestamp: SystemTime::UNIX_EPOCH,
            },
        },
        DaemonResponse::LogLine {
            stream: OutputStreamKind::Stderr,
            line: "error: oops".to_string(),
        },
        // --- terminal variants ---
        DaemonResponse::PipelineComplete {
            trace: serde_json::json!({
                "id": "01HYX-test-trace",
                "steps": [],
                "result": "ok"
            }),
            container_id: "abc123def456".into(),
            exit_code: 0,
        },
        // --- non-terminal (streaming) ---
        DaemonResponse::UpdateProgress {
            image: "alpine:latest".to_string(),
            status: "up to date".to_string(),
        },
        // --- terminal variants added after initial list ---
        DaemonResponse::ImageList {
            images: vec!["alpine:latest".to_string()],
        },
        DaemonResponse::SnapshotSaved {
            info: SnapshotInfo {
                container_id: "abc123".to_string(),
                name: "snap-1".to_string(),
                created_at: "2026-01-01T00:00:00Z".to_string(),
                adapter: "smolvm".to_string(),
                image: "alpine:latest".to_string(),
                size_bytes: 1024,
            },
        },
        DaemonResponse::SnapshotRestored {
            id: "abc123".to_string(),
            name: "snap-1".to_string(),
        },
        DaemonResponse::SnapshotList {
            id: "abc123".to_string(),
            snapshots: vec![],
        },
    ]
}

// ---------------------------------------------------------------------------
// Test 1: every DaemonResponse variant round-trips through serde
// ---------------------------------------------------------------------------

#[test]
fn test_all_response_variants_serde_roundtrip() {
    for variant in all_response_variants() {
        let json = serde_json::to_string(&variant)
            .unwrap_or_else(|e| panic!("serialize failed for {variant:?}: {e}"));

        let decoded: DaemonResponse = serde_json::from_str(&json)
            .unwrap_or_else(|e| panic!("deserialize failed for json={json:?}: {e}"));

        // Re-serialize the decoded value and compare byte-for-byte to verify
        // no data was silently dropped during the round-trip.
        let json2 =
            serde_json::to_string(&decoded).unwrap_or_else(|e| panic!("re-serialize failed: {e}"));

        assert_eq!(
            json, json2,
            "serde round-trip changed representation for variant: {variant:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// Test 2: DaemonRequest fields marked #[serde(default)] tolerate omission
// ---------------------------------------------------------------------------

/// Verifies that a JSON Run request that omits all optional fields still
/// deserializes successfully with the expected defaults.
#[test]
fn test_request_run_backward_compat_omits_optional_fields() {
    // Minimal Run — only the fields that were present in the original protocol
    // (no ephemeral, network, env, mounts, privileged, name, tty).
    let json = r#"{"type":"Run","image":"alpine","tag":null,"command":["/bin/sh"],"memory_limit_bytes":null,"cpu_weight":null}"#;

    let req: DaemonRequest =
        serde_json::from_str(json).expect("backward-compat Run deserialization failed");

    match req {
        DaemonRequest::Run {
            image,
            ephemeral,
            network,
            env,
            mounts,
            privileged,
            name,
            tty,
            ..
        } => {
            assert_eq!(image, "alpine");
            assert!(!ephemeral, "ephemeral should default to false");
            assert!(network.is_none(), "network should default to None");
            assert!(env.is_empty(), "env should default to []");
            assert!(mounts.is_empty(), "mounts should default to []");
            assert!(!privileged, "privileged should default to false");
            assert!(name.is_none(), "name should default to None");
            assert!(!tty, "tty should default to false");
        }
        other => panic!("expected DaemonRequest::Run, got {other:?}"),
    }
}

/// Verifies that a Exec request omitting the optional fields deserializes with
/// correct defaults.
#[test]
fn test_request_exec_backward_compat_omits_optional_fields() {
    let json = r#"{"type":"Exec","container_id":"abc123","cmd":["/bin/sh"]}"#;

    let req: DaemonRequest =
        serde_json::from_str(json).expect("backward-compat Exec deserialization failed");

    match req {
        DaemonRequest::Exec {
            env,
            working_dir,
            tty,
            ..
        } => {
            assert!(env.is_empty(), "env should default to []");
            assert!(working_dir.is_none(), "working_dir should default to None");
            assert!(!tty, "tty should default to false");
        }
        other => panic!("expected DaemonRequest::Exec, got {other:?}"),
    }
}

/// Verifies that a Prune request omitting `dry_run` deserializes with false.
#[test]
fn test_request_prune_backward_compat_omits_dry_run() {
    let json = r#"{"type":"Prune"}"#;

    let req: DaemonRequest =
        serde_json::from_str(json).expect("backward-compat Prune deserialization failed");

    match req {
        DaemonRequest::Prune { dry_run } => {
            assert!(!dry_run, "dry_run should default to false");
        }
        other => panic!("expected DaemonRequest::Prune, got {other:?}"),
    }
}

/// Verifies that a Commit request omitting optional string fields deserializes
/// with None / empty defaults.
#[test]
fn test_request_commit_backward_compat_omits_optional_fields() {
    let json = r#"{"type":"Commit","container_id":"abc123","target_image":"my-img:latest"}"#;

    let req: DaemonRequest =
        serde_json::from_str(json).expect("backward-compat Commit deserialization failed");

    match req {
        DaemonRequest::Commit {
            author,
            message,
            env_overrides,
            cmd_override,
            ..
        } => {
            assert!(author.is_none());
            assert!(message.is_none());
            assert!(env_overrides.is_empty());
            assert!(cmd_override.is_none());
        }
        other => panic!("expected DaemonRequest::Commit, got {other:?}"),
    }
}

/// Verifies Build omitting `build_args` and `no_cache` deserializes with
/// defaults.
#[test]
fn test_request_build_backward_compat_omits_optional_fields() {
    let json = r#"{"type":"Build","dockerfile":"FROM alpine","context_path":"/tmp/ctx","tag":"my:latest"}"#;

    let req: DaemonRequest =
        serde_json::from_str(json).expect("backward-compat Build deserialization failed");

    match req {
        DaemonRequest::Build {
            build_args,
            no_cache,
            ..
        } => {
            assert!(build_args.is_empty());
            assert!(!no_cache);
        }
        other => panic!("expected DaemonRequest::Build, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Test 3: terminal-vs-non-terminal classification is exhaustive
//
// This test mirrors the match in `minibox/src/daemon/src/server.rs::is_terminal_response`.
// When a new variant is added, the compiler will force an update here via the
// exhaustive match below.
// ---------------------------------------------------------------------------

/// Returns `true` for variants that terminate a request/response exchange
/// (i.e. after which the server closes the connection).
///
/// This MUST stay in sync with `is_terminal_response` in
/// `crates/minibox/src/daemon/src/server.rs`.  If the two diverge, one of the tests
/// below will catch it during review.
fn classify_terminal(r: &DaemonResponse) -> bool {
    match r {
        // --- terminal ---
        DaemonResponse::ContainerStopped { .. }
        | DaemonResponse::Error { .. }
        | DaemonResponse::Success { .. }
        | DaemonResponse::ContainerList { .. }
        | DaemonResponse::ImageLoaded { .. }
        | DaemonResponse::BuildComplete { .. }
        | DaemonResponse::ContainerPaused { .. }
        | DaemonResponse::ContainerResumed { .. }
        | DaemonResponse::Pruned { .. }
        | DaemonResponse::PipelineComplete { .. }
        | DaemonResponse::SnapshotSaved { .. }
        | DaemonResponse::SnapshotRestored { .. }
        | DaemonResponse::SnapshotList { .. }
        | DaemonResponse::ImageList { .. } => true,

        // --- non-terminal (streaming) ---
        DaemonResponse::ContainerCreated { .. }
        | DaemonResponse::ContainerOutput { .. }
        | DaemonResponse::ExecStarted { .. }
        | DaemonResponse::PushProgress { .. }
        | DaemonResponse::BuildOutput { .. }
        | DaemonResponse::Event { .. }
        | DaemonResponse::LogLine { .. }
        | DaemonResponse::UpdateProgress { .. } => false,

        // Manifest inspection — terminal (single response per request).
        DaemonResponse::Manifest { .. } | DaemonResponse::VerifyResult { .. } => true,
    }
}

#[test]
fn test_terminal_classification_is_exhaustive() {
    // Terminal variants — must each return true from classify_terminal.
    let terminal_variants: Vec<DaemonResponse> = vec![
        DaemonResponse::ContainerStopped { exit_code: 1 },
        DaemonResponse::Error {
            message: "err".to_string(),
        },
        DaemonResponse::Success {
            message: "ok".to_string(),
        },
        DaemonResponse::ContainerList { containers: vec![] },
        DaemonResponse::ImageLoaded {
            image: "x:latest".to_string(),
        },
        DaemonResponse::BuildComplete {
            image_id: "sha256:aabb".to_string(),
            tag: "x:1".to_string(),
        },
        DaemonResponse::ContainerPaused {
            id: "x".to_string(),
        },
        DaemonResponse::ContainerResumed {
            id: "x".to_string(),
        },
        DaemonResponse::Pruned {
            removed: vec![],
            freed_bytes: 0,
            dry_run: true,
        },
        DaemonResponse::PipelineComplete {
            trace: serde_json::json!({"steps": [], "result": "ok"}),
            container_id: "abc123def456".into(),
            exit_code: 0,
        },
        DaemonResponse::ImageList {
            images: vec!["alpine:latest".to_string()],
        },
        DaemonResponse::SnapshotSaved {
            info: SnapshotInfo {
                container_id: "abc123".to_string(),
                name: "snap-1".to_string(),
                created_at: "2026-01-01T00:00:00Z".to_string(),
                adapter: "smolvm".to_string(),
                image: "alpine:latest".to_string(),
                size_bytes: 1024,
            },
        },
        DaemonResponse::SnapshotRestored {
            id: "abc123".to_string(),
            name: "snap-1".to_string(),
        },
        DaemonResponse::SnapshotList {
            id: "abc123".to_string(),
            snapshots: vec![],
        },
    ];

    for v in &terminal_variants {
        assert!(
            classify_terminal(v),
            "expected terminal, got non-terminal for: {v:?}"
        );
    }

    // Non-terminal variants — must each return false from classify_terminal.
    let non_terminal_variants: Vec<DaemonResponse> = vec![
        DaemonResponse::ContainerCreated {
            id: "x".to_string(),
        },
        DaemonResponse::ContainerOutput {
            stream: OutputStreamKind::Stdout,
            data: "aGk=".to_string(),
        },
        DaemonResponse::ExecStarted {
            exec_id: "e1".to_string(),
        },
        DaemonResponse::PushProgress {
            layer_digest: "sha256:aa".to_string(),
            bytes_uploaded: 0,
            total_bytes: 100,
        },
        DaemonResponse::BuildOutput {
            step: 1,
            total_steps: 2,
            message: "step".to_string(),
        },
        DaemonResponse::Event {
            event: ContainerEvent::Stopped {
                id: "x".to_string(),
                exit_code: 0,
                timestamp: SystemTime::UNIX_EPOCH,
            },
        },
        DaemonResponse::LogLine {
            stream: OutputStreamKind::Stderr,
            line: "hello".to_string(),
        },
        DaemonResponse::UpdateProgress {
            image: "alpine:latest".to_string(),
            status: "updated".to_string(),
        },
    ];

    for v in &non_terminal_variants {
        assert!(
            !classify_terminal(v),
            "expected non-terminal, got terminal for: {v:?}"
        );
    }

    // Verify every variant in all_response_variants() is covered by one of
    // the two lists above.
    let total_covered = terminal_variants.len() + non_terminal_variants.len();
    let total_defined = all_response_variants().len();
    assert_eq!(
        total_covered, total_defined,
        "all_response_variants() has {total_defined} entries but only \
         {total_covered} are classified — add the missing variant(s) to one \
         of the two lists above"
    );
}

// ---------------------------------------------------------------------------
// Test 4: RunPipeline backward compatibility
// ---------------------------------------------------------------------------

/// Verifies that a RunPipeline request omitting all optional fields still
/// deserializes with the expected defaults.
#[test]
fn test_request_run_pipeline_backward_compat_omits_optional_fields() {
    let json = r#"{"type":"RunPipeline","pipeline_path":"work.cruxx"}"#;

    let req: DaemonRequest =
        serde_json::from_str(json).expect("backward-compat RunPipeline deserialization failed");

    match req {
        DaemonRequest::RunPipeline {
            pipeline_path,
            input,
            image,
            budget,
            env,
            max_depth,
            ..
        } => {
            assert_eq!(pipeline_path, "work.cruxx");
            assert!(input.is_none(), "input should default to None");
            assert!(image.is_none(), "image should default to None");
            assert!(budget.is_none(), "budget should default to None");
            assert!(env.is_empty(), "env should default to []");
            assert_eq!(max_depth, 3, "max_depth should default to 3");
        }
        other => panic!("expected DaemonRequest::RunPipeline, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Test 5: RunPipeline snapshot tests
// ---------------------------------------------------------------------------

#[test]
fn run_pipeline_request_snapshot() {
    let req = DaemonRequest::RunPipeline {
        pipeline_path: "/workspace/.cruxx/pipelines/work.cruxx".into(),
        input: Some(serde_json::json!({"prompt": "hello"})),
        image: None,
        budget: None,
        env: vec![("CRUX_LOG".into(), "debug".into())],
        max_depth: 3,
        priority: None,
        urgency: None,
        execution_context: None,
    };
    insta::assert_json_snapshot!(req);
}

#[test]
fn run_pipeline_request_minimal_snapshot() {
    let req = DaemonRequest::RunPipeline {
        pipeline_path: "work.cruxx".into(),
        input: None,
        image: None,
        budget: None,
        env: vec![],
        max_depth: 3,
        priority: None,
        urgency: None,
        execution_context: None,
    };
    insta::assert_json_snapshot!(req);
}

// ---------------------------------------------------------------------------
// Test 6: PipelineComplete response snapshot
// ---------------------------------------------------------------------------

#[test]
fn pipeline_complete_response_snapshot() {
    let resp = DaemonResponse::PipelineComplete {
        trace: serde_json::json!({
            "id": "01HYX-test-trace",
            "steps": [],
            "result": "ok"
        }),
        container_id: "abc123def456".into(),
        exit_code: 0,
    };
    insta::assert_json_snapshot!(resp);
}
