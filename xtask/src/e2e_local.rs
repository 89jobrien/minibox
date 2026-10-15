//! Local macOS end-to-end smoke test for ops/ci-runner.
//!
//! Validates the self-hosted GitHub Actions runner design against a local minibox daemon,
//! exercising `mbx build` (smolvm adapter) and `mbx run --privileged` + `mbx exec` (krun adapter).

use anyhow::Result;
use xshell::{Shell, cmd};

/// Run the local macOS e2e smoke test.
pub fn run(sh: &Shell) -> Result<()> {
    let test_script = "ops/ci-runner/e2e-local.nu";
    cmd!(sh, "nu {test_script}").run()?;
    Ok(())
}
