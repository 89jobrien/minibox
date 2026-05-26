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

const TARGET: &str = "aarch64-unknown-linux-musl";

/// Options parsed from CLI args.
pub struct Options {
    /// Skip cross-compilation (assume binaries already built).
    pub skip_build: bool,
    /// Keep the VM running after tests (for debugging).
    pub keep: bool,
    /// Extra arguments forwarded to the test runner.
    pub test_args: Vec<String>,
}

impl Options {
    pub fn from_args(args: &[String]) -> Self {
        Self {
            skip_build: args.iter().any(|a| a == "--skip-build"),
            keep: args.iter().any(|a| a == "--keep"),
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
    /// smolvm machine run (unprivileged — no mount/cgroup)
    Smolvm(PathBuf),
}

impl VmBackend {
    /// Whether this backend supports privileged operations (cgroups, overlayfs).
    fn is_privileged(&self) -> bool {
        matches!(self, VmBackend::Minibox(_))
    }
}

pub fn run(workspace_root: &Path, opts: &Options) -> Result<()> {
    let backend = detect_backend()?;

    let target_dir = std::env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| workspace_root.join("target"));

    // 1. Cross-compile
    if opts.skip_build {
        println!("[1/3] skipping build (--skip-build)");
    } else {
        println!("[1/3] cross-compiling for {TARGET} ...");
        cross_compile(workspace_root)?;
    }

    // 2. Locate test binaries
    let deps_dir = target_dir.join(TARGET).join("debug").join("deps");
    if !deps_dir.exists() {
        bail!(
            "deps dir not found: {}\nRun without --skip-build first.",
            deps_dir.display()
        );
    }

    // 3. Build the test runner script
    let script = build_test_script(&deps_dir, &opts.test_args, backend.is_privileged())?;
    let script_path = target_dir.join("test-in-vm-runner.sh");
    std::fs::write(&script_path, &script).context("writing test runner script")?;

    // 4. Boot VM and run tests
    let mount_spec = format!("{}:/mnt/tests:ro", target_dir.display());
    let mut cmd = match &backend {
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
            println!("[2/3] booting smolvm VM (unprivileged) ...");
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

/// Prefer minibox (privileged, full cgroup/overlay support) over smolvm (unprivileged).
///
/// TODO(#442): check daemon policy (MINIBOX_ALLOW_BIND_MOUNTS / MINIBOX_ALLOW_PRIVILEGED)
///       before selecting minibox backend, and print actionable error if denied.
fn detect_backend() -> Result<VmBackend> {
    if let Some(path) = which_bin("minibox") {
        return Ok(VmBackend::Minibox(path));
    }
    if let Some(path) = which_bin("smolvm") {
        return Ok(VmBackend::Smolvm(path));
    }
    bail!("neither minibox nor smolvm found on PATH. Install one first.");
}

fn cross_compile(workspace_root: &Path) -> Result<()> {
    // TODO(#443): support cross-rs as an alternative to raw musl-gcc (like youki does)
    //       for zero-setup cross-compilation.
    let cc = "aarch64-linux-musl-gcc";
    let cc_env = format!("CC_{}", TARGET.replace('-', "_"));
    let linker_env = format!(
        "CARGO_TARGET_{}_LINKER",
        TARGET.to_uppercase().replace('-', "_")
    );

    // Cross-compile miniboxd binary (needed by e2e/system tests)
    println!("  compiling miniboxd binary ...");
    let status = Command::new("cargo")
        .args(["build", "-p", "miniboxd", "--target", TARGET])
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
        .args(["test", "--no-run", "-p", "miniboxd", "--target", TARGET])
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
fn build_test_script(deps_dir: &Path, extra_args: &[String], privileged: bool) -> Result<String> {
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
    let bin_dir = format!("/mnt/tests/{TARGET}/debug");
    script.push_str(&format!("export MINIBOX_TEST_BIN_DIR={bin_dir}\n\n"));

    script.push_str("PASS=0\nFAIL=0\n\n");

    for (suite, binary) in &found {
        let bin_path = format!("/mnt/tests/{TARGET}/debug/deps/{binary}");
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
        let result = build_test_script(Path::new("/nonexistent"), &[], true);
        assert!(result.is_err());
    }

    #[test]
    fn build_test_script_fails_on_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let result = build_test_script(tmp.path(), &[], true);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("no test binaries"),);
    }
}
