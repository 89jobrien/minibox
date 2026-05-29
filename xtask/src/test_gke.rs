//! GKE adapter test gates.
//!
//! - `test_gke_profile` — runs unit tests matching "gke" in their name.
//! - `test_gke_adapter` — runs GKE adapter integration tests.

use anyhow::{Context, Result};
use xshell::{Shell, cmd};

/// Run GKE profile-specific unit tests (any test with "gke" in the name).
pub fn test_gke_profile(sh: &Shell) -> Result<()> {
    eprintln!("--- test-gke-profile: running GKE profile tests ---");
    cmd!(sh, "cargo nextest run -p minibox -E test(~gke)")
        .run()
        .context("GKE profile tests failed")?;
    eprintln!("test-gke-profile passed");
    Ok(())
}

/// Run GKE adapter integration tests.
pub fn test_gke_adapter(sh: &Shell) -> Result<()> {
    eprintln!("--- test-gke-adapter: running GKE adapter integration tests ---");
    // Check if the dedicated GKE adapter test file has any test functions.
    let result = cmd!(sh, "cargo nextest run --test gke_adapter_isolation_tests").run();

    match result {
        Ok(()) => {
            eprintln!("test-gke-adapter passed");
            Ok(())
        }
        Err(e) => {
            // If nextest found no tests (exit code but no compilation error),
            // treat as "no tests found" rather than hard failure.
            let msg = format!("{e}");
            if msg.contains("no tests ran") {
                eprintln!("no GKE adapter tests found");
                Ok(())
            } else {
                Err(e).context("GKE adapter integration tests failed")
            }
        }
    }
}
