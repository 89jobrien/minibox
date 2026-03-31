// dashbox/src/data/bench.rs
use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::PathBuf;

use super::DataSource;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct BenchMeta {
    #[serde(default)]
    pub git_sha: String,
    #[serde(default)]
    pub hostname: String,
    #[serde(default)]
    pub timestamp: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BenchTestStats {
    #[serde(default)]
    pub avg: Option<f64>,
    #[serde(default)]
    pub p95: Option<f64>,
    #[serde(default)]
    pub min: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BenchTest {
    pub name: String,
    #[serde(default)]
    pub iterations: u32,
    #[serde(default)]
    pub durations_micros: Vec<u64>,
    #[serde(default)]
    pub stats: Option<BenchTestStats>,
}

impl BenchTest {
    pub fn avg_us(&self) -> Option<f64> {
        self.stats.as_ref().and_then(|s| s.avg).or_else(|| {
            if self.durations_micros.is_empty() {
                None
            } else {
                Some(
                    self.durations_micros.iter().sum::<u64>() as f64
                        / self.durations_micros.len() as f64,
                )
            }
        })
    }

    pub fn p95_us(&self) -> Option<f64> {
        self.stats.as_ref().and_then(|s| s.p95).or_else(|| {
            if self.durations_micros.len() < 2 {
                return self.avg_us();
            }
            let mut sorted: Vec<u64> = self.durations_micros.clone();
            sorted.sort();
            let idx = (sorted.len() as f64 * 0.95) as usize;
            Some(sorted[idx.min(sorted.len() - 1)] as f64)
        })
    }

    pub fn min_us(&self) -> Option<f64> {
        self.stats
            .as_ref()
            .and_then(|s| s.min)
            .or_else(|| self.durations_micros.iter().min().map(|&v| v as f64))
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct BenchSuite {
    pub name: String,
    pub tests: Vec<BenchTest>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BenchRunRaw {
    #[serde(default)]
    pub metadata: BenchMeta,
    #[serde(default)]
    pub suites: Vec<BenchSuite>,
}

#[derive(Debug, Clone)]
pub struct BenchData {
    pub latest: Option<BenchRunRaw>,
    pub history: Vec<BenchRunRaw>,
}

pub struct BenchSource {
    latest_path: PathBuf,
    jsonl_path: PathBuf,
}

impl BenchSource {
    pub fn new() -> Self {
        let base = std::env::current_dir()
            .unwrap_or_default()
            .join("bench/results");
        Self {
            latest_path: base.join("latest.json"),
            jsonl_path: base.join("bench.jsonl"),
        }
    }
}

impl DataSource for BenchSource {
    type Data = BenchData;

    fn load(&self) -> Result<BenchData> {
        let latest = if self.latest_path.exists() {
            let content = std::fs::read_to_string(&self.latest_path)
                .with_context(|| format!("read {}", self.latest_path.display()))?;
            serde_json::from_str(&content).ok()
        } else {
            None
        };

        let mut history = Vec::new();
        if self.jsonl_path.exists() {
            let content = std::fs::read_to_string(&self.jsonl_path)?;
            for line in content.lines() {
                if let Ok(run) = serde_json::from_str::<BenchRunRaw>(line) {
                    history.push(run);
                }
            }
        }

        Ok(BenchData { latest, history })
    }
}
