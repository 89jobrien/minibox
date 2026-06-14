//! Preflight checks for the smolvm binary.
//!
//! Provides detection of the `smolvm` binary on PATH and version parsing.

use anyhow::{Context, Result};
use std::process::Command;

/// Result of a smolvm preflight check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmolvmStatus {
    /// Whether the `smolvm` binary was found on PATH.
    pub found: bool,
    /// The version string reported by `smolvm --version`, if available.
    pub version: Option<String>,
    /// The resolved path to the binary, if found.
    pub path: Option<std::path::PathBuf>,
}

/// Check whether `smolvm` is available on PATH and query its version.
///
/// Returns a [`SmolvmStatus`] describing what was found. Never errors --
/// a missing binary is reported as `found: false`.
#[must_use]
pub fn check_smolvm() -> SmolvmStatus {
    let bin = match which::which("smolvm") {
        Ok(p) => p,
        Err(_) => {
            return SmolvmStatus {
                found: false,
                version: None,
                path: None,
            };
        }
    };

    let version = query_version(&bin).ok();

    SmolvmStatus {
        found: true,
        version,
        path: Some(bin),
    }
}

/// Parse a version string from raw `smolvm --version` output.
///
/// Handles formats: `"smolvm 0.5.2"`, `"smolvm version 0.5.2"`, `"0.5.2"`.
#[must_use]
pub fn parse_version_output(raw: &str) -> String {
    let trimmed = raw.trim();
    trimmed
        .strip_prefix("smolvm version ")
        .or_else(|| trimmed.strip_prefix("smolvm "))
        .unwrap_or(trimmed)
        .to_string()
}

/// Run `smolvm --version` and parse the version string from stdout.
fn query_version(bin: &std::path::Path) -> Result<String> {
    let output = Command::new(bin)
        .arg("--version")
        .output()
        .with_context(|| format!("failed to run {} --version", bin.display()))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(parse_version_output(&stdout))
}

/// Check whether `smolvm` is on PATH (quick boolean probe).
///
/// Equivalent to `check_smolvm().found` but avoids the version query.
#[must_use]
pub fn smolvm_available() -> bool {
    which::which("smolvm").is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_smolvm_returns_status() {
        let status = check_smolvm();
        // We cannot guarantee smolvm is installed in CI, but the function
        // must not panic regardless.
        if status.found {
            assert!(status.path.is_some());
        } else {
            assert!(status.version.is_none());
            assert!(status.path.is_none());
        }
    }

    #[test]
    fn smolvm_available_does_not_panic() {
        // Just exercise the function -- result depends on environment.
        let _ = smolvm_available();
    }

    #[test]
    fn status_not_found_has_none_fields() {
        let status = SmolvmStatus {
            found: false,
            version: None,
            path: None,
        };
        assert!(!status.found);
        assert!(status.version.is_none());
        assert!(status.path.is_none());
    }

    #[test]
    fn status_equality() {
        let a = SmolvmStatus {
            found: true,
            version: Some("0.5.2".to_string()),
            path: Some("/usr/local/bin/smolvm".into()),
        };
        let b = a.clone();
        assert_eq!(a, b);
    }

    #[test]
    fn parse_version_output_standard() {
        assert_eq!(parse_version_output("smolvm 0.5.2"), "0.5.2");
    }

    #[test]
    fn parse_version_output_with_version_prefix() {
        assert_eq!(parse_version_output("smolvm version 0.5.2"), "0.5.2");
    }

    #[test]
    fn parse_version_output_bare_version() {
        assert_eq!(parse_version_output("0.5.2"), "0.5.2");
    }

    #[test]
    fn parse_version_output_with_trailing_newline() {
        assert_eq!(parse_version_output("smolvm 0.5.2\n"), "0.5.2");
    }
}
