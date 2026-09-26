#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::doc_markdown
)]

//! End-to-end checks of the packaged `minibox-cni` binaries.
//!
//! These run the real `[[bin]]` targets, so they verify the deliverable
//! rather than the library behind it. None of them need root, a network
//! namespace, or a real CNI plugin: `VERSION`, spec-version validation, and
//! the rollout preflight are all reachable with a synthetic environment
//! and fixture scripts in a temp directory.

use minibox_cni::rollout::{
    BRIDGE_PLUGIN_NAME, CONFLIST_FILE_NAME, ENV_MINIBOX_CNI_CONFIG_DIR, ENV_MINIBOX_CNI_PATH,
};
use minibox_cni::version::{
    CNI_COMMAND_CHECK, CNI_COMMAND_DEL, CNI_COMMAND_VERSION, CNI_SPEC_VERSION, ENV_CNI_ARGS,
    ENV_CNI_COMMAND, ENV_CNI_CONTAINERID, ENV_CNI_IFNAME, ENV_CNI_NETNS, ENV_CNI_PATH,
    ERR_INCOMPATIBLE_CNI_VERSION, ERR_INTERNAL, ERR_INVALID_ENVIRONMENT_VARIABLES,
    ERR_INVALID_NETWORK_CONFIG,
};
use std::path::Path;
use std::process::{Command, Output, Stdio};

const BRIDGE_BIN: &str = env!("CARGO_BIN_EXE_minibox-cni-bridge");
const VERIFY_BIN: &str = env!("CARGO_BIN_EXE_minibox-cni-verify");

const VALID_CONFIG: &str =
    r#"{"cniVersion":"1.0.0","name":"minibox0","type":"minibox-bridge","bridge":"minibox0"}"#;

/// Run the plugin binary with a clean `CNI_*` environment and `config` on
/// stdin. Passing `None` closes stdin, which is what a runtime does for
/// `CNI_COMMAND=VERSION`.
fn run_plugin(command: &str, config: Option<&str>) -> Output {
    let mut child = Command::new(BRIDGE_BIN)
        .env(ENV_CNI_COMMAND, command)
        .env(ENV_CNI_CONTAINERID, "container-1")
        .env(ENV_CNI_NETNS, "/proc/1/ns/net")
        .env(ENV_CNI_IFNAME, "eth0")
        .env(ENV_CNI_PATH, "/opt/cni/bin")
        .env(ENV_CNI_ARGS, "K8S_POD_NAMESPACE=default")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn plugin");
    {
        use std::io::Write as _;
        let mut stdin = child.stdin.take().expect("piped stdin");
        if let Some(config) = config {
            stdin
                .write_all(config.as_bytes())
                .expect("write plugin config");
        }
    }
    child.wait_with_output().expect("plugin output")
}

fn stdout_json(output: &Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|err| {
        panic!(
            "stdout was not a single JSON value ({err}); stdout={:?} stderr={:?}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn error_code(output: &Output) -> u64 {
    stdout_json(output)["code"]
        .as_u64()
        .unwrap_or_else(|| panic!("error reply had no numeric code: {output:?}"))
}

/// Assert the plugin exited non-zero with the given CNI spec error code.
fn assert_error_code(output: &Output, expected: u32) {
    assert_failure(output);
    assert_eq!(error_code(output), u64::from(expected), "{output:?}");
}

fn assert_failure(output: &Output) {
    assert!(
        !output.status.success(),
        "expected a non-zero exit, got {:?}",
        output.status
    );
}

fn write_executable(dir: &Path, name: &str, body: &str) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt as _;
    let path = dir.join(name);
    std::fs::write(&path, body).expect("write fixture");
    let mut perms = std::fs::metadata(&path).expect("stat").permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&path, perms).expect("chmod");
    path
}

fn write_conflist(dir: &Path, version: &str, plugin: &str) {
    std::fs::write(
        dir.join(CONFLIST_FILE_NAME),
        format!(
            r#"{{"cniVersion":"{version}","name":"minibox0","plugins":[{{"type":"{plugin}"}}]}}"#
        ),
    )
    .expect("write conflist");
}

// ── VERSION ────────────────────────────────────────────────────────────────

#[test]
fn version_command_reports_the_single_source_of_truth() {
    let output = run_plugin(CNI_COMMAND_VERSION, None);

    assert!(output.status.success(), "{output:?}");
    let reply = stdout_json(&output);
    assert_eq!(reply["cniVersion"], CNI_SPEC_VERSION);
    let supported = reply["supportedVersions"]
        .as_array()
        .expect("supportedVersions array");
    assert!(
        supported.iter().any(|v| v == CNI_SPEC_VERSION),
        "supportedVersions {supported:?} must advertise {CNI_SPEC_VERSION}"
    );
}

#[test]
fn version_command_needs_neither_config_nor_namespace() {
    // A bare environment: no CNI_CONTAINERID, CNI_NETNS, or CNI_IFNAME.
    let output = Command::new(BRIDGE_BIN)
        .env(ENV_CNI_COMMAND, CNI_COMMAND_VERSION)
        .stdin(Stdio::null())
        .output()
        .expect("run plugin");

    assert!(output.status.success(), "{output:?}");
    assert_eq!(stdout_json(&output)["cniVersion"], CNI_SPEC_VERSION);
}

// ── spec-version validation ────────────────────────────────────────────────

#[test]
fn add_rejects_an_unsupported_spec_version() {
    let config = r#"{"cniVersion":"0.3.1","name":"minibox0","type":"minibox-bridge"}"#;
    let output = run_plugin("ADD", Some(config));

    assert_error_code(&output, ERR_INCOMPATIBLE_CNI_VERSION);
    let msg = stdout_json(&output)["msg"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert!(msg.contains("0.3.1"), "{msg}");
}

#[test]
fn check_rejects_an_unsupported_spec_version() {
    let config = r#"{"cniVersion":"0.4.0","name":"minibox0","type":"minibox-bridge"}"#;
    let output = run_plugin(CNI_COMMAND_CHECK, Some(config));

    assert_error_code(&output, ERR_INCOMPATIBLE_CNI_VERSION);
}

#[test]
fn del_rejects_an_unsupported_spec_version() {
    let config = r#"{"cniVersion":"9.9.9","name":"minibox0","type":"minibox-bridge"}"#;
    let output = run_plugin(CNI_COMMAND_DEL, Some(config));

    assert_error_code(&output, ERR_INCOMPATIBLE_CNI_VERSION);
}

#[test]
fn add_rejects_a_config_without_a_spec_version() {
    let output = run_plugin(
        "ADD",
        Some(r#"{"name":"minibox0","type":"minibox-bridge"}"#),
    );

    assert_error_code(&output, ERR_INVALID_NETWORK_CONFIG);
}

// ── command and environment validation ─────────────────────────────────────

#[test]
fn an_unknown_command_is_an_environment_error() {
    let output = run_plugin("GC", Some(VALID_CONFIG));

    assert_error_code(&output, ERR_INVALID_ENVIRONMENT_VARIABLES);
}

#[test]
fn a_missing_cni_command_is_an_environment_error() {
    let output = Command::new(BRIDGE_BIN)
        .stdin(Stdio::null())
        .output()
        .expect("run plugin");

    assert_failure(&output);
    let reply = stdout_json(&output);
    assert_eq!(reply["code"], ERR_INVALID_ENVIRONMENT_VARIABLES);
    assert!(
        reply["msg"]
            .as_str()
            .unwrap_or_default()
            .contains("CNI_COMMAND")
    );
}

// ── DEL ────────────────────────────────────────────────────────────────────

#[test]
fn del_succeeds_and_prints_null() {
    let output = run_plugin(CNI_COMMAND_DEL, Some(VALID_CONFIG));

    assert!(output.status.success(), "{output:?}");
    assert!(
        stdout_json(&output).is_null(),
        "DEL must print null, got {:?}",
        String::from_utf8_lossy(&output.stdout)
    );
}

// ── ADD / CHECK: the deferred netlink adapter ──────────────────────────────

#[test]
fn add_refuses_loudly_instead_of_faking_a_result() {
    let output = run_plugin("ADD", Some(VALID_CONFIG));

    assert_failure(&output);
    let reply = stdout_json(&output);
    assert_eq!(reply["code"], ERR_INTERNAL);
    let details = reply["details"].as_str().unwrap_or_default();
    assert!(details.contains("netlink"), "{details}");
}

// ── the rollout verifier ───────────────────────────────────────────────────

fn version_reply_script(version: &str) -> String {
    format!(
        "#!/bin/sh\nif [ \"$CNI_COMMAND\" = \"{CNI_COMMAND_VERSION}\" ]; then\n  \
         echo '{{\"cniVersion\":\"{version}\",\"supportedVersions\":[\"{version}\"]}}'\nfi\nexit 0\n"
    )
}

#[test]
fn verify_reports_a_converged_rollout() {
    let bin_dir = tempfile::tempdir().expect("bin dir");
    let config_dir = tempfile::tempdir().expect("config dir");
    write_executable(
        bin_dir.path(),
        BRIDGE_PLUGIN_NAME,
        &version_reply_script(CNI_SPEC_VERSION),
    );
    write_conflist(config_dir.path(), CNI_SPEC_VERSION, BRIDGE_PLUGIN_NAME);

    let output = Command::new(VERIFY_BIN)
        .env(ENV_MINIBOX_CNI_PATH, bin_dir.path())
        .env(ENV_MINIBOX_CNI_CONFIG_DIR, config_dir.path())
        .stdin(Stdio::null())
        .output()
        .expect("run verifier");

    assert!(output.status.success(), "{output:?}");
    let report = stdout_json(&output);
    assert_eq!(report["network_name"], "minibox0");
    assert_eq!(report["conflist_spec_version"], CNI_SPEC_VERSION);
    assert_eq!(report["plugins"][0]["plugin_type"], BRIDGE_PLUGIN_NAME);
    assert_eq!(
        report["plugins"][0]["advertised_spec_version"],
        CNI_SPEC_VERSION
    );
}

#[test]
fn verify_rejects_a_plugin_advertising_a_different_spec_version() {
    let bin_dir = tempfile::tempdir().expect("bin dir");
    let config_dir = tempfile::tempdir().expect("config dir");
    write_executable(
        bin_dir.path(),
        BRIDGE_PLUGIN_NAME,
        &version_reply_script("0.3.1"),
    );
    write_conflist(config_dir.path(), CNI_SPEC_VERSION, BRIDGE_PLUGIN_NAME);

    let output = Command::new(VERIFY_BIN)
        .env(ENV_MINIBOX_CNI_PATH, bin_dir.path())
        .env(ENV_MINIBOX_CNI_CONFIG_DIR, config_dir.path())
        .stdin(Stdio::null())
        .output()
        .expect("run verifier");

    assert_error_code(&output, ERR_INCOMPATIBLE_CNI_VERSION);
}

#[test]
fn verify_reports_a_missing_plugin_binary_with_the_install_hint() {
    let bin_dir = tempfile::tempdir().expect("bin dir");
    let config_dir = tempfile::tempdir().expect("config dir");
    write_conflist(config_dir.path(), CNI_SPEC_VERSION, BRIDGE_PLUGIN_NAME);

    let output = Command::new(VERIFY_BIN)
        .env(ENV_MINIBOX_CNI_PATH, bin_dir.path())
        .env(ENV_MINIBOX_CNI_CONFIG_DIR, config_dir.path())
        .stdin(Stdio::null())
        .output()
        .expect("run verifier");

    assert_failure(&output);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(BRIDGE_PLUGIN_NAME), "{stderr}");
    assert!(stderr.contains("miniboxd"), "{stderr}");
}
