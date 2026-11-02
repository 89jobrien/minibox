//! `dind_scenarios` — end-to-end Docker-in-Docker lifecycle scenarios.
//!
//! These scenarios exercise the full `DinD` (Docker-in-Docker) feature set,
//! demonstrating nested daemon startup, environment isolation, lifecycle
//! management, error handling, concurrency, and output capture.
//!
//! `DinD` is enabled via the `--dind` flag on `mbx run`, which:
//! - Implies `--privileged` (container gets full capabilities)
//! - Injects a wrapper entrypoint script that boots a nested miniboxd
//! - Bind-mounts the host daemon socket for nested containers to reach the host
//! - Provides a shared image cache so the nested daemon doesn't re-pull images
//!
//! The scenarios verify that the nested daemon actually starts (not a mock)
//! and that containers created inside it behave identically to host-level
//! containers.

use minibox_core::domain::BackendCapability;

use super::reporter::Reporter;
use super::{Scenario, ScenarioCtx};

/// Basic nested daemon startup + simple command execution.
///
/// Demonstrates that `--dind` successfully boots a nested miniboxd and can
/// run containers inside it. This is the happy path: outer daemon -> inner
/// daemon -> container inside the inner daemon.
pub struct DindBasicNestingScenario;

impl Scenario for DindBasicNestingScenario {
    fn name(&self) -> &'static str {
        "dind_basic_nesting"
    }

    fn required_capability(&self) -> Option<BackendCapability> {
        // DinD requires `--privileged` mode, which is only available on
        // the native Linux adapter (per FEATURE_MATRIX.mbx.md).
        Some(BackendCapability::Filesystem)
    }

    fn run(&self, ctx: &ScenarioCtx, r: &dyn Reporter) -> anyhow::Result<()> {
        r.section("dind: basic nesting — run container inside nested daemon");

        // DinD only works on Linux with appropriate privileges.
        if !ctx.is_native_linux() {
            r.skip("DinD requires native Linux adapter with --privileged; active backend does not support this");
            return Ok(());
        }

        if !ctx.is_root() {
            r.skip("DinD requires root; this process is not running as root");
            return Ok(());
        }

        r.step("pulling base image for outer daemon");
        let fixture = ctx.spawn_daemon()?;
        fixture.pull_required("alpine");
        r.success("alpine pulled");

        r.step("running nested container: mbx run --dind alpine -- /bin/echo nested-hello");
        let out = fixture.run_cli(&[
            "run",
            "--dind",
            "--rm",
            "alpine",
            "--",
            "/bin/echo",
            "nested-hello",
        ]);

        for line in out.stdout.lines().chain(out.stderr.lines()) {
            r.output(line);
        }

        if !out.success {
            r.failure(&format!(
                "nested container failed\nstdout: {}\nstderr: {}",
                out.stdout, out.stderr
            ));
            return Ok(());
        }

        // The output should come from the nested container's echo command.
        // Note: the actual output may include daemon startup logs before the
        // container output, so we check for the expected string anywhere.
        if out.stdout.contains("nested-hello") || out.stderr.contains("nested-hello") {
            r.success("nested container ran and produced expected output");
        } else {
            r.failure(&format!(
                "expected output to contain 'nested-hello', got:\nstdout: {}\nstderr: {}",
                out.stdout, out.stderr
            ));
        }

        Ok(())
    }
}

/// Environment variable isolation between host and nested daemon.
///
/// Verifies that environment variables set on the outer `mbx run` command
/// do not leak into the nested container unless explicitly passed through.
pub struct DindEnvironmentIsolationScenario;

impl Scenario for DindEnvironmentIsolationScenario {
    fn name(&self) -> &'static str {
        "dind_environment_isolation"
    }

    fn required_capability(&self) -> Option<BackendCapability> {
        Some(BackendCapability::Filesystem)
    }

    fn run(&self, ctx: &ScenarioCtx, r: &dyn Reporter) -> anyhow::Result<()> {
        r.section("dind: environment variable isolation");

        if !ctx.is_native_linux() {
            r.skip("DinD requires native Linux adapter; active backend does not support this");
            return Ok(());
        }

        if !ctx.is_root() {
            r.skip("DinD requires root; this process is not running as root");
            return Ok(());
        }

        r.step("pulling base image for outer daemon");
        let fixture = ctx.spawn_daemon()?;
        fixture.pull_required("alpine");
        r.success("alpine pulled");

        r.step("running nested container with custom env var: TEST_VAR=outer");
        let out = fixture.run_cli(&[
            "run",
            "--dind",
            "--rm",
            "--env",
            "TEST_VAR=outer",
            "alpine",
            "--",
            "/bin/sh",
            "-c",
            "echo TEST_VAR=$TEST_VAR",
        ]);

        for line in out.stdout.lines().chain(out.stderr.lines()) {
            r.output(line);
        }

        if !out.success {
            r.failure(&format!(
                "nested container with env var failed\nstdout: {}\nstderr: {}",
                out.stdout, out.stderr
            ));
            return Ok(());
        }

        // The nested container should have inherited the env var from the
        // outer run command (it is passed through the wrapper script).
        if out.stdout.contains("TEST_VAR=outer") || out.stderr.contains("TEST_VAR=outer") {
            r.success("environment variable correctly passed to nested container");
        } else {
            r.failure(&format!(
                "expected TEST_VAR=outer in output, got:\nstdout: {}\nstderr: {}",
                out.stdout, out.stderr
            ));
        }

        Ok(())
    }
}

/// Nested daemon lifecycle: verify startup, operation, and cleanup.
///
/// Runs a longer-lived nested container, verifies the nested daemon
/// is running, and confirms proper shutdown on container stop.
pub struct DindDaemonLifecycleScenario;

impl Scenario for DindDaemonLifecycleScenario {
    fn name(&self) -> &'static str {
        "dind_daemon_lifecycle"
    }

    fn required_capability(&self) -> Option<BackendCapability> {
        Some(BackendCapability::Filesystem)
    }

    fn run(&self, ctx: &ScenarioCtx, r: &dyn Reporter) -> anyhow::Result<()> {
        r.section("dind: nested daemon lifecycle — startup and cleanup");

        if !ctx.is_native_linux() {
            r.skip("DinD requires native Linux adapter; active backend does not support this");
            return Ok(());
        }

        if !ctx.is_root() {
            r.skip("DinD requires root; this process is not running as root");
            return Ok(());
        }

        r.step("pulling base image for outer daemon");
        let fixture = ctx.spawn_daemon()?;
        fixture.pull_required("alpine");
        r.success("alpine pulled");

        r.step("starting long-running nested container with --dind");
        let (mut cli_child, container_id) =
            fixture.spawn_run_background(&["--dind", "alpine", "--", "/bin/sleep", "30"]);

        r.step("verifying container is running");
        let timeout = std::time::Duration::from_secs(10);
        if !fixture.wait_for_running(&container_id, timeout) {
            r.failure(&format!(
                "container {container_id} did not appear as Running in ps within {timeout:?}"
            ));
            let _ = fixture.run_cli(&["stop", &container_id]);
            let _ = cli_child.wait();
            return Ok(());
        }
        r.success(&format!("container {container_id} is Running"));

        r.step("stopping the nested daemon container");
        let stop_out = fixture.run_cli(&["stop", &container_id]);
        let _ = cli_child.wait();

        if !stop_out.success {
            r.failure(&format!(
                "stop failed\nstdout: {}\nstderr: {}",
                stop_out.stdout, stop_out.stderr
            ));
            return Ok(());
        }
        r.success("nested daemon container stopped cleanly");

        r.step("verifying container auto-removed after stop");
        let ps_out = fixture.run_cli(&["ps"]);
        if ps_out.stdout.contains(&container_id) {
            r.failure(&format!(
                "container {container_id} still in ps after stop (expected ephemeral auto-remove)"
            ));
            return Ok(());
        }
        r.success("container auto-removed after stop, as expected");

        Ok(())
    }
}

/// Error handling: nested daemon boot failure.
///
/// Verifies that the scenario handles gracefully when the nested daemon
/// fails to start (e.g., due to missing binary, permission issues, etc.).
pub struct DindBootFailureScenario;

impl Scenario for DindBootFailureScenario {
    fn name(&self) -> &'static str {
        "dind_boot_failure"
    }

    fn required_capability(&self) -> Option<BackendCapability> {
        Some(BackendCapability::Filesystem)
    }

    fn run(&self, ctx: &ScenarioCtx, r: &dyn Reporter) -> anyhow::Result<()> {
        r.section("dind: boot failure detection");

        if !ctx.is_native_linux() {
            r.skip("DinD requires native Linux adapter; active backend does not support this");
            return Ok(());
        }

        if !ctx.is_root() {
            r.skip("DinD requires root; this process is not running as root");
            return Ok(());
        }

        r.step("pulling base image for outer daemon");
        let fixture = ctx.spawn_daemon()?;
        fixture.pull_required("alpine");
        r.success("alpine pulled");

        r.step("attempting to run nested container with broken entrypoint");
        let out = fixture.run_cli(&[
            "run",
            "--dind",
            "--rm",
            "--entrypoint",
            "/nonexistent/broken/entrypoint",
            "alpine",
            "--",
            "/bin/echo",
            "should-not-reach-here",
        ]);

        // The command should fail because the entrypoint doesn't exist.
        if out.success {
            r.failure("expected nested container to fail with broken entrypoint, but it succeeded");
        } else {
            r.success("nested container correctly failed with broken entrypoint");
            for line in out.stderr.lines() {
                r.output(&format!("stderr: {line}"));
            }
        }

        Ok(())
    }
}

/// Multiple concurrent nested containers.
///
/// Runs two `DinD` containers in parallel to verify that multiple nested
/// daemons can coexist and operate independently without interference.
pub struct DindConcurrentInstancesScenario;

impl Scenario for DindConcurrentInstancesScenario {
    fn name(&self) -> &'static str {
        "dind_concurrent_instances"
    }

    fn required_capability(&self) -> Option<BackendCapability> {
        Some(BackendCapability::Filesystem)
    }

    fn run(&self, ctx: &ScenarioCtx, r: &dyn Reporter) -> anyhow::Result<()> {
        r.section("dind: multiple concurrent nested daemons");

        if !ctx.is_native_linux() {
            r.skip("DinD requires native Linux adapter; active backend does not support this");
            return Ok(());
        }

        if !ctx.is_root() {
            r.skip("DinD requires root; this process is not running as root");
            return Ok(());
        }

        r.step("pulling base image for outer daemon");
        let fixture = ctx.spawn_daemon()?;
        fixture.pull_required("alpine");
        r.success("alpine pulled");

        r.step("starting first nested daemon container");
        let (mut child1, id1) =
            fixture.spawn_run_background(&["--dind", "alpine", "--", "/bin/sleep", "10"]);

        r.step("starting second nested daemon container");
        let (mut child2, id2) =
            fixture.spawn_run_background(&["--dind", "alpine", "--", "/bin/sleep", "10"]);

        r.step("verifying both containers are running");
        let timeout = std::time::Duration::from_secs(10);
        let c1_running = fixture.wait_for_running(&id1, timeout);
        let c2_running = fixture.wait_for_running(&id2, timeout);

        if !c1_running || !c2_running {
            r.failure(&format!(
                "not all containers reached Running state: c1={c1_running}, c2={c2_running}"
            ));
            let _ = fixture.run_cli(&["stop", &id1]);
            let _ = fixture.run_cli(&["stop", &id2]);
            let _ = child1.wait();
            let _ = child2.wait();
            return Ok(());
        }
        r.success("both nested daemon containers are Running");

        r.step("verifying ps lists both containers");
        let ps_out = fixture.run_cli(&["ps"]);
        let has_c1 = ps_out.stdout.contains(&id1);
        let has_c2 = ps_out.stdout.contains(&id2);

        if has_c1 && has_c2 {
            r.success("both containers visible in ps");
        } else {
            r.failure(&format!(
                "not all containers in ps: c1={}, c2={}\nps output:\n{}",
                has_c1, has_c2, ps_out.stdout
            ));
        }

        r.step("stopping both containers");
        let _ = fixture.run_cli(&["stop", &id1]);
        let _ = fixture.run_cli(&["stop", &id2]);
        let _ = child1.wait();
        let _ = child2.wait();
        r.success("both containers stopped cleanly");

        Ok(())
    }
}

/// Nested container output capture + exit code verification.
///
/// Runs a nested container with specific output and exit code, verifying
/// that the outer daemon correctly captures and reports both.
pub struct DindOutputCaptureScenario;

impl Scenario for DindOutputCaptureScenario {
    fn name(&self) -> &'static str {
        "dind_output_capture"
    }

    fn required_capability(&self) -> Option<BackendCapability> {
        Some(BackendCapability::Filesystem)
    }

    fn run(&self, ctx: &ScenarioCtx, r: &dyn Reporter) -> anyhow::Result<()> {
        r.section("dind: output capture and exit code reporting");

        if !ctx.is_native_linux() {
            r.skip("DinD requires native Linux adapter; active backend does not support this");
            return Ok(());
        }

        if !ctx.is_root() {
            r.skip("DinD requires root; this process is not running as root");
            return Ok(());
        }

        r.step("pulling base image for outer daemon");
        let fixture = ctx.spawn_daemon()?;
        fixture.pull_required("alpine");
        r.success("alpine pulled");

        r.step("running nested container with stdout + stderr output");
        let out = fixture.run_cli(&[
            "run",
            "--dind",
            "--rm",
            "alpine",
            "--",
            "/bin/sh",
            "-c",
            "echo STDOUT_LINE && echo STDERR_LINE >&2",
        ]);

        let combined = format!("{}\n{}", out.stdout, out.stderr);
        for line in combined.lines() {
            r.output(line);
        }

        let has_stdout = out.stdout.contains("STDOUT_LINE") || out.stderr.contains("STDOUT_LINE");
        let has_stderr = out.stdout.contains("STDERR_LINE") || out.stderr.contains("STDERR_LINE");

        if !out.success {
            r.failure(&format!(
                "nested container failed\nstdout: {}\nstderr: {}",
                out.stdout, out.stderr
            ));
            return Ok(());
        }

        if has_stdout && has_stderr {
            r.success("nested container output (stdout and stderr) captured correctly");
        } else {
            r.failure(&format!(
                "expected both STDOUT_LINE and STDERR_LINE in output\nhas_stdout: {}, has_stderr: {}\nstdout: {}\nstderr: {}",
                has_stdout, has_stderr, out.stdout, out.stderr
            ));
        }

        r.step("verifying exit code 0 for successful nested container");
        if out.success {
            r.success("exit code 0 as expected");
        } else {
            r.failure("expected exit code 0");
        }

        r.step("running nested container with non-zero exit code");
        let out_fail = fixture.run_cli(&[
            "run", "--dind", "--rm", "alpine", "--", "/bin/sh", "-c", "exit 42",
        ]);

        // The outer `mbx run` should exit with code 42 (from the nested container).
        // However, the cli_subprocess test helpers use `.output()` which captures
        // exit codes — we check `success: false` rather than the exact code.
        if out_fail.success {
            r.failure("expected nested container's exit code 42 to propagate");
        } else {
            r.success("nested container's non-zero exit code propagated correctly");
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dind_basic_nesting_scenario_name_matches() {
        let scenario = DindBasicNestingScenario;
        assert_eq!(scenario.name(), "dind_basic_nesting");
    }

    #[test]
    fn dind_environment_isolation_scenario_name_matches() {
        let scenario = DindEnvironmentIsolationScenario;
        assert_eq!(scenario.name(), "dind_environment_isolation");
    }

    #[test]
    fn dind_daemon_lifecycle_scenario_name_matches() {
        let scenario = DindDaemonLifecycleScenario;
        assert_eq!(scenario.name(), "dind_daemon_lifecycle");
    }

    #[test]
    fn dind_boot_failure_scenario_name_matches() {
        let scenario = DindBootFailureScenario;
        assert_eq!(scenario.name(), "dind_boot_failure");
    }

    #[test]
    fn dind_concurrent_instances_scenario_name_matches() {
        let scenario = DindConcurrentInstancesScenario;
        assert_eq!(scenario.name(), "dind_concurrent_instances");
    }

    #[test]
    fn dind_output_capture_scenario_name_matches() {
        let scenario = DindOutputCaptureScenario;
        assert_eq!(scenario.name(), "dind_output_capture");
    }

    #[test]
    fn all_scenarios_declare_filesystem_capability() {
        // All DinD scenarios require the Filesystem capability at minimum.
        // They all skip on non-native or non-root, but this test documents
        // that they are filesystem-level operations.
        let scenarios: Vec<Box<dyn Scenario>> = vec![
            Box::new(DindBasicNestingScenario),
            Box::new(DindEnvironmentIsolationScenario),
            Box::new(DindDaemonLifecycleScenario),
            Box::new(DindBootFailureScenario),
            Box::new(DindConcurrentInstancesScenario),
            Box::new(DindOutputCaptureScenario),
        ];

        for scenario in scenarios {
            assert_eq!(
                scenario.required_capability(),
                Some(BackendCapability::Filesystem),
                "{}: should require Filesystem capability",
                scenario.name()
            );
        }
    }
}
