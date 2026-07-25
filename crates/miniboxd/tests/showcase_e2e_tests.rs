//! Showcase e2e tests: run the shared `minibox_testsuite::showcase`
//! scenarios in silent assertion mode.
//!
//! These are the exact same `Scenario` implementations narrated by
//! `cargo xtask demo` — only the `Reporter` differs
//! (`SilentAssertReporter` here vs. `NarratedReporter` there), so there is
//! zero duplicated scenario logic between narrated and assertion modes.
//!
//! **Requirements:** Linux, root, cgroups v2, built `miniboxd`/`mbx`
//! binaries (`MINIBOX_TEST_BIN_DIR` or a prior `cargo build --release`).
//!
//! **Running:**
//! ```bash
//! just test-e2e
//! ```

#![cfg(target_os = "linux")]

use minibox::preflight;
use minibox_core::require_capability;
use minibox_testsuite::showcase::{Scenario, ScenarioCtx, SilentAssertReporter};
use serial_test::serial;

/// Discover a `ScenarioCtx` (spawns a real daemon), run `scenario` against
/// it with a fresh `SilentAssertReporter`, and gate on
/// `required_capability()` exactly as `cargo xtask demo` does — printing a
/// `SKIP:` line and returning early instead of failing when the active
/// backend doesn't support it.
fn run_scenario(scenario: &dyn Scenario) {
    let ctx =
        ScenarioCtx::discover().expect("discover showcase ScenarioCtx (build miniboxd+mbx first)");
    let reporter = SilentAssertReporter::new();

    if let Some(cap) = scenario.required_capability()
        && !ctx.supports(cap)
    {
        println!(
            "SKIP: {} unsupported by backend '{}'",
            scenario.name(),
            ctx.descriptor.name
        );
        return;
    }

    scenario
        .run(&ctx, &reporter)
        .unwrap_or_else(|e| panic!("{} scenario failed: {e:#}", scenario.name()));
}

#[test]
#[serial]
fn showcase_lifecycle() {
    let caps = preflight::probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, cgroups_v2, "requires cgroups v2");

    run_scenario(&minibox_testsuite::showcase::lifecycle::Lifecycle);
}

#[test]
#[serial]
fn showcase_pause_resume_cgroup() {
    let caps = preflight::probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, cgroups_v2, "requires cgroups v2");

    run_scenario(&minibox_testsuite::showcase::pause_resume::PauseResumeCgroup);
}

#[test]
#[serial]
fn showcase_mounts_privileged() {
    let caps = preflight::probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, cgroups_v2, "requires cgroups v2");

    run_scenario(&minibox_testsuite::showcase::mounts_privileged::MountsPrivileged);
}

/// Bridge networking requires root + a Linux kernel with bridge/iptables
/// support that hosted CI does not provide — same gate as the existing
/// `#[ignore]`d `bridge_setup` smoke test in
/// `crates/minibox/src/adapters/network/bridge.rs`. Run manually via
/// `just test-e2e` on the self-hosted VPS runner (see `.claude.local.md`).
#[test]
#[serial]
#[ignore = "requires root and Linux kernel with bridge support"]
fn showcase_bridge_networking() {
    let caps = preflight::probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, cgroups_v2, "requires cgroups v2");

    run_scenario(&minibox_testsuite::showcase::networking::BridgeNetworking);
}

#[test]
#[serial]
fn showcase_build_commit_push() {
    let caps = preflight::probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, cgroups_v2, "requires cgroups v2");

    run_scenario(&minibox_testsuite::showcase::build_commit_push::BuildCommitPush);
}
