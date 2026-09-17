//! bump — bump the workspace version in the root Cargo.toml.
//!
//! Usage: cargo xtask bump <patch|minor|major>
//!
//! Minor bumps are rate-limited to once per calendar day. If a minor bump
//! has already occurred today, the request is silently downgraded to patch.
//! The last minor bump date is tracked in `.minibox-bump-state` (gitignored).

use anyhow::{Context, Result, bail};
use std::fs;
use std::path::Path;

pub fn bump(root: &Path, level: &str) -> Result<String> {
    let manifest_path = root.join("Cargo.toml");
    let content = fs::read_to_string(&manifest_path)?;
    let (current, next, record_minor) = calculate_next(root, &content, level)?;

    // Replace workspace.package version (first occurrence) and all internal
    // crate dependency versions that reference the current version.
    let updated = content.replace(
        &format!("version = \"{current}\""),
        &format!("version = \"{next}\""),
    );

    if updated == content {
        bail!("version string not found in Cargo.toml — nothing changed");
    }

    crate::changelog::atomic_write(&manifest_path, updated.as_bytes())?;
    if record_minor && let Err(error) = record_minor_bump(root) {
        crate::changelog::atomic_write(&manifest_path, content.as_bytes())
            .context("rolling back manifest after rate-limit state failure")?;
        return Err(error);
    }
    println!("[minibox] version bumped {current} → {next}");
    Ok(next)
}

pub fn bump_with_changelog(root: &Path, level: &str) -> Result<Option<String>> {
    let manifest_path = root.join("Cargo.toml");
    let original_manifest = fs::read_to_string(&manifest_path)
        .with_context(|| format!("reading {}", manifest_path.display()))?;
    let (current, next, record_minor) = calculate_next(root, &original_manifest, level)?;
    let Some(changelog) = crate::changelog::prepare(root, &next)? else {
        return Ok(None);
    };
    apply_prepared_changelog(
        root,
        &manifest_path,
        &original_manifest,
        &current,
        &next,
        record_minor,
        &changelog,
    )?;
    println!("[minibox] version bumped {current} → {next}");
    Ok(Some(next))
}

fn apply_prepared_changelog(
    root: &Path,
    manifest_path: &Path,
    original_manifest: &str,
    current: &str,
    next: &str,
    record_minor: bool,
    changelog: &crate::changelog::PreparedChangelog,
) -> Result<()> {
    let updated_manifest = original_manifest.replace(
        &format!("version = \"{current}\""),
        &format!("version = \"{next}\""),
    );
    if updated_manifest == original_manifest {
        bail!("version string not found in Cargo.toml — nothing changed");
    }

    crate::changelog::atomic_write(manifest_path, updated_manifest.as_bytes())?;
    if let Err(error) =
        crate::changelog::atomic_write(&changelog.path, changelog.updated.as_bytes())
    {
        crate::changelog::atomic_write(manifest_path, original_manifest.as_bytes())
            .context("rolling back manifest after changelog failure")?;
        return Err(error);
    }
    if record_minor && let Err(error) = record_minor_bump(root) {
        crate::changelog::atomic_write(manifest_path, original_manifest.as_bytes())
            .context("rolling back manifest after rate-limit state failure")?;
        crate::changelog::atomic_write(&changelog.path, changelog.original.as_bytes())
            .context("rolling back changelog after rate-limit state failure")?;
        return Err(error);
    }
    Ok(())
}

fn calculate_next(root: &Path, content: &str, level: &str) -> Result<(String, String, bool)> {
    let current = parse_workspace_version(content).ok_or_else(|| {
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
    Ok((current, next, effective_level == "minor"))
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
    fs::read_to_string(path).is_ok_and(|content| content.trim() == today())
}

fn record_minor_bump(root: &Path) -> Result<()> {
    let path = root.join(BUMP_STATE_FILE);
    crate::changelog::atomic_write(&path, today().as_bytes())
        .with_context(|| format!("recording minor bump in {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bump_returns_written_version() {
        let temp = tempfile::tempdir().expect("temporary workspace should be created");
        fs::write(
            temp.path().join("Cargo.toml"),
            "[workspace.package]\nversion = \"0.33.0\"\n",
        )
        .expect("fixture manifest should be written");

        let next = bump(temp.path(), "patch").expect("patch bump should succeed");

        assert_eq!(next, "0.33.1");
        let manifest = fs::read_to_string(temp.path().join("Cargo.toml"))
            .expect("bumped manifest should be readable");
        assert!(manifest.contains("version = \"0.33.1\""));
    }

    #[test]
    fn failed_minor_bump_does_not_consume_rate_limit() {
        let temp = tempfile::tempdir().expect("temporary workspace should be created");
        fs::write(
            temp.path().join("Cargo.toml"),
            "[workspace.package]\nversion = \"0.33.0\"\n",
        )
        .expect("fixture manifest should be written");
        fs::create_dir(temp.path().join(BUMP_STATE_FILE))
            .expect("state path should block file creation");

        assert!(bump(temp.path(), "minor").is_err());
        assert_eq!(
            fs::read_to_string(temp.path().join("Cargo.toml"))
                .expect("manifest should remain readable"),
            "[workspace.package]\nversion = \"0.33.0\"\n"
        );
    }

    #[test]
    fn calculates_major_minor_and_rate_limited_minor_versions() {
        let temp = tempfile::tempdir().expect("temporary workspace should be created");
        let manifest = "[workspace.package]\nversion = \"1.2.3\"\n";
        assert_eq!(
            calculate_next(temp.path(), manifest, "major").unwrap().1,
            "2.0.0"
        );
        assert_eq!(
            calculate_next(temp.path(), manifest, "minor").unwrap().1,
            "1.3.0"
        );
        fs::write(temp.path().join(BUMP_STATE_FILE), today())
            .expect("rate-limit fixture should be written");
        assert_eq!(
            calculate_next(temp.path(), manifest, "minor").unwrap().1,
            "1.2.4"
        );
        assert!(calculate_next(temp.path(), manifest, "invalid").is_err());
        assert!(calculate_next(temp.path(), "[workspace]\n", "patch").is_err());
        assert!(
            calculate_next(
                temp.path(),
                "[workspace.package]\nversion = \"bad\"\n",
                "patch"
            )
            .is_err()
        );
    }

    #[test]
    fn version_and_changelog_roll_back_together() {
        let temp = tempfile::tempdir().expect("temporary workspace should be created");
        let manifest_path = temp.path().join("Cargo.toml");
        let changelog_path = temp.path().join("CHANGELOG.md");
        let manifest = "[workspace.package]\nversion = \"1.2.3\"\n";
        let changelog = "# Changelog\n";
        fs::write(&manifest_path, manifest).expect("manifest should be written");
        fs::write(&changelog_path, changelog).expect("changelog should be written");
        fs::create_dir(temp.path().join(BUMP_STATE_FILE))
            .expect("rate-limit path should block persistence");
        let prepared = crate::changelog::PreparedChangelog {
            path: changelog_path.clone(),
            original: changelog.to_string(),
            updated: "# Changelog\n\n## 1.3.0\n".to_string(),
        };

        assert!(
            apply_prepared_changelog(
                temp.path(),
                &manifest_path,
                manifest,
                "1.2.3",
                "1.3.0",
                true,
                &prepared,
            )
            .is_err()
        );
        assert_eq!(fs::read_to_string(manifest_path).unwrap(), manifest);
        assert_eq!(fs::read_to_string(changelog_path).unwrap(), changelog);
    }
}
