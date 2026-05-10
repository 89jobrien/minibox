//! bump — bump the workspace version in the root Cargo.toml.
//!
//! Usage: cargo xtask bump <patch|minor|major>
//!
//! Minor bumps are rate-limited to once per calendar day. If a minor bump
//! has already occurred today, the request is silently downgraded to patch.
//! The last minor bump date is tracked in `.minibox-bump-state` (gitignored).

use anyhow::{Result, bail};
use std::fs;
use std::path::Path;

pub fn bump(root: &Path, level: &str) -> Result<()> {
    let manifest_path = root.join("Cargo.toml");
    let content = fs::read_to_string(&manifest_path)?;

    let current = parse_workspace_version(&content).ok_or_else(|| {
        anyhow::anyhow!("could not find [workspace.package] version in Cargo.toml")
    })?;

    let effective_level = if level == "minor" && minor_bumped_today(root) {
        eprintln!("[minibox] minor bump already applied today — downgrading to patch");
        "patch"
    } else {
        level
    };

    let (major, minor, patch) = parse_semver(&current)?;
    let next = match effective_level {
        "patch" => format!("{major}.{minor}.{}", patch + 1),
        "minor" => format!("{major}.{}.0", minor + 1),
        "major" => format!("{}.0.0", major + 1),
        other => bail!("unknown bump level: {other} (expected patch, minor, or major)"),
    };

    if effective_level == "minor" {
        record_minor_bump(root);
    }

    // Replace workspace.package version (first occurrence) and all internal
    // crate dependency versions that reference the current version.
    let updated = content.replace(
        &format!("version = \"{current}\""),
        &format!("version = \"{next}\""),
    );

    if updated == content {
        bail!("version string not found in Cargo.toml — nothing changed");
    }

    fs::write(&manifest_path, updated)?;
    println!("[minibox] version bumped {current} → {next}");
    Ok(())
}

fn parse_workspace_version(content: &str) -> Option<String> {
    // Find [workspace.package] section, then the first `version = "..."` line within it.
    let mut in_workspace_package = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == "[workspace.package]" {
            in_workspace_package = true;
            continue;
        }
        if in_workspace_package {
            if trimmed.starts_with('[') {
                break; // left the section
            }
            if let Some(v) = trimmed.strip_prefix("version = \"")
                && let Some(v) = v.strip_suffix('"')
            {
                return Some(v.to_string());
            }
        }
    }
    None
}

fn parse_semver(v: &str) -> Result<(u64, u64, u64)> {
    let parts: Vec<&str> = v.split('.').collect();
    if parts.len() != 3 {
        bail!("version {v:?} is not semver (expected X.Y.Z)");
    }
    Ok((parts[0].parse()?, parts[1].parse()?, parts[2].parse()?))
}

/// State file that records the date of the last minor bump.
const BUMP_STATE_FILE: &str = ".minibox-bump-state";

fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

fn minor_bumped_today(root: &Path) -> bool {
    let path = root.join(BUMP_STATE_FILE);
    fs::read_to_string(path)
        .ok()
        .map(|content| content.trim() == today())
        .unwrap_or(false)
}

fn record_minor_bump(root: &Path) {
    let path = root.join(BUMP_STATE_FILE);
    let _ = fs::write(path, today());
}
