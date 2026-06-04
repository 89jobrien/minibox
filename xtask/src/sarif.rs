//! Minimal SARIF 2.1.0 builder for xtask diagnostic output.
//!
//! Produces valid SARIF JSON that GitHub Code Scanning accepts via
//! `github/codeql-action/upload-sarif`. No external crate needed.
//!
//! Reference: <https://docs.oasis-open.org/sarif/sarif/v2.1.0/sarif-v2.1.0.html>

use serde::Serialize;
use std::path::Path;

// ── Top-level log ────────────────────────────────────────────────────────

/// A complete SARIF 2.1.0 log.
#[derive(Debug, Serialize)]
pub struct SarifLog {
    #[serde(rename = "$schema")]
    pub schema: &'static str,
    pub version: &'static str,
    pub runs: Vec<Run>,
}

impl SarifLog {
    /// Create a log with a single run for the given tool.
    pub fn new(tool_name: &str, tool_version: &str) -> Self {
        Self {
            schema: "https://raw.githubusercontent.com/oasis-tcs/sarif-spec/main/sarif-2.1/schema/sarif-schema-2.1.0.json",
            version: "2.1.0",
            runs: vec![Run {
                tool: Tool {
                    driver: ToolComponent {
                        name: tool_name.to_string(),
                        version: Some(tool_version.to_string()),
                        rules: vec![],
                    },
                },
                results: vec![],
            }],
        }
    }

    /// Access the first (and usually only) run mutably.
    pub fn run_mut(&mut self) -> &mut Run {
        &mut self.runs[0]
    }

    /// Write the SARIF log as pretty-printed JSON to `path`.
    pub fn write_to(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        eprintln!("sarif: wrote {}", path.display());
        Ok(())
    }
}

// ── Run ──────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct Run {
    pub tool: Tool,
    pub results: Vec<SarifResult>,
}

impl Run {
    /// Register a rule and return its index (for associating results).
    pub fn add_rule(&mut self, rule: ReportingDescriptor) -> usize {
        let idx = self.tool.driver.rules.len();
        self.tool.driver.rules.push(rule);
        idx
    }

    /// Add a result linked to a rule by index.
    pub fn add_result(&mut self, result: SarifResult) {
        self.results.push(result);
    }
}

// ── Tool ─────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct Tool {
    pub driver: ToolComponent,
}

#[derive(Debug, Serialize)]
pub struct ToolComponent {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<ReportingDescriptor>,
}

// ── Rule (ReportingDescriptor) ───────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct ReportingDescriptor {
    pub id: String,
    #[serde(rename = "shortDescription")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub short_description: Option<Message>,
    #[serde(rename = "fullDescription")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub full_description: Option<Message>,
    #[serde(rename = "helpUri")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub help_uri: Option<String>,
    #[serde(rename = "defaultConfiguration")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_configuration: Option<RuleConfiguration>,
}

impl ReportingDescriptor {
    /// Create a rule with just an ID and short description.
    pub fn new(id: impl Into<String>, short_desc: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            short_description: Some(Message {
                text: short_desc.into(),
            }),
            full_description: None,
            help_uri: None,
            default_configuration: None,
        }
    }

    /// Set the default severity level.
    pub fn with_level(mut self, level: Level) -> Self {
        self.default_configuration = Some(RuleConfiguration { level });
        self
    }
}

#[derive(Debug, Serialize)]
pub struct RuleConfiguration {
    pub level: Level,
}

// ── Result ───────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct SarifResult {
    #[serde(rename = "ruleId")]
    pub rule_id: String,
    #[serde(rename = "ruleIndex")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule_index: Option<usize>,
    pub level: Level,
    pub message: Message,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub locations: Vec<Location>,
}

impl SarifResult {
    /// Create a result for the given rule.
    pub fn new(rule_id: impl Into<String>, level: Level, message: impl Into<String>) -> Self {
        Self {
            rule_id: rule_id.into(),
            rule_index: None,
            level,
            message: Message {
                text: message.into(),
            },
            locations: vec![],
        }
    }

    /// Associate this result with a rule index.
    pub fn with_rule_index(mut self, idx: usize) -> Self {
        self.rule_index = Some(idx);
        self
    }

    /// Add a file location (no line/column).
    pub fn with_file(mut self, uri: impl Into<String>) -> Self {
        self.locations.push(Location {
            physical_location: PhysicalLocation {
                artifact_location: ArtifactLocation { uri: uri.into() },
                region: None,
            },
        });
        self
    }

    /// Add a file location with line number.
    #[allow(dead_code)]
    pub fn with_file_line(mut self, uri: impl Into<String>, line: u32) -> Self {
        self.locations.push(Location {
            physical_location: PhysicalLocation {
                artifact_location: ArtifactLocation { uri: uri.into() },
                region: Some(Region {
                    start_line: line,
                    start_column: None,
                }),
            },
        });
        self
    }
}

// ── Location ─────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct Location {
    #[serde(rename = "physicalLocation")]
    pub physical_location: PhysicalLocation,
}

#[derive(Debug, Serialize)]
pub struct PhysicalLocation {
    #[serde(rename = "artifactLocation")]
    pub artifact_location: ArtifactLocation,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<Region>,
}

#[derive(Debug, Serialize)]
pub struct ArtifactLocation {
    pub uri: String,
}

#[derive(Debug, Serialize)]
pub struct Region {
    #[serde(rename = "startLine")]
    pub start_line: u32,
    #[serde(rename = "startColumn")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_column: Option<u32>,
}

// ── Shared types ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct Message {
    pub text: String,
}

/// SARIF result severity levels.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub enum Level {
    Error,
    Warning,
    Note,
    None,
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_log_serializes() {
        let log = SarifLog::new("test-tool", "0.1.0");
        let json = serde_json::to_string_pretty(&log).unwrap();
        assert!(json.contains("\"version\": \"2.1.0\""));
        assert!(json.contains("\"name\": \"test-tool\""));
    }

    #[test]
    fn log_with_result_roundtrips() {
        let mut log = SarifLog::new("xtask", "0.1.0");
        let run = log.run_mut();

        let rule_idx = run.add_rule(
            ReportingDescriptor::new(
                "minibox/protocol-drift/wire-protocol",
                "Wire protocol hash mismatch",
            )
            .with_level(Level::Error),
        );

        run.add_result(
            SarifResult::new(
                "minibox/protocol-drift/wire-protocol",
                Level::Error,
                "expected abc123, got def456",
            )
            .with_rule_index(rule_idx)
            .with_file("crates/minibox-core/src/protocol.rs"),
        );

        let json = serde_json::to_string_pretty(&log).unwrap();
        assert!(json.contains("protocol-drift"));
        assert!(json.contains("abc123"));
        assert!(json.contains("protocol.rs"));
    }

    #[test]
    fn level_serializes_camel_case() {
        let json = serde_json::to_string(&Level::Error).unwrap();
        assert_eq!(json, "\"error\"");
        let json = serde_json::to_string(&Level::Warning).unwrap();
        assert_eq!(json, "\"warning\"");
    }

    #[test]
    fn write_to_creates_parent_dirs() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("nested/dir/report.sarif");
        let log = SarifLog::new("test", "0.1.0");
        log.write_to(&path).unwrap();
        assert!(path.exists());
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("2.1.0"));
    }
}
