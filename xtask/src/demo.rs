//! `cargo xtask demo [--adapter <name>] [--filter <name>] [--strict]` —
//! narrated end-to-end walkthrough of the showcase scenario suite.
//!
//! This is a thin runner over the shared scenario modules in
//! `minibox_testsuite::showcase` — the same `Scenario` implementations that
//! back the silent e2e assertions in
//! `crates/miniboxd/tests/showcase_e2e_tests.rs`. Narration is driven
//! entirely by `NarratedReporter`; this file only sequences scenarios and
//! handles capability-based skipping.
//!
//! Scenarios run in dependency order: core lifecycle first, then
//! pause/resume + cgroups, then mounts/privileged, then bridge networking,
//! then build/commit/push last (most complex, calls domain adapters
//! directly rather than the CLI).
//!
//! By default the demo is advisory: scenario failures are narrated via
//! `Reporter::failure()` but do not affect the process exit code, matching
//! the previous ad hoc demo.rs behavior. Pass `--strict` to propagate any
//! scenario failure as a non-zero exit (`Err`), which also makes this a
//! real, if slow and narrated, test surface.

use anyhow::Result;
use minibox_testsuite::showcase::{self, NarratedReporter, Reporter, Scenario, ScenarioCtx};

/// Run the showcase demo. `filter` restricts to scenarios whose name
/// contains the given substring. `strict` propagates scenario failures as a
/// process error instead of only narrating them.
pub fn run_demo(adapter: &str, filter: Option<&str>, strict: bool) -> Result<()> {
    // SAFETY: xtask is single-threaded at this point in startup (no other
    // thread has been spawned yet that could race on the environment).
    unsafe {
        std::env::set_var("MINIBOX_ADAPTER", adapter);
    }

    let reporter = NarratedReporter::new();
    println!("=== minibox showcase demo ===");
    println!("adapter: {adapter}");

    let ctx = match ScenarioCtx::discover() {
        Ok(ctx) => ctx,
        Err(e) => {
            println!(
                "note: could not start a real daemon for the showcase demo ({e:#}). \
                 Run `cargo build --release` first, or set MINIBOX_TEST_BIN_DIR."
            );
            println!("=== demo complete (skipped — daemon unavailable) ===");
            return Ok(());
        }
    };

    let scenarios: Vec<&dyn Scenario> = vec![
        &showcase::lifecycle::Lifecycle,
        &showcase::pause_resume::PauseResumeCgroup,
        &showcase::mounts_privileged::MountsPrivileged,
        &showcase::networking::BridgeNetworking,
        &showcase::build_commit_push::BuildCommitPush,
    ];

    let mut had_failure = false;

    for scenario in scenarios {
        if let Some(f) = filter
            && !scenario.name().contains(f)
        {
            continue;
        }

        reporter.section(scenario.name());

        if let Some(cap) = scenario.required_capability()
            && !ctx.supports(cap)
        {
            reporter.skip(&format!(
                "{} not supported by adapter '{}'",
                scenario.name(),
                ctx.descriptor.name
            ));
            continue;
        }

        if let Err(e) = scenario.run(&ctx, &reporter) {
            had_failure = true;
            reporter.failure(&format!("{} failed: {e:#}", scenario.name()));
            // Continue to the next scenario rather than aborting the whole
            // demo — a single broken capability shouldn't hide the rest.
        }
    }

    reporter.summary();

    if strict && had_failure {
        anyhow::bail!("one or more showcase scenarios failed (--strict)");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_demo_returns_ok_when_daemon_unavailable() {
        // No real miniboxd/mbx binaries in a typical CI/dev environment
        // without a prior release build; discover() should fail
        // gracefully and run_demo should return Ok (non-strict default).
        // SAFETY: test-only env mutation; MINIBOX_TEST_BIN_DIR isn't
        // touched by other parallel tests in this crate.
        unsafe {
            std::env::set_var(
                "MINIBOX_TEST_BIN_DIR",
                "/nonexistent/showcase-demo-test-dir",
            );
        }
        let result = run_demo("smolvm", None, false);
        unsafe {
            std::env::remove_var("MINIBOX_TEST_BIN_DIR");
        }
        assert!(
            result.is_ok(),
            "non-strict demo should return Ok even without a daemon"
        );
    }
}
