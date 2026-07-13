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
//! SmolVM test helpers for e2e tests.
//!
//!
//! Two execution modes:
//!
//! 1. **Persistent machines** (`SmolVmFixture`) — `machine create/start/exec/stop/delete`.
//!    Good for kernel-feature tests (cgroups, overlay FS) that don't need volumes
//!    or networking. Faster for multiple execs against the same VM.
//!
//! 2. **Ephemeral runs** (`smolvm_run`) — `machine run`. Each call boots a fresh VM
//!    with full volume, env, and network support. Used for daemon and CLI tests
//!    that need workspace access.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use super::{CmdOutput, workspace_root};

// ---------------------------------------------------------------------------
// Ephemeral VM runs (machine run)
// ---------------------------------------------------------------------------

/// Run a command in an ephemeral smolvm VM and return (success, stdout, stderr).
///
/// Uses `smolvm machine run` which handles image pull, volumes, networking,
/// and cleanup automatically. Each call boots a fresh VM.
pub fn smolvm_run(image: &str, args: &[&str]) -> CmdOutput {
    smolvm_run_with_opts(image, args, &[], &[], true, 60)
}

/// Run a command in an ephemeral smolvm VM with full options.
pub fn smolvm_run_with_opts(
    image: &str,
    args: &[&str],
    volumes: &[(&str, &str)],
    env: &[(&str, &str)],
    net: bool,
    timeout_secs: u32,
) -> CmdOutput {
    let mut cmd = Command::new("smolvm");
    cmd.args(["machine", "run", "--image", image]);

    if net {
        cmd.arg("--net");
    }

    cmd.args(["--timeout", &format!("{timeout_secs}sec")]);

    for (host, guest) in volumes {
        cmd.args(["-v", &format!("{host}:{guest}")]);
    }
    for (key, val) in env {
        cmd.args(["-e", &format!("{key}={val}")]);
    }

    cmd.arg("--");
    cmd.args(args);

    let output = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap_or_else(|e| panic!("smolvm machine run failed: {e}"));

    CmdOutput {
        success: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    }
}

/// Run a command in an ephemeral VM with the workspace mounted at /workspace.
pub fn smolvm_run_workspace(image: &str, args: &[&str]) -> CmdOutput {
    let ws = workspace_root();
    let ws_str = ws.to_string_lossy().to_string();
    smolvm_run_with_opts(image, args, &[(&ws_str, "/workspace")], &[], true, 120)
}

// ---------------------------------------------------------------------------
// Persistent machine fixture (create/start/exec/stop/delete)
// ---------------------------------------------------------------------------

/// RAII wrapper around a persistent smolvm machine.
///
/// Best for kernel-feature tests that run multiple execs without needing
/// volumes or outbound networking.
pub struct SmolVmFixture {
    /// Unique machine name for this test.
    pub machine_name: String,
}

impl SmolVmFixture {
    /// Create and start a VM from a Smolfile.
    pub fn start(name_prefix: &str, smolfile: &Path) -> Self {
        let suffix = &uuid::Uuid::new_v4().to_string()[..8];
        let machine_name = format!("{name_prefix}-{suffix}");

        let ws = workspace_root();
        let smolfile_abs = if smolfile.is_absolute() {
            smolfile.to_path_buf()
        } else {
            ws.join(smolfile)
        };

        // Create the machine
        let create_output = Command::new("smolvm")
            .args([
                "machine",
                "create",
                &machine_name,
                "--smolfile",
                &smolfile_abs.to_string_lossy(),
            ])
            .current_dir(&ws)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .unwrap_or_else(|e| panic!("smolvm not found or failed to run: {e}"));

        assert!(
            create_output.status.success(),
            "smolvm machine create failed: {}",
            String::from_utf8_lossy(&create_output.stderr)
        );

        // Start the machine
        let start_output = Command::new("smolvm")
            .args(["machine", "start", &machine_name])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .expect("smolvm machine start");

        assert!(
            start_output.status.success(),
            "smolvm machine start failed: {}",
            String::from_utf8_lossy(&start_output.stderr)
        );

        let fixture = Self { machine_name };
        fixture.wait_ready(Duration::from_secs(30));
        fixture
    }

    /// Execute a command inside the VM and return the raw output.
    pub fn exec_raw(&self, args: &[&str]) -> Output {
        let mut cmd = Command::new("smolvm");
        cmd.args(["machine", "exec", &self.machine_name, "--"]);
        cmd.args(args);
        cmd.stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .unwrap_or_else(|e| panic!("smolvm exec failed: {e}"))
    }

    /// Execute a command inside the VM and return (success, stdout, stderr).
    pub fn exec(&self, args: &[&str]) -> CmdOutput {
        let output = self.exec_raw(args);
        CmdOutput {
            success: output.status.success(),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        }
    }

    /// Wait until the VM responds to a basic exec command.
    fn wait_ready(&self, timeout: Duration) {
        let start = Instant::now();
        loop {
            let output = Command::new("smolvm")
                .args(["machine", "exec", &self.machine_name, "--", "true"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();

            if let Ok(status) = output {
                if status.success() {
                    return;
                }
            }

            if start.elapsed() > timeout {
                panic!(
                    "smolvm machine '{}' did not become ready within {}s",
                    self.machine_name,
                    timeout.as_secs()
                );
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    }
}

impl Drop for SmolVmFixture {
    fn drop(&mut self) {
        let _ = Command::new("smolvm")
            .args(["machine", "stop", &self.machine_name])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();

        let _ = Command::new("smolvm")
            .args(["machine", "delete", &self.machine_name, "--force"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

/// Returns true if smolvm is installed and functional.
pub fn smolvm_available() -> bool {
    Command::new("smolvm")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}
