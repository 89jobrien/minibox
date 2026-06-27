//! test-in-vm — run Linux-only tests inside an ephemeral Alpine VM.
//!
//! Cross-compiles test binaries for aarch64-unknown-linux-musl, boots an Alpine
//! VM via minibox (preferred, privileged) or smolvm (fallback, unprivileged),
//! mounts the target dir, and executes the test suites.
//!
//! ## Prerequisites
//!
//! - `minibox` or `smolvm` on PATH
//! - `aarch64-linux-musl-gcc` for cross-compilation
//! - If using minibox: daemon must run with `MINIBOX_ALLOW_BIND_MOUNTS=1`
//!   and `MINIBOX_ALLOW_PRIVILEGED=1`

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::xconfig::XConfig;

pub struct Options {
    /// Skip cross-compilation (assume binaries already built).
    pub skip_build: bool,
    /// Keep the VM running after tests (for debugging).
    pub keep: bool,
    /// Override smolfile path (smolvm backend only).
    pub smolfile: Option<String>,
    /// Extra arguments forwarded to the test runner.
    pub test_args: Vec<String>,
}

impl Options {
    pub fn from_args(args: &[String]) -> Self {
        let smolfile = args
            .windows(2)
            .find(|w| w[0] == "--smolfile")
            .map(|w| w[1].clone());
        Self {
            skip_build: args.iter().any(|a| a == "--skip-build"),
            keep: args.iter().any(|a| a == "--keep"),
            smolfile,
            test_args: args
                .iter()
                .skip_while(|a| *a != "--")
                .skip(1)
                .cloned()
                .collect(),
        }
    }
}

/// Which VM backend to use.
enum VmBackend {
    /// minibox run --privileged (full cgroup/overlay support)
    Minibox(PathBuf),
    /// smolvm persistent machine (pre-provisioned with Rust toolchain)
    SmolvmPersistent(PathBuf),
    /// smolvm machine run (ephemeral — no Rust toolchain, cross-compiled binaries only)
    Smolvm(PathBuf),
}

impl VmBackend {
    /// Whether this backend supports privileged operations (cgroups, overlayfs).
    const fn is_privileged(&self) -> bool {
        matches!(self, Self::Minibox(_))
    }

    /// Whether this backend has Rust toolchain pre-installed.
    const fn has_rust(&self) -> bool {
        matches!(self, Self::SmolvmPersistent(_))
    }
}

pub fn run(workspace_root: &Path, opts: &Options) -> Result<()> {
    let cfg = XConfig::load(workspace_root)?;
    let target = &cfg.cross.target;
    let vm_name = &cfg.vm.name;
    let ci_gate_smolfile = &cfg.vm.ci_gate_smolfile;

    let backend = detect_backend(vm_name)?;

    // Persistent VM path: mount workspace, run cargo test directly (no cross-compile)
    if backend.has_rust() {
        return run_persistent(&backend, workspace_root, opts, vm_name);
    }

    // Ephemeral path: cross-compile, mount binaries, run pre-compiled tests
    run_ephemeral(&backend, workspace_root, opts, target, ci_gate_smolfile)
}

/// Run tests via a pre-provisioned persistent smolvm machine.
///
/// The machine already has Rust toolchain installed. We mount the workspace
/// and run cargo commands directly — no cross-compilation needed.
fn run_persistent(
    backend: &VmBackend,
    _workspace_root: &Path,
    opts: &Options,
    vm_name: &str,
) -> Result<()> {
    let VmBackend::SmolvmPersistent(bin) = backend else {
        bail!("run_persistent called with non-persistent backend");
    };

    // Ensure machine is running
    println!("[1/2] ensuring '{vm_name}' is running ...");
    let status = Command::new(bin)
        .args(["machine", "start", "--name", vm_name])
        .status()
        .context("starting persistent machine")?;
    if !status.success() {
        bail!("failed to start '{vm_name}' — run `cargo xtask setup-test-vm` first");
    }

    // Build cargo test command
    let extra = if opts.test_args.is_empty() {
        String::new()
    } else {
        format!(" -- {}", opts.test_args.join(" "))
    };

    // SmolvmPersistent is unprivileged — integration_tests require root, cgroups v2,
    // and overlayfs.  Only run --lib (unit) tests unless the backend is privileged.
    let test_cmd = if backend.is_privileged() {
        format!(
            "cargo test -p miniboxd --lib --test integration_tests -- --include-ignored{extra} 2>&1"
        )
    } else {
        println!("  (skipping integration_tests — unprivileged backend, no cgroup/overlay)");
        format!("cargo test -p miniboxd --lib{extra} 2>&1")
    };

    let script = format!(
        r#"set -e
. "$HOME/.cargo/env"
export CARGO_BUILD_JOBS=1
export CARGO_TARGET_DIR=/tmp/target
cd /mnt/workspace
echo "--- cargo check -p miniboxd ---"
cargo check -p miniboxd 2>&1
echo ""
echo "--- {test_cmd_label} ---"
{test_cmd}
echo ""
echo "test-in-vm: all tests passed"
"#,
        test_cmd_label = if backend.is_privileged() {
            "cargo test -p miniboxd --lib --test integration_tests"
        } else {
            "cargo test -p miniboxd --lib"
        },
    );

    println!("[2/2] running tests in '{vm_name}' ...");
    // Volume is already mounted via create-time -v flag.
    // Use -t for TTY allocation so cargo progress output streams live.
    let status = Command::new(bin)
        .args(["machine", "exec", "-t", "--name", vm_name])
        .args(["--", "/bin/sh", "-c", &script])
        .status()
        .context("exec in persistent machine")?;

    if !status.success() {
        bail!(
            "test-in-vm: tests failed (exit {})",
            status.code().unwrap_or(-1)
        );
    }
    Ok(())
}

/// Run tests via ephemeral VM with cross-compiled binaries.
fn run_ephemeral(
    backend: &VmBackend,
    workspace_root: &Path,
    opts: &Options,
    target: &str,
    ci_gate_smolfile: &str,
) -> Result<()> {
    let target_dir = std::env::var("CARGO_TARGET_DIR")
        .map_or_else(|_| workspace_root.join("target"), PathBuf::from);

    // 1. Cross-compile
    if opts.skip_build {
        println!("[1/3] skipping build (--skip-build)");
    } else {
        println!("[1/3] cross-compiling for {target} ...");
        cross_compile(workspace_root, target)?;
    }

    // 2. Locate test binaries
    let deps_dir = target_dir.join(target).join("debug").join("deps");
    if !deps_dir.exists() {
        bail!(
            "deps dir not found: {}\nRun without --skip-build first.",
            deps_dir.display()
        );
    }

    // 3. Build the test runner script
    let script = build_test_script(&deps_dir, &opts.test_args, backend.is_privileged(), target)?;
    let script_path = target_dir.join("test-in-vm-runner.sh");
    std::fs::write(&script_path, &script).context("writing test runner script")?;

    // 4. Boot VM and run tests
    let mount_spec = format!("{}:/mnt/tests:ro", target_dir.display());
    let mut cmd = match backend {
        VmBackend::Minibox(bin) => {
            println!("[2/3] booting minibox VM (privileged) ...");
            let mut c = Command::new(bin);
            // TODO(#441): network bridge required for apk add; switch to --network none
            //       once we pre-bake test deps into a cached image (Phase C).
            c.args(["run", "--privileged", "--network", "bridge", "alpine"]);
            c.args(["-v", &mount_spec]);
            c.args(["--", "/bin/sh", "-c"]);
            c.arg(&script);
            c
        }
        VmBackend::Smolvm(bin) => {
            let smolfile = opts.smolfile.as_deref().unwrap_or(ci_gate_smolfile);
            let smolfile_path = workspace_root.join(smolfile);
            if smolfile_path.exists() {
                println!("[2/3] booting smolvm VM (unprivileged) via {smolfile} ...");
                let mut c = Command::new(bin);
                c.args([
                    "machine",
                    "run",
                    "--smolfile",
                    &smolfile_path.to_string_lossy(),
                ]);
                c.args(["-v", &mount_spec]);
                if opts.keep {
                    c.arg("--detach");
                }
                c.args(["--", "/bin/sh", "-c"]);
                c.arg(&script);
                c
            } else {
                println!(
                    "[2/3] booting smolvm VM (unprivileged, inline — {smolfile} not found) ..."
                );
                let mut c = Command::new(bin);
                c.args(["machine", "run", "--net", "--image", "alpine"]);
                c.args(["-v", &mount_spec]);
                c.args(["--mem", "4096"]);
                if opts.keep {
                    c.arg("--detach");
                }
                c.args(["--", "/bin/sh", "-c"]);
                c.arg(&script);
                c
            }
        }
        VmBackend::SmolvmPersistent(_) => unreachable!(),
    };

    println!("[3/3] running tests ...");
    let status = cmd.status().context("spawning VM")?;

    if !status.success() {
        bail!(
            "test-in-vm: tests failed (exit {})",
            status.code().unwrap_or(-1)
        );
    }

    println!("test-in-vm: all tests passed");
    Ok(())
}

fn which_bin(name: &str) -> Option<PathBuf> {
    Command::new("which")
        .arg(name)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| PathBuf::from(String::from_utf8_lossy(&o.stdout).trim().to_string()))
}

/// Returns `true` when `var` is set to `1`, `true`, or `yes` (case-insensitive).
fn env_flag(var: &str) -> bool {
    std::env::var(var)
        .ok()
        .is_some_and(|v| matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
}

/// Returns `true` when both `MINIBOX_ALLOW_BIND_MOUNTS` and
/// `MINIBOX_ALLOW_PRIVILEGED` are enabled.  Prints a warning naming the
/// missing flags when either is unset/false.
fn minibox_policy_allows() -> bool {
    let bind = env_flag("MINIBOX_ALLOW_BIND_MOUNTS");
    let priv_ = env_flag("MINIBOX_ALLOW_PRIVILEGED");
    if bind && priv_ {
        return true;
    }
    let mut missing = Vec::new();
    if !bind {
        missing.push("MINIBOX_ALLOW_BIND_MOUNTS");
    }
    if !priv_ {
        missing.push("MINIBOX_ALLOW_PRIVILEGED");
    }
    eprintln!(
        "warning: minibox on PATH but daemon policy denies required capabilities.\n\
         Set {} to 1/true/yes and restart miniboxd, or fall back to smolvm.",
        missing.join(" and "),
    );
    false
}

/// Prefer persistent smolvm > minibox (privileged) > ephemeral smolvm.
///
/// Persistent smolvm is preferred because it has Rust toolchain pre-installed
/// and avoids cross-compilation entirely.
fn detect_backend(vm_name: &str) -> Result<VmBackend> {
    // Persistent smolvm machine first — has Rust, no cross-compile needed
    if let Some(path) = which_bin("smolvm")
        && persistent_machine_exists(&path, vm_name)?
    {
        println!("detected persistent '{vm_name}' — using native cargo test (no cross-compile)");
        return Ok(VmBackend::SmolvmPersistent(path));
    }
    // minibox with privileged mode (requires running daemon with policy flags)
    if let Some(path) = which_bin("minibox")
        && minibox_policy_allows()
    {
        return Ok(VmBackend::Minibox(path));
    }
    // Ephemeral smolvm fallback
    if let Some(path) = which_bin("smolvm") {
        return Ok(VmBackend::Smolvm(path));
    }
    bail!("neither minibox nor smolvm found on PATH. Install one first.");
}

fn persistent_machine_exists(smolvm: &Path, vm_name: &str) -> Result<bool> {
    let output = Command::new(smolvm)
        .args(["machine", "list"])
        .output()
        .context("smolvm machine list")?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout.lines().any(|line| line.starts_with(vm_name)))
}

fn cross_compile(workspace_root: &Path, target: &str) -> Result<()> {
    // TODO(#443): support cross-rs as an alternative to raw musl-gcc (like youki does)
    //       for zero-setup cross-compilation.
    let cc = "aarch64-linux-musl-gcc";
    let cc_env = format!("CC_{}", target.replace('-', "_"));
    let linker_env = format!(
        "CARGO_TARGET_{}_LINKER",
        target.to_uppercase().replace('-', "_")
    );

    // Cross-compile miniboxd binary (needed by e2e/system tests)
    println!("  compiling miniboxd binary ...");
    let status = Command::new("cargo")
        .args(["build", "-p", "miniboxd", "--target", target])
        .env(&cc_env, cc)
        .env(&linker_env, cc)
        .current_dir(workspace_root)
        .status()
        .context("spawning cargo build miniboxd")?;
    if !status.success() {
        bail!("cross-compile failed for miniboxd binary");
    }

    // Cross-compile miniboxd test binaries — these are the Linux-only
    // suites (cgroup, integration, sandbox, system, e2e).
    println!("  compiling miniboxd tests ...");
    let status = Command::new("cargo")
        .args(["test", "--no-run", "-p", "miniboxd", "--target", target])
        .env(&cc_env, cc)
        .env(&linker_env, cc)
        .current_dir(workspace_root)
        .status()
        .context("spawning cargo test --no-run")?;
    if !status.success() {
        bail!("cross-compile failed for miniboxd tests");
    }
    Ok(())
}

/// Find test binaries in the deps dir and build a shell script to run them.
fn build_test_script(
    deps_dir: &Path,
    extra_args: &[String],
    privileged: bool,
    target: &str,
) -> Result<String> {
    // TODO(#444): add sandbox_tests, system_tests, cli_e2e_tests once minibox
    //       backend is validated end-to-end with privileged mode.
    let mut suites = vec!["integration_tests"];
    if privileged {
        suites.push("cgroup_tests");
    } else {
        println!("  (skipping cgroup_tests — unprivileged backend)");
    }

    let mut found = Vec::new();
    if deps_dir.exists() {
        for entry in std::fs::read_dir(deps_dir).context("reading deps dir")? {
            let entry = entry.context("dir entry")?;
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            // Test binaries: <suite_name>-<hash> with no extension
            for suite in &suites {
                let prefix = suite.replace('-', "_");
                if name_str.starts_with(&prefix) && !name_str.contains('.') {
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        if entry
                            .metadata()
                            .is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
                        {
                            found.push((suite.to_string(), name_str.to_string()));
                        }
                    }
                    #[cfg(not(unix))]
                    {
                        found.push((suite.to_string(), name_str.to_string()));
                    }
                    break;
                }
            }
        }
    }

    if found.is_empty() {
        bail!(
            "no test binaries found in {}. Run without --skip-build.",
            deps_dir.display()
        );
    }

    let extra = if extra_args.is_empty() {
        String::new()
    } else {
        format!(" {}", extra_args.join(" "))
    };

    // Suites that require --ignored (tests gated behind #[ignore] for root/Linux)
    let ignored_suites = ["integration_tests"];

    let mut script = String::from("#!/bin/sh\n\n");
    // TODO(#446): pre-bake test deps into a cached image (Phase C) to avoid
    //       network access at boot and speed up repeated runs.
    script.push_str("# Install test dependencies (best-effort, no set -e yet)\n");
    script.push_str("apk add --no-cache coreutils util-linux 2>/dev/null || true\n\n");

    // Make cgroup writable for cgroup tests (best-effort — may fail in
    // unprivileged VMs where /sys/fs/cgroup is read-only).
    script.push_str("mount -o remount,rw /sys/fs/cgroup 2>/dev/null || true\n");
    script.push_str("(echo '+cpu +memory +pids +io' > /sys/fs/cgroup/cgroup.subtree_control) 2>/dev/null || true\n\n");

    // From here on, fail on errors.
    script.push_str("set -e\n\n");

    // Set bin dir for e2e tests that need miniboxd/minibox binaries
    let bin_dir = format!("/mnt/tests/{target}/debug");
    script.push_str(&format!("export MINIBOX_TEST_BIN_DIR={bin_dir}\n\n"));

    script.push_str("PASS=0\nFAIL=0\n\n");

    for (suite, binary) in &found {
        let bin_path = format!("/mnt/tests/{target}/debug/deps/{binary}");
        let ignored_flag = if ignored_suites.contains(&suite.as_str()) {
            " --ignored"
        } else {
            ""
        };
        script.push_str(&format!("echo '=== {suite} ==='\n"));
        script.push_str(&format!(
            "if {bin_path} --test-threads=1{ignored_flag}{extra}; then\n  PASS=$((PASS + 1))\nelse\n  FAIL=$((FAIL + 1))\nfi\n\n"
        ));
    }

    script.push_str("echo \"\"\n");
    script.push_str("echo \"=== Results: $PASS passed, $FAIL failed ===\"\n");
    if !privileged {
        script.push_str("echo \"Note: cgroup tests skipped (unprivileged backend)\"\n");
    }
    script.push_str("[ \"$FAIL\" -eq 0 ]\n");

    Ok(script)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_parse_skip_build() {
        let opts = Options::from_args(&["--skip-build".to_string()]);
        assert!(opts.skip_build);
        assert!(!opts.keep);
        assert!(opts.test_args.is_empty());
    }

    #[test]
    fn options_parse_keep() {
        let opts = Options::from_args(&["--keep".to_string()]);
        assert!(!opts.skip_build);
        assert!(opts.keep);
    }

    #[test]
    fn options_parse_test_args() {
        let args: Vec<String> = vec!["--skip-build", "--", "--nocapture", "foo"]
            .into_iter()
            .map(String::from)
            .collect();
        let opts = Options::from_args(&args);
        assert!(opts.skip_build);
        assert_eq!(opts.test_args, vec!["--nocapture", "foo"]);
    }

    #[test]
    fn build_test_script_fails_on_missing_dir() {
        let result = build_test_script(
            Path::new("/nonexistent"),
            &[],
            true,
            "aarch64-unknown-linux-musl",
        );
        assert!(result.is_err());
    }

    #[test]
    fn build_test_script_fails_on_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let result = build_test_script(tmp.path(), &[], true, "aarch64-unknown-linux-musl");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("no test binaries"),);
    }

    #[test]
    fn options_parse_smolfile() {
        let args: Vec<String> = vec!["--smolfile", "custom.smolfile"]
            .into_iter()
            .map(String::from)
            .collect();
        let opts = Options::from_args(&args);
        assert_eq!(opts.smolfile.as_deref(), Some("custom.smolfile"));
    }

    #[test]
    fn options_default_smolfile_is_none() {
        let opts = Options::from_args(&[]);
        assert!(opts.smolfile.is_none());
    }

    #[test]
    fn all_smolfiles_exist_and_have_required_keys() {
        let ws = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("workspace root");
        let smolfiles = [
            "tests/smolfiles/minimal.smolfile",
            "tests/smolfiles/ci-gate.smolfile",
            "tests/smolfiles/ci-cached.smolfile",
            "tests/smolfiles/e2e.smolfile",
            "tests/smolfiles/network.smolfile",
        ];
        for sf in &smolfiles {
            let path = ws.join(sf);
            assert!(path.exists(), "smolfile missing: {sf}");
            let content =
                std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {sf}: {e}"));
            assert!(content.contains("image ="), "{sf} missing 'image' key");
            assert!(content.contains("cpus ="), "{sf} missing 'cpus' key");
            assert!(content.contains("memory ="), "{sf} missing 'memory' key");
        }
    }

    #[test]
    fn ci_gate_smolfile_from_xconfig_matches_real_file() {
        let ws = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("workspace root");
        let cfg = XConfig::load(ws).expect("load xconfig");
        assert!(
            ws.join(&cfg.vm.ci_gate_smolfile).exists(),
            "xconfig vm.ci_gate_smolfile points to missing file: {}",
            cfg.vm.ci_gate_smolfile
        );
    }

    /// Serialize env-mutating tests (Rust 2024: set_var/remove_var are unsafe).
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn env_flag_true_values() {
        let _g = ENV_LOCK.lock().expect("env lock");
        for val in ["1", "true", "yes", "TRUE", "Yes"] {
            // SAFETY: serialized via ENV_LOCK; no other thread reads this var.
            unsafe { std::env::set_var("_TEST_FLAG_442", val) };
            assert!(env_flag("_TEST_FLAG_442"), "expected true for {val:?}");
        }
        // SAFETY: same guard
        unsafe { std::env::remove_var("_TEST_FLAG_442") };
    }

    #[test]
    fn env_flag_false_values() {
        let _g = ENV_LOCK.lock().expect("env lock");
        for val in ["0", "false", "no", ""] {
            // SAFETY: serialized via ENV_LOCK
            unsafe { std::env::set_var("_TEST_FLAG_442", val) };
            assert!(!env_flag("_TEST_FLAG_442"), "expected false for {val:?}");
        }
        // SAFETY: same guard
        unsafe { std::env::remove_var("_TEST_FLAG_442") };
    }

    #[test]
    fn env_flag_unset_is_false() {
        let _g = ENV_LOCK.lock().expect("env lock");
        // SAFETY: serialized via ENV_LOCK
        unsafe { std::env::remove_var("_TEST_FLAG_442_UNSET") };
        assert!(!env_flag("_TEST_FLAG_442_UNSET"));
    }

    #[test]
    fn minibox_policy_allows_both_set() {
        let _g = ENV_LOCK.lock().expect("env lock");
        // SAFETY: serialized via ENV_LOCK
        unsafe {
            std::env::set_var("MINIBOX_ALLOW_BIND_MOUNTS", "1");
            std::env::set_var("MINIBOX_ALLOW_PRIVILEGED", "1");
        }
        assert!(minibox_policy_allows());
        // SAFETY: cleanup
        unsafe {
            std::env::remove_var("MINIBOX_ALLOW_BIND_MOUNTS");
            std::env::remove_var("MINIBOX_ALLOW_PRIVILEGED");
        }
    }

    #[test]
    fn minibox_policy_denies_when_missing() {
        let _g = ENV_LOCK.lock().expect("env lock");
        // SAFETY: serialized via ENV_LOCK
        unsafe {
            std::env::remove_var("MINIBOX_ALLOW_BIND_MOUNTS");
            std::env::remove_var("MINIBOX_ALLOW_PRIVILEGED");
        }
        assert!(!minibox_policy_allows());
    }
}
