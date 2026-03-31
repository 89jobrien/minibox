// dashbox/src/data/agents.rs
use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;

use super::DataSource;

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

pub struct AgentsSource {
    path: PathBuf,
}

impl AgentsSource {
    pub fn new() -> Self {
        let path = dirs::home_dir()
            .unwrap_or_default()
            .join(".mbx/agent-runs.jsonl");
        Self { path }
    }
}

impl DataSource for AgentsSource {
    type Data = AgentsData;

    fn load(&self) -> Result<AgentsData> {
        let content = std::fs::read_to_string(&self.path)
            .with_context(|| format!("read {}", self.path.display()))?;

        // Deduplicate by run_id (complete wins over running)
        let mut by_id: HashMap<String, AgentRun> = HashMap::new();
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let run: AgentRun = match serde_json::from_str(line) {
                Ok(r) => r,
                Err(_) => continue,
            };
            let id = run.run_id.clone();
            let existing = by_id.get(&id);
            if existing.is_none() || run.status == "complete" {
                by_id.insert(id, run);
            }
        }

        let runs: Vec<AgentRun> = {
            let mut v: Vec<_> = by_id.into_values().collect();
            v.sort_by(|a, b| b.run_id.cmp(&a.run_id));
            v
        };

        let total = runs.len();
        let complete = runs.iter().filter(|r| r.status == "complete").count();
        let running = runs.iter().filter(|r| r.status == "running").count();
        let crashed = runs.iter().filter(|r| r.status == "crashed").count();

        Ok(AgentsData {
            runs,
            total,
            complete,
            running,
            crashed,
        })
    }
}
