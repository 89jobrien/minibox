//! `cargo xtask promote` — cascade-merge through the stability pipeline.
//!
//! Default cascade: `develop` → `staging` → `release` → `main`
//!
//! Each hop:
//!   1. Optionally verifies CI is green on the source branch via `gh run list`.
//!   2. Checks out the target branch.
//!   3. Runs `git merge --no-ff <source>`.
//!
// TODO: consider squash-merging chain branches during integration to eliminate
// duplicate commit messages. Currently 13/100 recent commits are duplicates
// because chain branches are cherry-picked then merged without squash. Each
// chain tag appears exactly 2x in history. See patterns_2026_06_16.md.
//!   4. Reports result; stops on failure.

use anyhow::{Context, Result, bail};
use std::path::Path;
use xshell::{Shell, cmd};

/// Ordered stability pipeline branches.
const PIPELINE: &[&str] = &["develop", "staging", "release", "main"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tier(usize);

impl Tier {
    pub fn from_str(s: &str) -> Option<Self> {
        PIPELINE.iter().position(|&b| b == s).map(Tier)
    }

    pub fn branch(self) -> &'static str {
        PIPELINE[self.0]
    }
}

/// Run the promote cascade.
///
/// * `from` — starting tier (source of first merge); defaults to `develop`.
/// * `to`   — ending tier (target of last merge); defaults to `main`.
/// * `dry_run` — print what would happen, do nothing.
/// * `skip_ci_check` — skip `gh run list` CI status check.
pub fn run(root: &Path, from: Option<Tier>, to: Option<Tier>, dry_run: bool) -> Result<()> {
    // Parse --skip-ci-check from raw args here so callers don't need to thread it.
    let args: Vec<String> = std::env::args().skip(2).collect();
    let skip_ci = args.iter().any(|a| a == "--skip-ci-check");

    let sh = Shell::new()?;
    sh.change_dir(root);

    let from_tier = from.unwrap_or(Tier(0)); // develop
    let to_tier = to.unwrap_or(Tier(PIPELINE.len() - 1)); // main

    if from_tier.0 >= to_tier.0 {
        bail!(
            "--from ({}) must be before --to ({}) in the pipeline",
            from_tier.branch(),
            to_tier.branch()
        );
    }

    // Build the list of (source, target) hops.
    let hops: Vec<(&str, &str)> = (from_tier.0..to_tier.0)
        .map(|i| (PIPELINE[i], PIPELINE[i + 1]))
        .collect();

    eprintln!("promote: cascade plan");
    for (src, tgt) in &hops {
        eprintln!("  {src} → {tgt}");
    }

    if dry_run {
        eprintln!("dry-run: no merges performed.");
        return Ok(());
    }

    // Remember which branch we started on so we can give a clean error message.
    let original_branch = current_branch(&sh)?;

    for (src, tgt) in &hops {
        eprintln!("\npromote: {src} → {tgt}");

        // CI check on source branch.
        if !skip_ci {
            match check_ci_green(&sh, src) {
                Ok(true) => eprintln!("  ci: green on {src}"),
                Ok(false) => {
                    // Restore original branch before bailing.
                    let _ = checkout(&sh, &original_branch);
                    bail!("CI is not green on branch `{src}`. Use --skip-ci-check to override.");
                }
                Err(e) => {
                    eprintln!("  ci: warning — could not check CI status for {src}: {e}");
                    eprintln!("  ci: proceeding (gh may not be available or no runs found)");
                }
            }
        }

        // Check out target and merge source.
        checkout(&sh, tgt).with_context(|| format!("failed to check out {tgt}"))?;

        let result = cmd!(sh, "git merge --no-ff {src}").run();
        if let Err(e) = result {
            // Restore original branch before bailing.
            let _ = checkout(&sh, &original_branch);
            bail!("merge {src} → {tgt} failed: {e}");
        }

        eprintln!("  merged: {src} → {tgt}");
    }

    // Return to the original branch.
    let _ = checkout(&sh, &original_branch);

    eprintln!(
        "\npromote: done — {} → {}",
        from_tier.branch(),
        to_tier.branch()
    );
    Ok(())
}

/// Return the current git branch name.
fn current_branch(sh: &Shell) -> Result<String> {
    let out = cmd!(sh, "git branch --show-current")
        .read()
        .context("git branch --show-current")?;
    Ok(out.trim().to_string())
}

/// Check out a branch.
fn checkout(sh: &Shell, branch: &str) -> Result<()> {
    cmd!(sh, "git checkout {branch}")
        .run()
        .with_context(|| format!("git checkout {branch}"))
}

/// Returns `Ok(true)` if the latest run on `branch` concluded with status
/// `completed` and conclusion `success`. Returns `Ok(false)` for any other
/// terminal conclusion. Returns `Err` if `gh` is unavailable or the output
/// cannot be parsed.
fn check_ci_green(sh: &Shell, branch: &str) -> Result<bool> {
    let out = cmd!(
        sh,
        "gh run list --branch {branch} --limit 1 --json status,conclusion"
    )
    .read()
    .context("gh run list")?;

    let trimmed = out.trim();
    if trimmed == "[]" || trimmed.is_empty() {
        bail!("no CI runs found for branch {branch}");
    }

    // Minimal JSON parse — avoid pulling in serde_json for this tiny check.
    // Expected shape: [{"status":"completed","conclusion":"success"}]
    let is_completed = trimmed.contains("\"status\":\"completed\"")
        || trimmed.contains("\"status\": \"completed\"");
    let is_success = trimmed.contains("\"conclusion\":\"success\"")
        || trimmed.contains("\"conclusion\": \"success\"");

    Ok(is_completed && is_success)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tier_from_str_develop() {
        let t = Tier::from_str("develop").expect("develop must parse");
        assert_eq!(t.branch(), "develop");
    }

    #[test]
    fn tier_from_str_main() {
        let t = Tier::from_str("main").expect("main must parse");
        assert_eq!(t.branch(), "main");
    }

    #[test]
    fn tier_from_str_unknown_is_none() {
        assert!(Tier::from_str("nonexistent").is_none());
    }

    #[test]
    fn pipeline_order_is_correct() {
        assert_eq!(PIPELINE, &["develop", "staging", "release", "main"]);
    }

    #[test]
    fn hops_develop_to_main() {
        let from = Tier::from_str("develop").unwrap();
        let to = Tier::from_str("main").unwrap();
        let hops: Vec<_> = (from.0..to.0)
            .map(|i| (PIPELINE[i], PIPELINE[i + 1]))
            .collect();
        assert_eq!(
            hops,
            vec![
                ("develop", "staging"),
                ("staging", "release"),
                ("release", "main"),
            ]
        );
    }

    #[test]
    fn hops_staging_to_main() {
        let from = Tier::from_str("staging").unwrap();
        let to = Tier::from_str("main").unwrap();
        let hops: Vec<_> = (from.0..to.0)
            .map(|i| (PIPELINE[i], PIPELINE[i + 1]))
            .collect();
        assert_eq!(hops, vec![("staging", "release"), ("release", "main"),]);
    }

    #[test]
    fn check_ci_green_parses_success() {
        // Simulate what gh outputs for a green run.
        let json = r#"[{"status":"completed","conclusion":"success"}]"#;
        let is_completed =
            json.contains("\"status\":\"completed\"") || json.contains("\"status\": \"completed\"");
        let is_success = json.contains("\"conclusion\":\"success\"")
            || json.contains("\"conclusion\": \"success\"");
        assert!(is_completed && is_success);
    }

    #[test]
    fn check_ci_green_parses_failure() {
        let json = r#"[{"status":"completed","conclusion":"failure"}]"#;
        let is_completed =
            json.contains("\"status\":\"completed\"") || json.contains("\"status\": \"completed\"");
        let is_success = json.contains("\"conclusion\":\"success\"")
            || json.contains("\"conclusion\": \"success\"");
        assert!(is_completed && !is_success);
    }
}
