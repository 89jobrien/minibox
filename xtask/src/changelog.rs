//! Changelog generation for explicit workspace version bumps.

use anyhow::{Context, Result, bail};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use xshell::{Shell, cmd};

const UNRELEASED_HEADING: &str = "## [Unreleased]";

#[derive(Debug)]
pub struct PreparedChangelog {
    pub path: PathBuf,
    pub original: String,
    pub updated: String,
}

trait ChangelogSource {
    fn render(&self, root: &Path, version: &str) -> Result<String>;
    fn read(&self, path: &Path) -> Result<String>;
}

struct SystemChangelogSource;

impl ChangelogSource for SystemChangelogSource {
    fn render(&self, root: &Path, version: &str) -> Result<String> {
        let sh = Shell::new().context("creating shell for git-cliff")?;
        sh.change_dir(root);
        let config = root.join("cliff.toml");
        let version_tag = format!("v{version}");
        cmd!(
            sh,
            "git cliff --config {config} --unreleased --tag {version_tag} --strip all --no-exec --offline --output /dev/stdout"
        )
        .read()
        .context("rendering release notes with git-cliff; install git-cliff and retry")
    }

    fn read(&self, path: &Path) -> Result<String> {
        fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))
    }
}

/// Moves unreleased changelog entries into a versioned release section.
pub fn prepare(root: &Path, version: &str) -> Result<Option<PreparedChangelog>> {
    prepare_with(&SystemChangelogSource, root, version)
}

fn prepare_with(
    source: &impl ChangelogSource,
    root: &Path,
    version: &str,
) -> Result<Option<PreparedChangelog>> {
    validate_version(version)?;
    let changelog_path = root.join("CHANGELOG.md");
    let fragment = source.render(root, version)?;
    let existing = source.read(&changelog_path)?;
    Ok(
        prepare_update(&existing, version, &fragment)?.map(|updated| PreparedChangelog {
            path: changelog_path,
            original: existing,
            updated,
        }),
    )
}

/// Replaces a file atomically through a temporary sibling file.
pub fn atomic_write(path: &Path, contents: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .with_context(|| format!("output path has no parent: {}", path.display()))?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .with_context(|| format!("creating temporary file beside {}", path.display()))?;
    temporary
        .write_all(contents)
        .and_then(|()| temporary.flush())
        .with_context(|| format!("writing temporary file for {}", path.display()))?;
    temporary
        .persist(path)
        .map_err(|error| error.error)
        .with_context(|| format!("replacing {}", path.display()))?;
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

    struct FakeSource {
        render: Result<String, &'static str>,
        read: Result<String, &'static str>,
    }

    impl ChangelogSource for FakeSource {
        fn render(&self, _root: &Path, _version: &str) -> Result<String> {
            self.render.clone().map_err(anyhow::Error::msg)
        }

        fn read(&self, _path: &Path) -> Result<String> {
            self.read.clone().map_err(anyhow::Error::msg)
        }
    }

    #[test]
    fn prepare_reports_command_and_io_errors() {
        let root = Path::new("/workspace");
        let command_error = prepare_with(
            &FakeSource {
                render: Err("git-cliff failed"),
                read: Ok(EXISTING.to_string()),
            },
            root,
            "0.34.0",
        )
        .expect_err("render errors must propagate");
        assert!(command_error.to_string().contains("git-cliff failed"));

        let io_error = prepare_with(
            &FakeSource {
                render: Ok(FRAGMENT.to_string()),
                read: Err("read failed"),
            },
            root,
            "0.34.0",
        )
        .expect_err("read errors must propagate");
        assert!(io_error.to_string().contains("read failed"));
    }
}
