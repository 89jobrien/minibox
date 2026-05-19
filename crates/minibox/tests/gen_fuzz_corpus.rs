//! Generates seed corpus files for the fuzz targets.
//!
//! Run with: `cargo test -p minibox --test gen_fuzz_corpus -- --nocapture`
//!
//! Writes encoded protocol messages to
//! `crates/minibox/fuzz/corpus/fuzz_decode_{request,response}/`.
//! These seed inputs let the fuzzer start from valid wire frames and mutate
//! outward, reaching deeper parser paths faster than starting from random bytes.

use minibox::protocol::{
    DaemonRequest, DaemonResponse, OutputStreamKind, encode_request, encode_response,
};
use std::path::Path;

fn corpus_dir(target: &str) -> std::path::PathBuf {
    // Walk up from the test binary's cwd to find the workspace root,
    // then construct the corpus path.
    let mut dir = std::env::current_dir().expect("cwd");
    // In nextest the cwd is the workspace root; in plain `cargo test` it may
    // be the crate root. Handle both.
    if dir.join("Cargo.lock").exists() {
        dir.join(format!("crates/minibox/fuzz/seeds/{target}"))
    } else {
        // crate root
        dir.join(format!("fuzz/seeds/{target}"))
    }
}

fn write_seed(dir: &Path, name: &str, bytes: Vec<u8>) {
    std::fs::create_dir_all(dir).expect("create corpus dir");
    std::fs::write(dir.join(name), bytes).expect("write seed");
}

#[test]
fn generate_request_corpus() {
    let dir = corpus_dir("fuzz_decode_request");

    let seeds: Vec<(&str, DaemonRequest)> = vec![
        ("list", DaemonRequest::List),
        ("subscribe_events", DaemonRequest::SubscribeEvents),
        ("prune_dry", DaemonRequest::Prune { dry_run: true }),
        ("prune_wet", DaemonRequest::Prune { dry_run: false }),
        (
            "stop",
            DaemonRequest::Stop {
                id: "deadbeefdeadbeef".into(),
            },
        ),
        (
            "remove",
            DaemonRequest::Remove {
                id: "deadbeefdeadbeef".into(),
            },
        ),
        (
            "pause",
            DaemonRequest::PauseContainer {
                id: "deadbeefdeadbeef".into(),
            },
        ),
        (
            "resume",
            DaemonRequest::ResumeContainer {
                id: "deadbeefdeadbeef".into(),
            },
        ),
        (
            "pull_no_tag",
            DaemonRequest::Pull {
                image: "alpine".into(),
                tag: None,
                platform: None,
            },
        ),
        (
            "pull_tagged",
            DaemonRequest::Pull {
                image: "library/alpine".into(),
                tag: Some("3.19".into()),
                platform: Some("linux/amd64".into()),
            },
        ),
        (
            "remove_image",
            DaemonRequest::RemoveImage {
                image_ref: "alpine:latest".into(),
            },
        ),
        (
            "run_minimal",
            DaemonRequest::Run {
                image: "alpine".into(),
                tag: None,
                command: vec!["/bin/sh".into()],
                memory_limit_bytes: None,
                cpu_weight: None,
                ephemeral: false,
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
            },
        ),
        (
            "run_full",
            DaemonRequest::Run {
                image: "library/ubuntu".into(),
                tag: Some("22.04".into()),
                command: vec!["bash".into(), "-c".into(), "echo hello".into()],
                memory_limit_bytes: Some(512 * 1024 * 1024),
                cpu_weight: Some(512),
                ephemeral: true,
                network: None,
                mounts: vec![],
                privileged: false,
                env: vec!["FOO=bar".into(), "BAZ=qux".into()],
                name: Some("my-container".into()),
                tty: true,
                entrypoint: Some("/bin/bash".into()),
                user: Some("1000:1000".into()),
                auto_remove: true,
                priority: None,
                urgency: None,
                execution_context: None,
                platform: Some("linux/amd64".into()),
            },
        ),
        (
            "logs",
            DaemonRequest::ContainerLogs {
                container_id: "deadbeefdeadbeef".into(),
                follow: false,
            },
        ),
        (
            "logs_follow",
            DaemonRequest::ContainerLogs {
                container_id: "deadbeefdeadbeef".into(),
                follow: true,
            },
        ),
    ];

    for (name, req) in seeds {
        let bytes = encode_request(&req).expect("encode_request");
        write_seed(&dir, name, bytes);
    }

    println!("wrote {} request seeds to {}", 16, dir.display());
}

#[test]
fn generate_response_corpus() {
    let dir = corpus_dir("fuzz_decode_response");

    let seeds: Vec<(&str, DaemonResponse)> = vec![
        (
            "created",
            DaemonResponse::ContainerCreated {
                id: "deadbeefdeadbeef".into(),
            },
        ),
        (
            "success",
            DaemonResponse::Success {
                message: "container removed".into(),
            },
        ),
        (
            "error_short",
            DaemonResponse::Error {
                message: "not found".into(),
            },
        ),
        (
            "error_long",
            DaemonResponse::Error {
                message: "container deadbeef not found: no such container".into(),
            },
        ),
        (
            "list_empty",
            DaemonResponse::ContainerList { containers: vec![] },
        ),
        (
            "paused",
            DaemonResponse::ContainerPaused {
                id: "deadbeefdeadbeef".into(),
            },
        ),
        (
            "resumed",
            DaemonResponse::ContainerResumed {
                id: "deadbeefdeadbeef".into(),
            },
        ),
        (
            "output_stdout",
            DaemonResponse::ContainerOutput {
                stream: OutputStreamKind::Stdout,
                data: "hello world\n".into(),
            },
        ),
        (
            "output_stderr",
            DaemonResponse::ContainerOutput {
                stream: OutputStreamKind::Stderr,
                data: "error: something went wrong\n".into(),
            },
        ),
        (
            "stopped_zero",
            DaemonResponse::ContainerStopped { exit_code: 0 },
        ),
        (
            "stopped_nonzero",
            DaemonResponse::ContainerStopped { exit_code: 1 },
        ),
        (
            "stopped_signal",
            DaemonResponse::ContainerStopped { exit_code: -1 },
        ),
        (
            "image_loaded",
            DaemonResponse::ImageLoaded {
                image: "alpine:latest".into(),
            },
        ),
        (
            "push_progress",
            DaemonResponse::PushProgress {
                layer_digest: "sha256:deadbeef".into(),
                bytes_uploaded: 1024,
                total_bytes: 65536,
            },
        ),
        (
            "build_output",
            DaemonResponse::BuildOutput {
                step: 1,
                total_steps: 5,
                message: "Step 1/5: FROM alpine".into(),
            },
        ),
        (
            "build_complete",
            DaemonResponse::BuildComplete {
                image_id: "sha256:abc123".into(),
                tag: "myapp:latest".into(),
            },
        ),
        (
            "pruned",
            DaemonResponse::Pruned {
                removed: vec!["alpine:3.18".into()],
                freed_bytes: 5 * 1024 * 1024,
                dry_run: false,
            },
        ),
        (
            "log_stdout",
            DaemonResponse::LogLine {
                stream: OutputStreamKind::Stdout,
                line: "2026-01-01 container started".into(),
            },
        ),
    ];

    for (name, resp) in seeds {
        let bytes = encode_response(&resp).expect("encode_response");
        write_seed(&dir, name, bytes);
    }

    println!("wrote 18 response seeds to {}", dir.display());
}
