//! Showcase scenario: pause/resume via cgroup.freeze, narrated against real
//! cgroup limit files (`memory.max`, `cpu.weight`, `pids.max`).
//!
//! Mirrors the assertions in `crates/miniboxd/tests/cgroup_tests.rs` and
//! `crates/miniboxd/tests/system_tests.rs` (`test_e2e_run_with_memory_limit`,
//! `test_e2e_run_with_cpu_weight`), but driven through the shared showcase
//! `Reporter`/`Scenario` abstraction so the same code narrates in
//! `cargo xtask demo` and asserts silently in e2e tests.
//!
//! Pause/resume is native-adapter-only per `docs/core/FEATURE_MATRIX.mbx.md`
//! (cgroup.freeze has no equivalent on the macOS VM adapters or GKE). There is
//! no `BackendCapability` variant covering pause/resume specifically (see the
//! capability-matrix research: the enum covers Commit/BuildFromContext/
//! PushToRegistry/Checkpoint/Filesystem/Exec/Network/Tty/Pty/Metrics/
//! RegistryRouter/ImageLoader, nothing for pause/resume or cgroups), so this
//! scenario gates ad hoc on the bound backend descriptor's name instead of
//! `required_capability()`.

use anyhow::Context;

use super::reporter::Reporter;
use super::{Scenario, ScenarioCtx};

/// Memory limit passed to `--memory` (128 MiB), matching the value asserted
/// against `memory.max` in `system_tests.rs::test_e2e_run_with_memory_limit`.
const MEMORY_LIMIT_BYTES: u64 = 134_217_728;

/// CPU weight passed to `--cpu-weight`, matching
/// `system_tests.rs::test_e2e_run_with_cpu_weight`.
const CPU_WEIGHT: u64 = 250;

/// Default `pids.max` value the daemon applies when no explicit pids limit is
/// requested (there is no `--pids-limit` CLI flag today — verified against
/// `crates/mbx/src/main.rs`'s `Run` subcommand args, which exposes only
/// `--memory` and `--cpu-weight`). Matches
/// `cgroup_tests.rs::test_cgroup_pids_max_default`.
const DEFAULT_PIDS_MAX: &str = "1024";

/// Name of the only adapter that wires pause/resume today (see
/// `docs/core/FEATURE_MATRIX.mbx.md`: "pause/resume only native").
const NATIVE_ADAPTER_NAME: &str = "native";

/// Demonstrates pause/resume against a container started with cgroup limits,
/// asserting the underlying cgroup v2 files reflect the requested limits both
/// before and after a freeze/thaw cycle.
pub struct PauseResumeCgroup;

impl Scenario for PauseResumeCgroup {
    fn name(&self) -> &'static str {
        "pause_resume_cgroup"
    }

    fn required_capability(&self) -> Option<minibox_core::domain::BackendCapability> {
        // No BackendCapability variant covers pause/resume or cgroup limits;
        // gate ad hoc in `run()` against the backend descriptor's adapter
        // name instead (see module docs / capability_matrix research).
        None
    }

    fn run(&self, ctx: &ScenarioCtx, r: &dyn Reporter) -> anyhow::Result<()> {
        r.section(self.name());

        let adapter_name = ctx.descriptor.name;
        if adapter_name != NATIVE_ADAPTER_NAME {
            r.skip(&format!(
                "pause/resume is native-adapter-only (cgroup.freeze); current backend is '{adapter_name}'"
            ));
            return Ok(());
        }

        r.step("pulling alpine");
        ctx.pull_required("alpine");
        r.success("alpine present");

        r.step("starting container with --memory and --cpu-weight");
        let (mut cli_child, container_id) = ctx.spawn_run_background(&[
            "alpine",
            "--memory",
            &MEMORY_LIMIT_BYTES.to_string(),
            "--cpu-weight",
            &CPU_WEIGHT.to_string(),
            "--",
            "/bin/sleep",
            "30",
        ]);

        let running = ctx.wait_for_running(&container_id, std::time::Duration::from_secs(5));
        if !running {
            let _ = cli_child.kill();
            r.failure(&format!(
                "container {container_id} did not reach Running state within 5s"
            ));
            anyhow::bail!("container {container_id} never reached Running");
        }
        r.success(&format!("container {container_id} running"));

        let cgroup_dir = ctx.cgroup_root.join(&container_id);

        r.step("checking memory.max cgroup file");
        assert_cgroup_value(
            r,
            &cgroup_dir.join("memory.max"),
            &MEMORY_LIMIT_BYTES.to_string(),
            "memory.max",
        )?;

        r.step("checking cpu.weight cgroup file");
        assert_cgroup_value(
            r,
            &cgroup_dir.join("cpu.weight"),
            &CPU_WEIGHT.to_string(),
            "cpu.weight",
        )?;

        r.step("checking pids.max cgroup file (default limit)");
        assert_cgroup_value(
            r,
            &cgroup_dir.join("pids.max"),
            DEFAULT_PIDS_MAX,
            "pids.max",
        )?;

        r.step("pausing container via `mbx pause`");
        let pause_out = ctx.run_cli(&["pause", &container_id]);
        if !pause_out.success {
            r.failure(&format!(
                "pause failed.\nstdout: {}\nstderr: {}",
                pause_out.stdout, pause_out.stderr
            ));
            anyhow::bail!("mbx pause {container_id} failed");
        }
        r.success("pause command succeeded");

        r.step("checking cgroup.freeze reports frozen");
        assert_cgroup_value(r, &cgroup_dir.join("cgroup.freeze"), "1", "cgroup.freeze")?;

        r.step("resuming container via `mbx resume`");
        let resume_out = ctx.run_cli(&["resume", &container_id]);
        if !resume_out.success {
            r.failure(&format!(
                "resume failed.\nstdout: {}\nstderr: {}",
                resume_out.stdout, resume_out.stderr
            ));
            anyhow::bail!("mbx resume {container_id} failed");
        }
        r.success("resume command succeeded");

        r.step("checking cgroup.freeze reports thawed");
        assert_cgroup_value(r, &cgroup_dir.join("cgroup.freeze"), "0", "cgroup.freeze")?;

        r.step("checking limits survived the freeze/thaw cycle");
        assert_cgroup_value(
            r,
            &cgroup_dir.join("memory.max"),
            &MEMORY_LIMIT_BYTES.to_string(),
            "memory.max",
        )?;
        assert_cgroup_value(
            r,
            &cgroup_dir.join("cpu.weight"),
            &CPU_WEIGHT.to_string(),
            "cpu.weight",
        )?;
        r.success("cgroup limits unchanged after resume");

        r.step("stopping container");
        let _ = ctx.run_cli(&["stop", &container_id]);
        let _ = cli_child.wait();
        r.success("container stopped");

        Ok(())
    }
}

/// Read `path` and assert its trimmed contents equal `expected`, narrating the
/// comparison via `r.output()` and translating a mismatch into
/// `Reporter::failure()` plus a propagated error.
fn assert_cgroup_value(
    r: &dyn Reporter,
    path: &std::path::Path,
    expected: &str,
    label: &str,
) -> anyhow::Result<()> {
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("reading {label} at {}", path.display()))?;
    let actual = contents.trim();
    r.output(&format!("{label}: expected={expected} actual={actual}"));
    if actual != expected {
        r.failure(&format!(
            "{label} mismatch: expected {expected}, got {actual}"
        ));
        anyhow::bail!("{label} mismatch: expected {expected}, got {actual}");
    }
    r.success(&format!("{label} == {expected}"));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constants_match_system_tests_values() {
        // Pin these against the literals used in
        // crates/miniboxd/tests/system_tests.rs so a drift there is caught
        // here too.
        assert_eq!(MEMORY_LIMIT_BYTES, 134_217_728);
        assert_eq!(CPU_WEIGHT, 250);
        assert_eq!(DEFAULT_PIDS_MAX, "1024");
    }
}
