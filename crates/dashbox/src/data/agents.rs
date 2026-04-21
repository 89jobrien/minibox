// dashbox/src/data/agents.rs
//
// Hexagonal architecture:
//   Port:     AutomationLogPort  — what the Agents tab depends on
//   Adapters: JsonlFileSource    — reads a single JSONL file
//             MultiSourceLog     — merges N sources, deduplicates by run_id
//
// Domain types (AgentRun, AgentsData) have zero infrastructure dependencies.
use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// Domain types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct AgentRun {
    pub run_id: String,
    pub script: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub duration_s: Option<f64>,
    #[serde(default)]
    pub output: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AgentsData {
    pub runs: Vec<AgentRun>,
    pub total: usize,
    pub complete: usize,
    pub running: usize,
    pub crashed: usize,
}

impl AgentsData {
    /// Build summary counts from a deduplicated, sorted run list.
    fn from_runs(runs: Vec<AgentRun>) -> Self {
        let total = runs.len();
        let complete = runs.iter().filter(|r| r.status == "complete").count();
        let running = runs.iter().filter(|r| r.status == "running").count();
        let crashed = runs.iter().filter(|r| r.status == "crashed").count();
        Self {
            runs,
            total,
            complete,
            running,
            crashed,
        }
    }
}

// ---------------------------------------------------------------------------
// Port — what tabs depend on (not a concrete file path)
// ---------------------------------------------------------------------------

pub trait AutomationLogPort {
    /// Load all automation runs, deduplicated and sorted newest-first.
    fn load_runs(&self) -> Result<AgentsData>;
}

// ---------------------------------------------------------------------------
// Adapter: JsonlFileSource — reads one JSONL log file
// ---------------------------------------------------------------------------

pub struct JsonlFileSource {
    path: PathBuf,
}

impl JsonlFileSource {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Default path: ~/.minibox/agent-runs.jsonl
    pub fn default_agent_log() -> Self {
        let path = dirs::home_dir()
            .expect("cannot determine home directory")
            .join(".minibox/agent-runs.jsonl");
        Self { path }
    }
}

impl AutomationLogPort for JsonlFileSource {
    fn load_runs(&self) -> Result<AgentsData> {
        let content = std::fs::read_to_string(&self.path)
            .with_context(|| format!("read {}", self.path.display()))?;
        let runs = parse_and_deduplicate(content.lines());
        Ok(AgentsData::from_runs(runs))
    }
}

// ---------------------------------------------------------------------------
// Adapter: MultiSourceLog — merges N sources, deduplicates across all
// ---------------------------------------------------------------------------

pub struct MultiSourceLog {
    sources: Vec<Box<dyn AutomationLogPort>>,
}

impl MultiSourceLog {
    pub fn new(sources: Vec<Box<dyn AutomationLogPort>>) -> Self {
        Self { sources }
    }
}

impl AutomationLogPort for MultiSourceLog {
    fn load_runs(&self) -> Result<AgentsData> {
        let mut by_id: HashMap<String, AgentRun> = HashMap::new();

        for source in &self.sources {
            // Ignore sources that fail to load (e.g. file not yet created)
            let Ok(data) = source.load_runs() else {
                continue;
            };
            for run in data.runs {
                let id = run.run_id.clone();
                let existing = by_id.get(&id);
                // complete/crashed beat running for the same run_id
                if existing.is_none() || run.status == "complete" || run.status == "crashed" {
                    by_id.insert(id, run);
                }
            }
        }

        let mut runs: Vec<AgentRun> = by_id.into_values().collect();
        runs.sort_by(|a, b| b.run_id.cmp(&a.run_id));
        Ok(AgentsData::from_runs(runs))
    }
}

// ---------------------------------------------------------------------------
// Parsing helper (shared between adapters)
// ---------------------------------------------------------------------------

fn parse_and_deduplicate<'a>(lines: impl Iterator<Item = &'a str>) -> Vec<AgentRun> {
    let mut by_id: HashMap<String, AgentRun> = HashMap::new();
    for line in lines {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(run) = serde_json::from_str::<AgentRun>(line) else {
            continue;
        };
        let id = run.run_id.clone();
        let existing = by_id.get(&id);
        if existing.is_none() || run.status == "complete" || run.status == "crashed" {
            by_id.insert(id, run);
        }
    }
    let mut v: Vec<_> = by_id.into_values().collect();
    v.sort_by(|a, b| b.run_id.cmp(&a.run_id));
    v
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn run(id: &str, status: &str) -> AgentRun {
        AgentRun {
            run_id: id.to_string(),
            script: "test".to_string(),
            status: status.to_string(),
            duration_s: None,
            output: None,
        }
    }

    #[test]
    fn complete_beats_running_same_id() {
        let lines = [
            r#"{"run_id":"2026-01-01T00:00:00","script":"s","status":"running"}"#,
            r#"{"run_id":"2026-01-01T00:00:00","script":"s","status":"complete"}"#,
        ];
        let runs = parse_and_deduplicate(lines.iter().copied());
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].status, "complete");
    }

    #[test]
    fn multi_source_merges_and_deduplicates() {
        struct FakeSource(Vec<AgentRun>);
        impl AutomationLogPort for FakeSource {
            fn load_runs(&self) -> Result<AgentsData> {
                Ok(AgentsData::from_runs(self.0.clone()))
            }
        }

        let s1: Box<dyn AutomationLogPort> = Box::new(FakeSource(vec![
            run("2026-01-01T00:00:01", "complete"),
            run("2026-01-01T00:00:02", "running"),
        ]));
        let s2: Box<dyn AutomationLogPort> = Box::new(FakeSource(vec![
            run("2026-01-01T00:00:02", "complete"), // should win over running
            run("2026-01-01T00:00:03", "crashed"),
        ]));

        let merged = MultiSourceLog::new(vec![s1, s2]);
        let data = merged.load_runs().unwrap();

        assert_eq!(data.total, 3);
        assert_eq!(data.complete, 2);
        assert_eq!(data.crashed, 1);
        let id2 = data
            .runs
            .iter()
            .find(|r| r.run_id.contains("00:02"))
            .unwrap();
        assert_eq!(id2.status, "complete");
    }

    #[test]
    fn agents_data_counts_correct() {
        let runs = vec![
            run("a", "complete"),
            run("b", "complete"),
            run("c", "running"),
            run("d", "crashed"),
        ];
        let data = AgentsData::from_runs(runs);
        assert_eq!(data.total, 4);
        assert_eq!(data.complete, 2);
        assert_eq!(data.running, 1);
        assert_eq!(data.crashed, 1);
    }
}
