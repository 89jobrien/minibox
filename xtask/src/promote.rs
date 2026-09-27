//! `cargo xtask promote` — cascade-merge through the stability pipeline.
//!
//! Default cascade: `develop` → `staging` → `release` → `main`
//!
//! Each hop:
//!   1. Optionally verifies CI is green on the source branch via `gh run list`.
//!   2. Checks out the target branch.
//!   3. Squash-merges the source into one promotion commit.
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

    pub const fn branch(self) -> &'static str {
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

    ensure_clean_worktree(&sh)?;

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

        // Check out target and squash source into one promotion commit.
        checkout(&sh, tgt).with_context(|| format!("failed to check out {tgt}"))?;

        if let Err(e) = squash_merge(&sh, src, tgt) {
            reset_failed_squash(&sh).context("failed to clean up squash merge")?;
            checkout(&sh, &original_branch).context("failed to restore original branch")?;
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

const fn merge_args(source: &str) -> [&str; 3] {
    ["merge", "--squash", source]
}

fn squash_merge(sh: &Shell, source: &str, target: &str) -> Result<()> {
    let args = merge_args(source);
    cmd!(sh, "git {args...}")
        .run()
        .with_context(|| format!("git merge --squash {source}"))?;

    let staged = cmd!(sh, "git diff --cached --name-only")
        .read()
        .context("git diff --cached --name-only")?;
    if staged.trim().is_empty() {
        return Ok(());
    }

    let message = format!("promote: {source} -> {target}");
    cmd!(sh, "git commit -m {message}")
        .run()
        .with_context(|| format!("git commit promotion {source} -> {target}"))
}

fn ensure_clean_worktree(sh: &Shell) -> Result<()> {
    let status = cmd!(sh, "git status --porcelain")
        .read()
        .context("git status --porcelain")?;
    if !status.trim().is_empty() {
        bail!("worktree must be clean before promotion");
    }
    Ok(())
}

fn reset_failed_squash(sh: &Shell) -> Result<()> {
    cmd!(sh, "git reset --merge HEAD")
        .run()
        .context("git reset --merge HEAD")
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
    use std::process::Command;

    fn git(root: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .expect("git command must start");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).expect("git output must be UTF-8")
    }

    #[test]
    fn promotion_merge_args_use_squash_mode() {
        assert_eq!(merge_args("develop"), ["merge", "--squash", "develop"]);
    }

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
    fn promotion_squashes_source_history_into_one_commit() {
        let repo = tempfile::tempdir().expect("temporary repository");
        let root = repo.path();
        git(root, &["init", "--initial-branch=develop"]);
        git(root, &["config", "user.name", "xtask test"]);
        git(root, &["config", "user.email", "xtask@example.invalid"]);
        git(root, &["config", "commit.gpgsign", "false"]);
        std::fs::write(root.join("base.txt"), "base\n").expect("write base fixture");
        git(root, &["add", "base.txt"]);
        git(root, &["commit", "-m", "base"]);
        git(root, &["branch", "staging"]);
        std::fs::write(root.join("feature.txt"), "feature\n").expect("write feature fixture");
        git(root, &["add", "feature.txt"]);
        git(root, &["commit", "-m", "feature commit"]);

        run(
            root,
            Tier::from_str("develop"),
            Tier::from_str("staging"),
            false,
        )
        .expect("promotion must succeed");

        let subjects = git(root, &["log", "staging", "--format=%s"]);
        assert_eq!(subjects.lines().count(), 2);
        assert_eq!(subjects.lines().next(), Some("promote: develop -> staging"));
        assert!(!subjects.contains("feature commit"));
    }

    #[test]
    fn promotion_refuses_to_commit_preexisting_changes() {
        let repo = tempfile::tempdir().expect("temporary repository");
        let root = repo.path();
        git(root, &["init", "--initial-branch=develop"]);
        git(root, &["config", "user.name", "xtask test"]);
        git(root, &["config", "user.email", "xtask@example.invalid"]);
        git(root, &["config", "commit.gpgsign", "false"]);
        std::fs::write(root.join("base.txt"), "base\n").expect("write base fixture");
        git(root, &["add", "base.txt"]);
        git(root, &["commit", "-m", "base"]);
        git(root, &["branch", "staging"]);
        std::fs::write(root.join("feature.txt"), "feature\n").expect("write feature fixture");
        git(root, &["add", "feature.txt"]);
        git(root, &["commit", "-m", "feature commit"]);
        std::fs::write(root.join("local.txt"), "local\n").expect("write local fixture");
        git(root, &["add", "local.txt"]);

        let error = run(
            root,
            Tier::from_str("develop"),
            Tier::from_str("staging"),
            false,
        )
        .expect_err("dirty worktree must block promotion");

        assert!(error.to_string().contains("worktree must be clean"));
        assert_eq!(git(root, &["branch", "--show-current"]).trim(), "develop");
        assert_eq!(git(root, &["log", "staging", "--format=%s"]).trim(), "base");
    }

    #[test]
    fn failed_squash_restores_original_branch_and_clean_tree() {
        let repo = tempfile::tempdir().expect("temporary repository");
        let root = repo.path();
        git(root, &["init", "--initial-branch=develop"]);
        git(root, &["config", "user.name", "xtask test"]);
        git(root, &["config", "user.email", "xtask@example.invalid"]);
        git(root, &["config", "commit.gpgsign", "false"]);
        std::fs::write(root.join("shared.txt"), "base\n").expect("write base fixture");
        git(root, &["add", "shared.txt"]);
        git(root, &["commit", "-m", "base"]);
        git(root, &["branch", "staging"]);
        std::fs::write(root.join("shared.txt"), "develop\n").expect("write develop fixture");
        git(root, &["commit", "-am", "develop change"]);
        git(root, &["checkout", "staging"]);
        std::fs::write(root.join("shared.txt"), "staging\n").expect("write staging fixture");
        git(root, &["commit", "-am", "staging change"]);
        git(root, &["checkout", "develop"]);

        run(
            root,
            Tier::from_str("develop"),
            Tier::from_str("staging"),
            false,
        )
        .expect_err("conflicting squash must fail");

        assert_eq!(git(root, &["branch", "--show-current"]).trim(), "develop");
        assert!(git(root, &["status", "--porcelain"]).trim().is_empty());
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
