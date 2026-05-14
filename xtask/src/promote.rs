use anyhow::{Context, Result};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    Dev,
    Testing,
    Staging,
    Main,
}

impl Tier {
    pub fn sequence() -> Vec<Tier> {
        vec![Tier::Dev, Tier::Testing, Tier::Staging, Tier::Main]
    }

    pub fn next(self) -> Option<Tier> {
        match self {
            Tier::Dev => Some(Tier::Testing),
            Tier::Testing => Some(Tier::Staging),
            Tier::Staging => Some(Tier::Main),
            Tier::Main => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Tier::Dev => "dev",
            Tier::Testing => "testing",
            Tier::Staging => "staging",
            Tier::Main => "main",
        }
    }

    #[allow(dead_code)]
    pub fn branch_name(self) -> &'static str {
        match self {
            Tier::Dev | Tier::Testing | Tier::Staging => "develop",
            Tier::Main => "main",
        }
    }

    pub fn from_str(s: &str) -> Option<Tier> {
        match s {
            "dev" => Some(Tier::Dev),
            "testing" => Some(Tier::Testing),
            "staging" => Some(Tier::Staging),
            "main" => Some(Tier::Main),
            _ => None,
        }
    }

    /// Gates owned by this tier (not inherited from predecessors).
    pub fn own_gates(self) -> Vec<&'static str> {
        match self {
            Tier::Dev => vec!["lint", "test-unit"],
            Tier::Testing => vec!["test-conformance", "test-property", "borrow-fixtures"],
            Tier::Staging => vec!["prepush", "coverage-check"],
            Tier::Main => vec!["check-no-unwrap", "check-protocol-drift", "lint-docs"],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromoteState {
    pub current_tier: Tier,
    pub promoted_at: String,
}

impl PromoteState {
    pub fn to_toml(&self) -> String {
        format!(
            "current_tier = \"{}\"\npromoted_at = \"{}\"\n",
            self.current_tier.name(),
            self.promoted_at
        )
    }

    pub fn from_toml(s: &str) -> Result<Self> {
        let mut current_tier = None;
        let mut promoted_at = String::new();
        for line in s.lines() {
            if let Some(val) = line.strip_prefix("current_tier = \"") {
                let val = val.trim_end_matches('"');
                current_tier = Tier::from_str(val);
            }
            if let Some(val) = line.strip_prefix("promoted_at = \"") {
                promoted_at = val.trim_end_matches('"').to_string();
            }
        }
        Ok(PromoteState {
            current_tier: current_tier.context("missing or unknown current_tier in state file")?,
            promoted_at,
        })
    }

    pub fn load(root: &Path) -> Option<Self> {
        let path = root.join(".minibox-promote-state");
        let content = std::fs::read_to_string(&path).ok()?;
        Self::from_toml(&content).ok()
    }

    pub fn save(&self, root: &Path) -> Result<()> {
        let path = root.join(".minibox-promote-state");
        std::fs::write(&path, self.to_toml()).context("failed to write promote state")?;
        Ok(())
    }
}

/// Returns the full ordered gate list for promoting from `from` to `to` (inclusive).
/// Each entry is (tier, gate_name).
pub fn build_promotion_plan(from: Tier, to: Tier) -> Vec<(Tier, &'static str)> {
    let sequence = Tier::sequence();
    let from_idx = sequence.iter().position(|t| *t == from).unwrap_or(0);
    let to_idx = sequence
        .iter()
        .position(|t| *t == to)
        .unwrap_or(0)
        .min(sequence.len() - 1);

    let mut plan = Vec::new();
    for tier in &sequence[from_idx..=to_idx] {
        for gate in tier.own_gates() {
            plan.push((*tier, gate));
        }
    }
    plan
}

pub fn run(root: &Path, from: Option<Tier>, to: Option<Tier>, dry_run: bool) -> Result<()> {
    let sh = xshell::Shell::new()?;
    sh.change_dir(root);

    let current_state = PromoteState::load(root);
    let from_tier = from
        .or_else(|| current_state.as_ref().map(|s| s.current_tier))
        .unwrap_or(Tier::Dev);
    let to_tier = to
        .or_else(|| from_tier.next())
        .context("already at main tier — nothing to promote to")?;

    // Dev tier: verify branch is descended from develop
    if from_tier == Tier::Dev {
        let result = xshell::cmd!(sh, "git merge-base --is-ancestor develop HEAD").run();
        if result.is_err() {
            anyhow::bail!(
                "current branch is not descended from `develop`. \
                 All feature branches must be cut from develop."
            );
        }
    }

    let plan = build_promotion_plan(from_tier, to_tier);

    eprintln!("Promotion plan: {} -> {}", from_tier.name(), to_tier.name());
    for (tier, gate) in &plan {
        eprintln!("  [{:8}] {}", tier.name(), gate);
    }

    if dry_run {
        eprintln!("Dry run — no gates executed.");
        return Ok(());
    }

    for (tier, gate) in &plan {
        eprintln!("  Running [{:8}] {} ...", tier.name(), gate);
        let result = xshell::cmd!(sh, "cargo xtask {gate}").run();
        if let Err(e) = result {
            anyhow::bail!("[{}] gate `{}` failed: {}", tier.name(), gate, e);
        }
        eprintln!("  ok: {}", gate);
    }

    let now = chrono::Utc::now().to_rfc3339();
    let new_state = PromoteState {
        current_tier: to_tier,
        promoted_at: now,
    };
    new_state.save(root)?;
    eprintln!("Promoted to: {}", to_tier.name());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tier_sequence_is_dev_testing_staging_main() {
        let tiers = Tier::sequence();
        assert_eq!(
            tiers,
            vec![Tier::Dev, Tier::Testing, Tier::Staging, Tier::Main]
        );
    }

    #[test]
    fn tier_next_dev_is_testing() {
        assert_eq!(Tier::Dev.next(), Some(Tier::Testing));
    }

    #[test]
    fn tier_next_main_is_none() {
        assert_eq!(Tier::Main.next(), None);
    }

    #[test]
    fn promote_state_round_trips() {
        let state = PromoteState {
            current_tier: Tier::Testing,
            promoted_at: "2026-05-14T10:00:00Z".to_string(),
        };
        let toml = state.to_toml();
        let parsed = PromoteState::from_toml(&toml).expect("round-trip parse must succeed");
        assert_eq!(parsed.current_tier, Tier::Testing);
    }

    #[test]
    fn dry_run_returns_gate_list_without_running() {
        let plan = build_promotion_plan(Tier::Dev, Tier::Testing);
        assert!(
            plan.iter()
                .any(|(t, g)| *t == Tier::Dev && g.contains("lint")),
            "plan must include dev/lint"
        );
        assert!(
            plan.iter()
                .any(|(t, g)| *t == Tier::Testing && g.contains("test-conformance")),
            "plan must include testing/test-conformance"
        );
        assert!(plan.len() >= 4, "expected >= 4 gates, got {}", plan.len());
    }
}
