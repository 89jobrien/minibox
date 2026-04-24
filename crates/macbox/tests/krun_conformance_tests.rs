//! krun adapter conformance tests.
//!
//! Tests the `KrunRuntime`, `KrunFilesystem`, and `KrunLimiter` adapters by
//! shelling out to `smolvm`. Each test skips gracefully if `smolvm` is not
//! found on PATH.
//!
//! Run with:
//!   cargo test -p macbox --test krun_conformance_tests -- --test-threads=1
//!
//! `--test-threads=1` is required: parallel smolvm invocations collide on the
//! same agent socket path and produce spurious "connect to agent" failures.
//!
//! These tests require `smolvm` on PATH and network access for image pulls.
//! They are intentionally slow (microVM boot) and should not run in unit-test
//! gates — add to an explicit CI job or run manually.

#[cfg(target_os = "macos")]
mod suite {
    use macbox::krun::process::SmolvmProcess;
    use std::time::Duration;

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    /// Returns `false` if `smolvm` is not found on PATH, so tests can skip.
    fn smolvm_available() -> bool {
        which::which("smolvm").is_ok()
    }

    /// Returns `true` if krun integration tests are explicitly opted in via env var.
    fn krun_tests_enabled() -> bool {
        std::env::var("MINIBOX_KRUN_TESTS")
            .map(|v| v == "1")
            .unwrap_or(false)
    }

    macro_rules! skip_if_no_smolvm {
        () => {
            if !krun_tests_enabled() {
                eprintln!("SKIP: set MINIBOX_KRUN_TESTS=1 to run krun integration tests");
                return;
            }
            if !smolvm_available() {
                eprintln!("SKIP: smolvm not found on PATH");
                return;
            }
        };
    }

    // -----------------------------------------------------------------------
    // Tests
    // -----------------------------------------------------------------------

    /// `SmolvmProcess::spawn` with a nonexistent binary path returns `Err`.
    #[tokio::test]
    async fn krun_adapter_missing_binary_returns_error() {
        let result = SmolvmProcess::spawn_with_bin(
            std::path::Path::new("/nonexistent/smolvm"),
            "alpine",
            &["/bin/true".to_string()],
            &[],
        )
        .await;
        assert!(result.is_err(), "expected Err for missing binary");
    }

    /// A process that exits 0 produces a `ContainerStopped` with exit_code 0.
    #[tokio::test]
    async fn krun_process_exits_zero_for_true_command() {
        skip_if_no_smolvm!();
        let mut proc = SmolvmProcess::spawn("alpine", &["/bin/true".to_string()], &[])
            .await
            .expect("spawn failed");
        let exit = tokio::time::timeout(Duration::from_secs(30), proc.wait())
            .await
            .expect("timed out")
            .expect("wait failed");
        assert_eq!(exit, 0, "expected exit code 0");
    }

    /// Stdout from the process is readable via `proc.stdout()`.
    #[tokio::test]
    async fn krun_stdout_is_captured() {
        skip_if_no_smolvm!();
        let mut proc = SmolvmProcess::spawn(
            "alpine",
            &[
                "/bin/sh".to_string(),
                "-c".to_string(),
                "echo krun-hello".to_string(),
            ],
            &[],
        )
        .await
        .expect("spawn failed");

        let output = tokio::time::timeout(Duration::from_secs(30), proc.collect_stdout())
            .await
            .expect("timed out")
            .expect("collect_stdout failed");

        assert!(
            output.contains("krun-hello"),
            "stdout must contain 'krun-hello'; got: {output:?}"
        );
    }

    /// A nonzero exit code is propagated correctly.
    #[tokio::test]
    async fn krun_nonzero_exit_code_propagated() {
        skip_if_no_smolvm!();
        let mut proc = SmolvmProcess::spawn(
            "alpine",
            &[
                "/bin/sh".to_string(),
                "-c".to_string(),
                "exit 42".to_string(),
            ],
            &[],
        )
        .await
        .expect("spawn failed");

        let exit = tokio::time::timeout(Duration::from_secs(30), proc.wait())
            .await
            .expect("timed out")
            .expect("wait failed");
        assert_eq!(exit, 42, "expected exit code 42");
    }

    /// Env vars passed to spawn are visible inside the VM process.
    #[tokio::test]
    async fn krun_env_var_passed_to_process() {
        skip_if_no_smolvm!();
        let mut proc = SmolvmProcess::spawn(
            "alpine",
            &[
                "/bin/sh".to_string(),
                "-c".to_string(),
                "echo $KRUN_TEST_VAR".to_string(),
            ],
            &[("KRUN_TEST_VAR".to_string(), "krun-env-ok".to_string())],
        )
        .await
        .expect("spawn failed");

        let output = tokio::time::timeout(Duration::from_secs(30), proc.collect_stdout())
            .await
            .expect("timed out")
            .expect("collect_stdout failed");

        assert!(
            output.contains("krun-env-ok"),
            "env var must be visible inside VM; got: {output:?}"
        );
    }

    /// The hostname inside the VM must differ from the macOS host hostname.
    #[tokio::test]
    async fn krun_hostname_differs_from_host() {
        skip_if_no_smolvm!();
        let host_hostname = hostname::get()
            .ok()
            .and_then(|h| h.into_string().ok())
            .unwrap_or_default();

        let mut proc = SmolvmProcess::spawn(
            "alpine",
            &[
                "/bin/sh".to_string(),
                "-c".to_string(),
                "hostname".to_string(),
            ],
            &[],
        )
        .await
        .expect("spawn failed");

        let output = tokio::time::timeout(Duration::from_secs(30), proc.collect_stdout())
            .await
            .expect("timed out")
            .expect("collect_stdout failed");

        let vm_hostname = output.trim();
        assert_ne!(
            vm_hostname, host_hostname,
            "VM hostname must differ from macOS host hostname"
        );
    }
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("krun_conformance_tests: skipped (macOS only)");
}
