//! `lifecycle` scenario — core container lifecycle walkthrough.
//!
//! Exercises the primary happy-path lifecycle end to end, using the exact
//! CLI verbs/flags documented in `crates/miniboxd/tests/system_tests.rs`:
//!
//! ```text
//! mbx pull alpine
//! mbx run alpine -- /bin/echo hello-lifecycle
//! mbx run alpine -- /bin/sleep 30      (background, for exec/logs/ps/stop)
//! mbx exec <id> -- /bin/echo exec-ok    (native-only, setns + PTY)
//! mbx logs <id>
//! mbx ps                                (poll for "Running")
//! mbx stop <id>
//! mbx rm <id>
//! ```
//!
//! Per `docs/core/FEATURE_MATRIX.mbx.md`, `exec` and `logs` are only fully
//! supported on the native Linux adapter (colima reports "Limited"; smolvm,
//! krun, gke, and winbox do not support them at all). Those two steps check
//! `BackendCapability::Exec` against the bound `BackendDescriptor` and call
//! `Reporter::skip()` with a clear reason rather than failing hard when the
//! active backend lacks the capability — mirroring the
//! `ConformanceTest::required_capability()` auto-skip convention described
//! in the `capability_matrix` research. The scenario as a whole has no single
//! required capability (`required_capability()` returns `None`) because the
//! pull/run/ps/stop/rm steps are expected to work on every adapter; only the
//! exec/logs sub-steps are gated individually.

use minibox_core::domain::BackendCapability;

use super::reporter::Reporter;
use super::{Scenario, ScenarioCtx};

/// Core lifecycle showcase scenario: pull, run, exec, logs, ps, stop, rm.
pub struct Lifecycle;

impl Scenario for Lifecycle {
    fn name(&self) -> &'static str {
        "lifecycle"
    }

    fn required_capability(&self) -> Option<BackendCapability> {
        // The bulk of this scenario (pull/run/ps/stop/rm) works on every
        // adapter; only the exec/logs sub-steps require `Exec` and those
        // are gated individually inside `run()` via `Reporter::skip()`.
        None
    }

    fn run(&self, ctx: &ScenarioCtx, r: &dyn Reporter) -> anyhow::Result<()> {
        r.section("lifecycle: pull, run, exec, logs, ps, stop, rm");

        r.step("pulling alpine");
        let fixture = ctx.spawn_daemon()?;
        fixture.pull_required("alpine");
        r.success("alpine pulled");

        r.step("running alpine -- /bin/echo hello-lifecycle");
        let echo_out = fixture.run_cli(&["run", "alpine", "--", "/bin/echo", "hello-lifecycle"]);
        for line in echo_out.stdout.lines().chain(echo_out.stderr.lines()) {
            r.output(line);
        }
        if !echo_out.success {
            r.failure(&format!(
                "echo run failed\nstdout: {}\nstderr: {}",
                echo_out.stdout, echo_out.stderr
            ));
            return Ok(());
        }
        if !echo_out.stdout.contains("hello-lifecycle") {
            r.failure(&format!(
                "expected echo output to contain 'hello-lifecycle', got:\n{}",
                echo_out.stdout
            ));
            return Ok(());
        }
        r.success("echo container ran and produced expected output");

        r.step("starting long-running container: /bin/sleep 30");
        let (mut cli_child, container_id) =
            fixture.spawn_run_background(&["alpine", "--", "/bin/sleep", "30"]);

        r.step("polling ps for Running state");
        let timeout = ctx.running_timeout();
        let appeared = fixture.wait_for_running(&container_id, timeout);
        if !appeared {
            r.failure(&format!(
                "container {container_id} did not appear as Running in ps within {timeout:?}"
            ));
            let _ = fixture.run_cli(&["stop", &container_id]);
            let _ = cli_child.wait();
            return Ok(());
        }
        r.success(&format!("container {container_id} is Running"));

        r.step("checking exec capability (native-only per FEATURE_MATRIX)");
        if ctx.supports(BackendCapability::Exec) {
            r.step("exec: /bin/echo exec-ok (setns + PTY)");
            let exec_out = fixture.run_cli(&["exec", &container_id, "--", "/bin/echo", "exec-ok"]);
            for line in exec_out.stdout.lines().chain(exec_out.stderr.lines()) {
                r.output(line);
            }
            if exec_out.success && exec_out.stdout.contains("exec-ok") {
                r.success("exec ran inside the running container");
            } else {
                r.failure(&format!(
                    "exec failed or missing expected output\nstdout: {}\nstderr: {}",
                    exec_out.stdout, exec_out.stderr
                ));
            }

            r.step("logs: fetching stored stdout/stderr");
            let logs_out = fixture.run_cli(&["logs", &container_id]);
            for line in logs_out.stdout.lines() {
                r.output(line);
            }
            if logs_out.success {
                r.success("logs retrieved for running container");
            } else {
                r.failure(&format!(
                    "logs command failed\nstdout: {}\nstderr: {}",
                    logs_out.stdout, logs_out.stderr
                ));
            }
        } else {
            r.skip(
                "exec/logs require BackendCapability::Exec, which is native-Linux-only \
                 per docs/core/FEATURE_MATRIX.mbx.md (colima is Limited; smolvm/krun/gke/winbox \
                 do not support it)",
            );
        }

        r.step(&format!("stopping container {container_id}"));
        let stop_out = fixture.run_cli(&["stop", &container_id]);
        let _ = cli_child.wait();
        if !stop_out.success {
            r.failure(&format!(
                "stop failed\nstdout: {}\nstderr: {}",
                stop_out.stdout, stop_out.stderr
            ));
            return Ok(());
        }
        r.success("container stopped");

        r.step(&format!("removing container {container_id}"));
        let rm_out = fixture.run_cli(&["rm", &container_id]);
        if !rm_out.success {
            r.failure(&format!(
                "rm failed\nstdout: {}\nstderr: {}",
                rm_out.stdout, rm_out.stderr
            ));
            return Ok(());
        }
        r.success("container removed");

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_matches_scenario_id() {
        let scenario = Lifecycle;
        assert_eq!(scenario.name(), "lifecycle");
    }

    #[test]
    fn declares_no_top_level_capability_gate() {
        // exec/logs are gated individually inside run(); the scenario as a
        // whole (pull/run/ps/stop/rm) is expected to work on every adapter.
        let scenario = Lifecycle;
        assert!(scenario.required_capability().is_none());
    }
}
