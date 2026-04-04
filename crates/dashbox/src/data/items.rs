// dashbox/src/data/items.rs
use anyhow::{Context, Result};
use serde::Deserialize;
use std::process::Command;

use super::DataSource;

#[derive(Debug, Clone, Deserialize)]
pub struct HandoffItem {
    pub handoff_id: String,
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    pub priority: String,
    pub status: String,
    #[serde(default)]
    pub files: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ItemsData {
    pub items: Vec<HandoffItem>,
    pub open: usize,
    pub done: usize,
    pub blocked: usize,
}

pub struct ItemsSource {
    project: String,
}

impl ItemsSource {
    pub fn new(project: impl Into<String>) -> Self {
        Self {
            project: project.into(),
        }
    }
}

impl DataSource for ItemsSource {
    type Data = ItemsData;

    fn load(&self) -> Result<ItemsData> {
        let output = Command::new("doob")
            .args(["handoff", "list", "--json"])
            .output()
            .context("failed to run doob")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("doob exited with {}: {}", output.status, stderr.trim());
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let all: Vec<HandoffItem> =
            serde_json::from_str(&stdout).context("failed to parse doob handoff list output")?;

        let mut items: Vec<HandoffItem> = all
            .into_iter()
            .filter(|i| {
                i.handoff_id
                    .starts_with(&format!("{}-", self.project))
            })
            .collect();

        // Sort: P0 first, then P1, P2; within priority: open before done/parked/blocked
        items.sort_by(|a, b| {
            let p_ord = |p: &str| match p {
                "P0" => 0,
                "P1" => 1,
                _ => 2,
            };
            let s_ord = |s: &str| match s {
                "open" => 0,
                "blocked" => 1,
                "parked" => 2,
                _ => 3,
            };
            p_ord(&a.priority)
                .cmp(&p_ord(&b.priority))
                .then(s_ord(&a.status).cmp(&s_ord(&b.status)))
        });

        let open = items.iter().filter(|i| i.status == "open").count();
        let done = items.iter().filter(|i| i.status == "done").count();
        let blocked = items.iter().filter(|i| i.status == "blocked").count();

        Ok(ItemsData {
            items,
            open,
            done,
            blocked,
        })
    }
}
