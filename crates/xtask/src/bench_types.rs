/// Typed representations of the minibox bench JSON format.
///
/// These mirror the structs in `minibox-bench/src/main.rs` but add
/// `Deserialize` so xtask can read result files without going through
/// `serde_json::Value`.
use serde::{Deserialize, Serialize};

/// Top-level benchmark report written by minibox-bench.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct BenchReport {
    pub metadata: Metadata,
    pub suites: Vec<SuiteResult>,
    #[serde(default)]
    pub errors: Vec<ErrorCount>,
}

/// Run-level metadata embedded in every [`BenchReport`].
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Metadata {
    pub timestamp: String,
    pub hostname: String,
    pub git_sha: String,
    pub minibox_version: String,
}

/// Results for a single named benchmark suite.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct SuiteResult {
    pub name: String,
    pub tests: Vec<TestResult>,
}

/// Timing data and summary statistics for a single benchmark case.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct TestResult {
    pub name: String,
    pub iterations: usize,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub durations_micros: Vec<u64>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub durations_nanos: Vec<u64>,
    pub stats: Option<Stats>,
    #[serde(skip_serializing_if = "String::is_empty", default)]
    pub unit: String,
}

/// Aggregate statistics for a bench case.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Stats {
    pub min: u64,
    pub avg: u64,
    pub p95: u64,
}

/// Error message with occurrence count.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ErrorCount {
    pub message: String,
    pub count: usize,
}
