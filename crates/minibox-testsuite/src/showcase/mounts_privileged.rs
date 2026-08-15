//! `mounts_privileged` scenario — bind mounts + `--privileged` mode.
//!
//! Demonstrates the only real end-to-end coverage the research found for
//! bind mounts combined with privileged mode: a Docker-in-Docker style run
//! where a nested `miniboxd` + `mbx` binary pair is bind-mounted into a
//! privileged container and used to pull/run an inner container (mirroring
//! `test_e2e_dind_pull_and_run` in `crates/miniboxd/tests/system_tests.rs`).
//!
//! Exact CLI syntax exercised (see `crates/mbx/src/main.rs`):
//!
//! ```text
//! mbx run --privileged \
//!     -v <host_miniboxd>:<container_miniboxd> \
//!     -v <host_mbx>:<container_mbx> \
//!     -v <host_cgroup>:<container_cgroup> \
//!     -v <host_data>:<container_data> \
//!     -v <host_run>:<container_run> \
//!     <image> -- <cmd>
//! ```
//!
//! Both `--privileged` and bind mounts (`-v`/`--mount`) are gated by the
//! daemon's deny-by-default `ContainerPolicy`. The daemon must be started
//! with `MINIBOX_ALLOW_BIND_MOUNTS` and `MINIBOX_ALLOW_PRIVILEGED` set,
//! otherwise the request is rejected before it ever reaches the runtime
//! adapter. Per `docs/core/FEATURE_MATRIX.mbx.md`, bind mounts and
//! privileged mode are native-Linux-only features — every other adapter
//! (colima, smolvm, krun, gke, winbox) lacks the capability entirely, so
//! this scenario skips gracefully rather than failing when run against a
//! non-native backend or a non-Linux host.

use minibox_core::domain::BackendCapability;

use super::reporter::Reporter;
use super::{Scenario, ScenarioCtx};

/// Bind mounts + privileged mode showcase scenario.
pub struct MountsPrivileged;

impl Scenario for MountsPrivileged {
    fn name(&self) -> &'static str {
        "mounts_privileged"
    }

    fn required_capability(&self) -> Option<BackendCapability> {
        // No `BackendCapability` variant maps 1:1 onto "bind mounts" or
        // "privileged mode" (see capability_matrix research); the harness's
        // enum-based skip mechanism can't express this gate, so we do the
        // check by hand in `run()` against the ad hoc adapter-name /
        // platform data documented in FEATURE_MATRIX.mbx.md instead.
        None
    }

    fn run(&self, ctx: &ScenarioCtx, r: &dyn Reporter) -> anyhow::Result<()> {
        r.section("mounts_privileged: bind mounts + --privileged (DinD)");

        if !ctx.is_native_linux() {
            r.skip(
                "bind mounts and --privileged are native-Linux-only \
                 (see docs/core/FEATURE_MATRIX.mbx.md); current adapter \
                 does not support this capability",
            );
            return Ok(());
        }

        r.step("preparing nested miniboxd bind-mount paths");
        let outer_miniboxd = ctx.workspace_binary("miniboxd")?;
        let outer_mbx = ctx.workspace_binary("mbx")?;
        let outer_cgroup_root = &ctx.cgroup_root;
        let outer_data_dir = &ctx.data_dir;
        let outer_run_dir = &ctx.run_dir;

        r.step("spawning daemon with bind-mount + privileged policy allowed");
        // Deny-by-default `ContainerPolicy` requires explicit opt-in before
        // the daemon will honor `-v`/`--mount` or `--privileged` requests.
        let fixture = ctx.spawn_daemon_with_env(&[
            ("MINIBOX_ALLOW_BIND_MOUNTS", "1"),
            ("MINIBOX_ALLOW_PRIVILEGED", "1"),
        ])?;

        r.step("pulling alpine");
        fixture.pull_required("alpine");

        let inner_miniboxd = "/nested/miniboxd";
        let inner_mbx = "/nested/mbx";
        let inner_cgroup = "/sys/fs/cgroup";
        let inner_data = "/var/lib/minibox";
        let inner_run = "/run/minibox";

        // Inner script: launch the nested daemon in the background, wait
        // for its socket, then use the nested CLI to pull + run a trivial
        // container, proving the privileged + bind-mounted binaries work.
        let script = format!(
            "{inner_miniboxd} & \
             for i in $(seq 1 50); do [ -S /run/minibox.sock ] && break; sleep 0.2; done; \
             {inner_mbx} pull alpine && {inner_mbx} run alpine -- /bin/echo dind-ok"
        );

        r.step("running nested container: --privileged + 5 bind mounts");
        let v_miniboxd = format!("{}:{inner_miniboxd}", outer_miniboxd.display());
        let v_mbx = format!("{}:{inner_mbx}", outer_mbx.display());
        let v_cgroup = format!("{}:{inner_cgroup}", outer_cgroup_root.display());
        let v_data = format!("{}:{inner_data}", outer_data_dir.display());
        let v_run = format!("{}:{inner_run}", outer_run_dir.display());

        let run_args: Vec<&str> = vec![
            "run",
            "--privileged",
            "-v",
            &v_miniboxd,
            "-v",
            &v_mbx,
            "-v",
            &v_cgroup,
            "-v",
            &v_data,
            "-v",
            &v_run,
            "alpine",
            "--",
            "/bin/sh",
            "-c",
            &script,
        ];

        let (exit_code, stdout, stderr) = fixture.run_cli_with_exit_code(&run_args);
        for line in stdout.lines().chain(stderr.lines()) {
            r.output(line);
        }

        if exit_code != 0 {
            r.failure(&format!(
                "nested DinD run exited with code {exit_code}\nstdout: {stdout}\nstderr: {stderr}"
            ));
            return Ok(());
        }

        if !stdout.contains("dind-ok") {
            r.failure(&format!(
                "expected nested container output to contain 'dind-ok', got:\n{stdout}"
            ));
            return Ok(());
        }

        r.success("nested miniboxd ran inside a privileged, bind-mounted container");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_matches_scenario_id() {
        let scenario = MountsPrivileged;
        assert_eq!(scenario.name(), "mounts_privileged");
    }

    #[test]
    fn declares_no_static_capability_gate() {
        // Gating is done ad hoc in `run()` against adapter/platform, not via
        // the `BackendCapability` enum (no variant covers bind mounts or
        // privileged mode today).
        let scenario = MountsPrivileged;
        assert!(scenario.required_capability().is_none());
    }
}
