//! Convert `cargo clippy --message-format=json` output to SARIF.
//!
//! Runs clippy, captures JSON diagnostics, and writes SARIF via the generic
//! [`crate::sarif::from_diagnostics`] converter.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::Path;

use crate::sarif::{self, Diagnostic, Level};

/// Workspace crates to lint (matches the `gates::lint` list).
const CLIPPY_PACKAGES: &[&str] = &[
    "minibox",
    "minibox-macros",
    "mbx",
    "minibox-core",
    "macbox",
    "miniboxd",
    "winbox",
];

/// Run clippy with JSON output and write SARIF to `sarif_path`.
pub fn run(sarif_path: &Path) -> Result<()> {
    let diagnostics = run_clippy_json()?;
    let log = sarif::from_diagnostics("cargo-clippy", env!("CARGO_PKG_VERSION"), &diagnostics);
    log.write_to(sarif_path)
}

/// Convert raw `cargo clippy --message-format=json` output (one JSON object
/// per line) into [`Diagnostic`] items.
pub fn parse_clippy_json(output: &str) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    for line in output.lines() {
        let Ok(msg) = serde_json::from_str::<CargoMessage>(line) else {
            continue;
        };
        let Some(diag) = msg.message else {
            continue;
        };
        // Skip non-lint messages (notes, help, raw compiler output).
        if !matches!(diag.level.as_str(), "warning" | "error") {
            continue;
        }
        // Skip the summary line ("N warnings emitted").
        if diag.code.is_none() {
            continue;
        }

        let level = match diag.level.as_str() {
            "error" => Level::Error,
            _ => Level::Warning,
        };

        let rule_id = diag
            .code
            .as_ref()
            .map(|c| c.code.clone())
            .unwrap_or_else(|| "unknown".into());

        let (file, line_num, col) = diag
            .spans
            .iter()
            .find(|s| s.is_primary)
            .map(|s| {
                (
                    Some(s.file_name.clone()),
                    Some(s.line_start),
                    Some(s.column_start),
                )
            })
            .unwrap_or((None, None, None));

        diagnostics.push(Diagnostic {
            rule_id,
            level,
            message: diag.message,
            file,
            line: line_num,
            column: col,
        });
    }

    diagnostics
}

fn run_clippy_json() -> Result<Vec<Diagnostic>> {
    let mut cmd = std::process::Command::new("cargo");
    cmd.arg("clippy");
    for pkg in CLIPPY_PACKAGES {
        cmd.args(["-p", pkg]);
    }
    cmd.args(["--message-format=json", "--", "-D", "warnings"]);
    cmd.stderr(std::process::Stdio::inherit());

    let output = cmd.output().context("failed to run cargo clippy")?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(parse_clippy_json(&stdout))
}

// ── Cargo JSON message types (subset) ───────────────────────────────────

#[derive(Deserialize)]
struct CargoMessage {
    message: Option<ClippyDiagnostic>,
}

#[derive(Deserialize)]
struct ClippyDiagnostic {
    level: String,
    message: String,
    code: Option<DiagnosticCode>,
    #[serde(default)]
    spans: Vec<DiagnosticSpan>,
}

#[derive(Deserialize)]
struct DiagnosticCode {
    code: String,
}

#[derive(Deserialize)]
struct DiagnosticSpan {
    file_name: String,
    line_start: u32,
    column_start: u32,
    is_primary: bool,
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_CLIPPY_LINE: &str = r#"{"reason":"compiler-message","package_id":"minibox 0.30.0","manifest_path":"/dev/minibox/Cargo.toml","message":{"$message_type":"diagnostic","message":"unused variable: `x`","code":{"code":"unused_variables","explanation":null},"level":"warning","spans":[{"file_name":"src/lib.rs","byte_start":100,"byte_end":101,"line_start":10,"line_end":10,"column_start":9,"column_end":10,"is_primary":true,"text":[],"label":null,"suggested_replacement":null,"suggestion_applicability":null,"expansion":null}],"children":[],"rendered":"warning: unused variable"}}"#;

    #[test]
    fn parses_clippy_warning() {
        let items = parse_clippy_json(SAMPLE_CLIPPY_LINE);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].rule_id, "unused_variables");
        assert_eq!(items[0].file, Some("src/lib.rs".into()));
        assert_eq!(items[0].line, Some(10));
        assert_eq!(items[0].column, Some(9));
    }

    #[test]
    fn skips_non_diagnostic_lines() {
        let mixed = format!(
            "{}\n{}\n{}",
            r#"{"reason":"build-script-executed","package_id":"foo 1.0.0"}"#,
            SAMPLE_CLIPPY_LINE,
            r#"{"reason":"compiler-artifact","package_id":"bar 1.0.0"}"#,
        );
        let items = parse_clippy_json(&mixed);
        assert_eq!(items.len(), 1);
    }

    #[test]
    fn skips_summary_line() {
        let summary = r#"{"reason":"compiler-message","package_id":"x 0.1.0","manifest_path":"x","message":{"$message_type":"diagnostic","message":"3 warnings emitted","code":null,"level":"warning","spans":[],"children":[],"rendered":""}}"#;
        let items = parse_clippy_json(summary);
        assert!(items.is_empty());
    }

    #[test]
    fn converts_to_sarif() {
        let items = parse_clippy_json(SAMPLE_CLIPPY_LINE);
        let log = sarif::from_diagnostics("clippy", "0.1.0", &items);
        let json = serde_json::to_string_pretty(&log).expect("serialize");
        assert!(json.contains("unused_variables"));
        assert!(json.contains("src/lib.rs"));
    }
}
