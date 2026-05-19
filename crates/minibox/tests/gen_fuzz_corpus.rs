//! Generates seed corpus files for the fuzz targets.
//!
//! Run with: `cargo test -p minibox --test gen_fuzz_corpus -- --nocapture`
//!
//! Writes encoded protocol messages to
//! `crates/minibox/fuzz/corpus/fuzz_decode_{request,response}/`.
//! These seed inputs let the fuzzer start from valid wire frames and mutate
//! outward, reaching deeper parser paths faster than starting from random bytes.

use flate2::{Compression, write::GzEncoder};
use minibox::protocol::{
    DaemonRequest, DaemonResponse, OutputStreamKind, encode_request, encode_response,
};
use std::io::Write as _;
use std::path::Path;

fn corpus_dir(target: &str) -> std::path::PathBuf {
    // Walk up from the test binary's cwd to find the workspace root,
    // then construct the corpus path.
    let dir = std::env::current_dir().expect("cwd");
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

// ---------------------------------------------------------------------------
// Layer extraction seeds
// ---------------------------------------------------------------------------

/// Build a minimal valid gzip-compressed tar with a single regular file.
fn tar_gz_regular(name: &str, content: &[u8]) -> Vec<u8> {
    let gz = GzEncoder::new(Vec::new(), Compression::default());
    let mut ar = tar::Builder::new(gz);
    let mut h = tar::Header::new_gnu();
    h.set_path(name).expect("set_path");
    h.set_size(content.len() as u64);
    h.set_entry_type(tar::EntryType::Regular);
    h.set_mode(0o644);
    h.set_cksum();
    ar.append(&h, content).expect("append");
    ar.into_inner().expect("inner").finish().expect("finish gz")
}

/// Build a gzip-compressed tar with a symlink entry.
fn tar_gz_symlink(name: &str, target: &str) -> Vec<u8> {
    let gz = GzEncoder::new(Vec::new(), Compression::default());
    let mut ar = tar::Builder::new(gz);
    let mut h = tar::Header::new_gnu();
    h.set_path(name).expect("set_path");
    h.set_size(0);
    h.set_entry_type(tar::EntryType::Symlink);
    h.set_link_name(target).expect("set_link_name");
    h.set_mode(0o777);
    h.set_cksum();
    ar.append(&h, &[][..]).expect("append");
    ar.into_inner().expect("inner").finish().expect("finish gz")
}

/// Build a raw tar.gz with a manually crafted header so we can embed filenames
/// that the `tar` crate's builder would reject (e.g. `../`).
fn raw_tar_gz(filename: &str) -> Vec<u8> {
    let mut header = [0u8; 512];
    let name = filename.as_bytes();
    let len = name.len().min(100);
    header[..len].copy_from_slice(&name[..len]);
    header[100..108].copy_from_slice(b"0000644\0");
    header[108..116].copy_from_slice(b"0000000\0");
    header[116..124].copy_from_slice(b"0000000\0");
    header[124..136].copy_from_slice(b"00000000000\0");
    header[136..148].copy_from_slice(b"00000000000\0");
    header[156] = b'0';
    header[257..263].copy_from_slice(b"ustar ");
    header[263..265].copy_from_slice(b" \0");
    header[148..156].fill(b' ');
    let sum: u32 = header.iter().map(|&b| b as u32).sum();
    let cksum = format!("{sum:06o}\0 ");
    header[148..156].copy_from_slice(cksum.as_bytes());

    let mut tar_bytes = Vec::new();
    tar_bytes.extend_from_slice(&header);
    tar_bytes.extend_from_slice(&[0u8; 1024]); // two EOA blocks

    let mut gz = GzEncoder::new(Vec::new(), Compression::default());
    gz.write_all(&tar_bytes).expect("write gz");
    gz.finish().expect("finish gz")
}

/// Write a seed for fuzz_validate_tar_path: raw path bytes (not tar).
fn path_seed(s: &str) -> Vec<u8> {
    s.as_bytes().to_vec()
}

#[test]
fn generate_layer_corpus() {
    // --- fuzz_extract_layer seeds ---
    let dir = corpus_dir("fuzz_extract_layer");

    let layer_seeds: Vec<(&str, Vec<u8>)> = vec![
        // valid inputs — teach fuzzer the gzip+tar format
        ("valid_regular", tar_gz_regular("hello.txt", b"hello world")),
        (
            "valid_nested",
            tar_gz_regular("usr/bin/tool", b"binary content"),
        ),
        ("valid_empty_file", tar_gz_regular("empty", b"")),
        // relative symlink — accepted
        ("valid_symlink_relative", tar_gz_symlink("link", "target")),
        // absolute symlink — rewrites and accepts
        (
            "valid_symlink_absolute",
            tar_gz_symlink("bin/sh", "/bin/bash"),
        ),
        // adversarial inputs — must be rejected cleanly (no panic)
        ("dotdot_escape", raw_tar_gz("../escape.txt")),
        ("dotdot_deep", raw_tar_gz("a/b/../../etc/passwd")),
        ("absolute_path", raw_tar_gz("/etc/passwd")),
        // empty / garbage bytes
        ("empty_input", vec![]),
        ("garbage", b"this is not gzip data at all".to_vec()),
        ("partial_gzip_header", vec![0x1f, 0x8b]),
    ];

    for (name, bytes) in &layer_seeds {
        write_seed(&dir, name, bytes.clone());
    }

    println!(
        "wrote {} fuzz_extract_layer seeds to {}",
        layer_seeds.len(),
        dir.display()
    );

    // --- fuzz_validate_tar_path seeds ---
    let path_dir = corpus_dir("fuzz_validate_tar_path");

    let path_seeds: Vec<(&str, Vec<u8>)> = vec![
        // accepted
        ("simple", path_seed("hello.txt")),
        ("nested", path_seed("usr/bin/env")),
        ("dot_component", path_seed("./foo")),
        ("deep", path_seed("a/b/c/d/e/f")),
        // rejected — .. traversal
        ("dotdot_prefix", path_seed("../escape")),
        ("dotdot_middle", path_seed("foo/../../etc/passwd")),
        ("bare_dotdot", path_seed("..")),
        // rejected — absolute
        ("absolute", path_seed("/etc/passwd")),
        ("absolute_nested", path_seed("/usr/bin/env")),
        // edge cases
        ("empty", path_seed("")),
        ("null_byte", b"foo\x00bar".to_vec()),
        ("dot", path_seed(".")),
        ("dotdot_as_filename", path_seed("foo..bar")),
    ];

    for (name, bytes) in &path_seeds {
        write_seed(&path_dir, name, bytes.clone());
    }

    println!(
        "wrote {} fuzz_validate_tar_path seeds to {}",
        path_seeds.len(),
        path_dir.display()
    );
}

// ---------------------------------------------------------------------------
// Manifest / platform / image-ref parsing seeds (tier 3)
// ---------------------------------------------------------------------------

/// Build a fuzz_parse_manifest seed: first byte = selector (0=single,1=list),
/// remaining bytes = JSON body.
fn manifest_seed(selector: u8, json: &str) -> Vec<u8> {
    let mut v = vec![selector];
    v.extend_from_slice(json.as_bytes());
    v
}

#[test]
fn generate_manifest_corpus() {
    // --- fuzz_parse_manifest seeds ---
    let dir = corpus_dir("fuzz_parse_manifest");

    let oci_single = r#"{
        "schemaVersion": 2,
        "mediaType": "application/vnd.oci.image.manifest.v1+json",
        "config": {"mediaType": "application/vnd.oci.image.config.v1+json",
                   "size": 1234, "digest": "sha256:abc123"},
        "layers": [
            {"mediaType": "application/vnd.oci.image.layer.v1.tar+gzip",
             "size": 5678, "digest": "sha256:def456"}
        ]
    }"#;

    let docker_single = r#"{
        "schemaVersion": 2,
        "mediaType": "application/vnd.docker.distribution.manifest.v2+json",
        "config": {"mediaType": "application/vnd.docker.container.image.v1+json",
                   "size": 100, "digest": "sha256:cfg001"},
        "layers": []
    }"#;

    let manifest_list = r#"{
        "schemaVersion": 2,
        "mediaType": "application/vnd.docker.distribution.manifest.list.v2+json",
        "manifests": [
            {"mediaType": "application/vnd.docker.distribution.manifest.v2+json",
             "size": 528, "digest": "sha256:amd64",
             "platform": {"architecture": "amd64", "os": "linux"}},
            {"mediaType": "application/vnd.docker.distribution.manifest.v2+json",
             "size": 528, "digest": "sha256:arm64",
             "platform": {"architecture": "arm64", "os": "linux", "variant": "v8"}}
        ]
    }"#;

    let oci_index = r#"{
        "schemaVersion": 2,
        "mediaType": "application/vnd.oci.image.index.v1+json",
        "manifests": [
            {"mediaType": "application/vnd.oci.image.manifest.v1+json",
             "size": 1000, "digest": "sha256:arm_manifest",
             "platform": {"architecture": "arm64", "os": "linux", "variant": "v8"}}
        ]
    }"#;

    let manifest_seeds: Vec<(&str, Vec<u8>)> = vec![
        // selector byte 0x00 = single, 0x01 = list
        ("oci_single", manifest_seed(0x00, oci_single)),
        ("docker_single", manifest_seed(0x00, docker_single)),
        (
            "empty_layers",
            manifest_seed(
                0x00,
                r#"{"schemaVersion":2,"mediaType":"","config":{"mediaType":"","size":0,"digest":"sha256:0"},"layers":[]}"#,
            ),
        ),
        ("manifest_list", manifest_seed(0x01, manifest_list)),
        ("oci_index", manifest_seed(0x01, oci_index)),
        (
            "empty_manifests",
            manifest_seed(0x01, r#"{"schemaVersion":2,"mediaType":"","manifests":[]}"#),
        ),
        // adversarial
        ("garbage_single", manifest_seed(0x00, "not json at all")),
        ("garbage_list", manifest_seed(0x01, "{broken")),
        ("empty_body", manifest_seed(0x00, "")),
        (
            "huge_array",
            manifest_seed(
                0x01,
                r#"{"schemaVersion":2,"mediaType":"","manifests":[{"mediaType":"","size":0,"digest":"sha256:x"},{"mediaType":"","size":0,"digest":"sha256:y"}]}"#,
            ),
        ),
    ];

    for (name, bytes) in &manifest_seeds {
        write_seed(&dir, name, bytes.clone());
    }
    println!(
        "wrote {} fuzz_parse_manifest seeds to {}",
        manifest_seeds.len(),
        dir.display()
    );

    // --- fuzz_parse_platform seeds ---
    let plat_dir = corpus_dir("fuzz_parse_platform");

    let platform_seeds: &[(&str, &str)] = &[
        ("linux_amd64", "linux/amd64"),
        ("linux_arm64", "linux/arm64"),
        ("linux_arm64_v8", "linux/arm64/v8"),
        ("linux_arm_v7", "linux/arm/v7"),
        ("windows_amd64", "windows/amd64"),
        ("normalized_x86_64", "linux/x86_64"),
        ("normalized_aarch64", "linux/aarch64"),
        ("empty", ""),
        ("only_os", "linux"),
        ("empty_os", "/amd64"),
        ("empty_arch", "linux/"),
        ("empty_variant", "linux/amd64/"),
        ("too_many_parts", "linux/amd64/v8/extra"),
    ];

    for (name, s) in platform_seeds {
        write_seed(&plat_dir, name, s.as_bytes().to_vec());
    }
    println!(
        "wrote {} fuzz_parse_platform seeds to {}",
        platform_seeds.len(),
        plat_dir.display()
    );

    // --- fuzz_parse_image_ref seeds ---
    let ref_dir = corpus_dir("fuzz_parse_image_ref");

    let image_ref_seeds: &[(&str, &str)] = &[
        ("alpine", "alpine"),
        ("alpine_latest", "alpine:latest"),
        ("alpine_3_19", "alpine:3.19"),
        ("org_image", "myorg/myimage"),
        ("org_image_tag", "myorg/myimage:v2"),
        ("ghcr_full", "ghcr.io/org/minibox-rust-ci:stable"),
        ("localhost", "localhost/myns/myimage:latest"),
        ("docker_io_explicit", "docker.io/library/alpine:latest"),
        // adversarial
        ("empty", ""),
        ("only_colon", ":"),
        ("only_slash", "/"),
        ("double_colon", "alpine::latest"),
        ("null_byte", "alp\x00ine"),
        ("just_tag", ":latest"),
        ("ghcr_no_namespace", "ghcr.io/image:tag"),
    ];

    for (name, s) in image_ref_seeds {
        write_seed(&ref_dir, name, s.as_bytes().to_vec());
    }
    println!(
        "wrote {} fuzz_parse_image_ref seeds to {}",
        image_ref_seeds.len(),
        ref_dir.display()
    );
}
