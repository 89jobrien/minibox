use std::path::{Path, PathBuf};

/// Outcome of a single conformance test case.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConformanceOutcome {
    /// Test ran and passed.
    Pass,
    /// Test was skipped (capability not declared by backend).
    Skip,
    /// Test ran and failed.
    Fail,
}

impl ConformanceOutcome {
    /// Display string used in Markdown tables.
    pub fn as_str(&self) -> &'static str {
        match self {
            ConformanceOutcome::Pass => "pass",
            ConformanceOutcome::Skip => "skip",
            ConformanceOutcome::Fail => "fail",
        }
    }
}

/// One row in the conformance matrix.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ConformanceRow {
    /// Backend name (matches [`BackendDescriptor::name`]).
    pub backend: String,
    /// Capability under test (e.g. `"Commit"`, `"BuildFromContext"`, `"PushToRegistry"`).
    pub capability: String,
    /// Name of the individual test case within that capability group.
    pub test_name: String,
    /// Outcome.
    pub outcome: ConformanceOutcome,
    /// Optional human-readable message (failure reason, skip reason, etc.).
    pub message: Option<String>,
}

/// Aggregated result of the full conformance matrix run.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ConformanceMatrixResult {
    /// ISO-8601 timestamp of when the suite ran.
    pub timestamp: String,
    /// Individual test rows.
    pub rows: Vec<ConformanceRow>,
}

impl ConformanceMatrixResult {
    /// Create a result with the current UTC timestamp.
    pub fn new(rows: Vec<ConformanceRow>) -> Self {
        // Use a simple hand-rolled timestamp to avoid pulling in `chrono`.
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        Self {
            timestamp: format!("{ts}"),
            rows,
        }
    }

    /// Count rows with a given outcome.
    pub fn count(&self, outcome: &ConformanceOutcome) -> usize {
        self.rows.iter().filter(|r| &r.outcome == outcome).count()
    }
}

/// Write `report.md` and `report.json` under `artifact_dir`.
///
/// `artifact_dir` is created if it does not exist.
///
/// # Errors
///
/// Returns an error if the directory cannot be created or if either file
/// cannot be written.
pub fn write_conformance_reports(
    result: &ConformanceMatrixResult,
    artifact_dir: &Path,
) -> std::io::Result<(PathBuf, PathBuf)> {
    std::fs::create_dir_all(artifact_dir)?;

    // --- JSON report ---
    let json_path = artifact_dir.join("report.json");
    let json = serde_json::to_string_pretty(result)
        .map_err(std::io::Error::other)?;
    std::fs::write(&json_path, json.as_bytes())?;

    // --- Markdown report ---
    let md_path = artifact_dir.join("report.md");
    let mut md = String::new();
    md.push_str("# Conformance Suite Report\n\n");
    md.push_str(&format!("**Timestamp:** {}\n\n", result.timestamp));
    md.push_str(&format!(
        "**Pass:** {}  **Skip:** {}  **Fail:** {}\n\n",
        result.count(&ConformanceOutcome::Pass),
        result.count(&ConformanceOutcome::Skip),
        result.count(&ConformanceOutcome::Fail),
    ));
    md.push_str("| Backend | Capability | Test | Outcome | Message |\n");
    md.push_str("|---------|------------|------|---------|--------|\n");
    for row in &result.rows {
        let msg = row.message.as_deref().unwrap_or("");
        md.push_str(&format!(
            "| {} | {} | {} | {} | {} |\n",
            row.backend,
            row.capability,
            row.test_name,
            row.outcome.as_str(),
            msg
        ));
    }
    std::fs::write(&md_path, md.as_bytes())?;

    Ok((md_path, json_path))
}
