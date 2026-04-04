// dashbox/src/data/ci.rs
use anyhow::{Context, Result};
use serde::Deserialize;
use std::process::Command;

use super::DataSource;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CiRun {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub head_branch: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub conclusion: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    #[allow(dead_code)]
    pub database_id: u64,
    #[serde(default)]
    pub url: String,
}

#[derive(Debug, Clone)]
pub struct CiData {
    pub runs: Vec<CiRun>,
    pub success_rate: f64,
}

pub struct CiSource;

impl CiSource {
    pub fn new() -> Self {
        Self
    }
}

impl DataSource for CiSource {
    type Data = CiData;

    fn load(&self) -> Result<CiData> {
        let output = Command::new("gh")
            .args([
                "run",
                "list",
                "--json",
                "conclusion,status,headBranch,createdAt,name,databaseId,url",
                "--limit",
                "15",
            ])
            .output()
            .context("failed to run gh")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("gh exited with {}: {}", output.status, stderr.trim());
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let runs: Vec<CiRun> =
            serde_json::from_str(&stdout).context("failed to parse gh output")?;

        let completed: Vec<&CiRun> = runs.iter().filter(|r| r.status == "completed").collect();
        let success_count = completed
            .iter()
            .filter(|r| r.conclusion == "success")
            .count();
        let success_rate = if completed.is_empty() {
            0.0
        } else {
            success_count as f64 / completed.len() as f64 * 100.0
        };

        Ok(CiData { runs, success_rate })
    }
}
