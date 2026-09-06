//! Changelog generation for explicit workspace version bumps.

use anyhow::{Context, Result, bail};
use std::fs;
use std::path::Path;
use xshell::{Shell, cmd};

const UNRELEASED_HEADING: &str = "## [Unreleased]";

pub fn prepare(root: &Path, version: &str) -> Result<Option<String>> {
    validate_version(version)?;

    let sh = Shell::new().context("creating shell for git-cliff")?;
    sh.change_dir(root);
    let config = root.join("cliff.toml");
    let version_tag = format!("v{version}");
    let fragment = cmd!(
        sh,
        "git cliff --config {config} --unreleased --tag {version_tag} --strip all --no-exec --offline --output /dev/stdout"
    )
    .read()
    .context("rendering release notes with git-cliff; install git-cliff and retry")?;
    let changelog_path = root.join("CHANGELOG.md");
    let existing = fs::read_to_string(&changelog_path)
        .with_context(|| format!("reading {}", changelog_path.display()))?;
    prepare_update(&existing, version, &fragment)
}

pub fn write(root: &Path, contents: &str) -> Result<()> {
    let changelog_path = root.join("CHANGELOG.md");
    fs::write(&changelog_path, contents)
        .with_context(|| format!("writing {}", changelog_path.display()))?;
    Ok(())
}

fn insert_release(existing: &str, version: &str, fragment: &str) -> Result<String> {
    validate_version(version)?;
    reject_duplicate_release(existing, version)?;

    let marker = format!("{UNRELEASED_HEADING}\n");
    let marker_start = existing.find(&marker).ok_or_else(|| {
        anyhow::anyhow!("CHANGELOG.md is missing the `{UNRELEASED_HEADING}` heading")
    })?;
    let body_start = marker_start + marker.len();
    let remainder = &existing[body_start..];
    let next_heading = remainder
        .find("\n## ")
        .map_or(existing.len(), |offset| body_start + offset + 1);

    if !existing[body_start..next_heading].trim().is_empty() {
        bail!("CHANGELOG.md has manual entries in the Unreleased section");
    }

    let fragment = fragment.trim();
    if fragment.is_empty() {
        bail!("git-cliff rendered an empty release fragment for version {version}");
    }
    if !fragment_has_entries(fragment) {
        bail!("git-cliff rendered no changelog entries for version {version}");
    }

    let prefix = &existing[..body_start];
    let suffix = &existing[next_heading..];
    if suffix.is_empty() {
        Ok(format!("{prefix}\n{fragment}\n"))
    } else {
        Ok(format!("{prefix}\n{fragment}\n\n{suffix}"))
    }
}

fn prepare_update(existing: &str, version: &str, fragment: &str) -> Result<Option<String>> {
    if !fragment_has_entries(fragment) {
        return Ok(None);
    }
    insert_release(existing, version, fragment).map(Some)
}

fn fragment_has_entries(fragment: &str) -> bool {
    fragment
        .lines()
        .any(|line| line.trim_start().starts_with("- "))
}

fn validate_version(version: &str) -> Result<()> {
    let mut parts = version.split('.');
    let valid = (0..3).all(|_| {
        parts
            .next()
            .is_some_and(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
    }) && parts.next().is_none();

    if !valid {
        bail!("invalid release version `{version}`; expected MAJOR.MINOR.PATCH");
    }
    Ok(())
}

fn reject_duplicate_release(existing: &str, version: &str) -> Result<()> {
    let bare = format!("## {version}");
    let bracketed = format!("## [v{version}]");
    if existing.lines().any(|line| {
        line == bare
            || line.starts_with(&format!("{bare} - "))
            || line == bracketed
            || line.starts_with(&format!("{bracketed} - "))
    }) {
        bail!("CHANGELOG.md already contains release {version}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXISTING: &str =
        "# Changelog\n\n## [Unreleased]\n\n## [v0.33.0] - 2026-08-26\n\nOld notes.\n";
    const FRAGMENT: &str = "## 0.34.0 - 2026-09-06\n\n### Features\n\n- Add release automation\n";

    #[test]
    fn inserts_release_after_unreleased_heading() {
        let updated = insert_release(EXISTING, "0.34.0", FRAGMENT)
            .expect("empty Unreleased section should accept a release");

        assert_eq!(
            updated,
            "# Changelog\n\n## [Unreleased]\n\n## 0.34.0 - 2026-09-06\n\n### Features\n\n- Add release automation\n\n## [v0.33.0] - 2026-08-26\n\nOld notes.\n"
        );
    }

    #[test]
    fn preserves_historical_content_byte_for_byte() {
        let history = "## [v0.33.0] - 2026-08-26\n\n- Existing detail.  \n";
        let existing = format!("# Changelog\n\n## [Unreleased]\n\n{history}");

        let updated = insert_release(&existing, "0.34.0", FRAGMENT)
            .expect("existing history should be retained");

        assert!(updated.ends_with(history));
    }

    #[test]
    fn rejects_nonempty_unreleased_section() {
        let existing =
            "# Changelog\n\n## [Unreleased]\n\n- Manual note\n\n## [v0.33.0] - 2026-08-26\n";

        let error = insert_release(existing, "0.34.0", FRAGMENT)
            .expect_err("manual Unreleased notes must be resolved first");

        assert!(error.to_string().contains("manual entries"));
    }

    #[test]
    fn rejects_duplicate_release_headings() {
        for heading in ["## 0.34.0 - 2026-09-06", "## [v0.34.0] - 2026-09-06"] {
            let existing = format!("{EXISTING}\n{heading}\n");
            let error = insert_release(&existing, "0.34.0", FRAGMENT)
                .expect_err("duplicate releases must be rejected");
            assert!(error.to_string().contains("already contains release"));
        }
    }

    #[test]
    fn rejects_missing_unreleased_heading() {
        let error = insert_release("# Changelog\n", "0.34.0", FRAGMENT)
            .expect_err("the Unreleased marker is required");

        assert!(error.to_string().contains("missing the `## [Unreleased]`"));
    }

    #[test]
    fn rejects_non_semver_release_version() {
        for version in ["v0.34.0", "0.34", "0.34.beta", "0.34.0.1"] {
            let error = insert_release(EXISTING, version, FRAGMENT)
                .expect_err("invalid versions must be rejected");
            assert!(error.to_string().contains("expected MAJOR.MINOR.PATCH"));
        }
    }

    #[test]
    fn rejects_heading_only_release_fragment() {
        let error = insert_release(EXISTING, "0.34.0", "## 0.34.0 - 2026-09-06\n")
            .expect_err("a release without entries must be rejected");

        assert!(error.to_string().contains("no changelog entries"));
    }

    #[test]
    fn heading_only_release_is_a_safe_noop() {
        let updated = prepare_update(EXISTING, "0.34.0", "## 0.34.0 - 2026-09-06\n")
            .expect("an empty release should not fail");

        assert_eq!(updated, None);
    }
}
