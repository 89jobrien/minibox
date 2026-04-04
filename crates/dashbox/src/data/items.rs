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
    #[serde(default, rename = "uuid")]
    pub doob_uuid: String,
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
            .filter(|i| i.handoff_id.starts_with(&format!("{}-", self.project)))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_handoff_item_deserializes_doob_uuid() {
        let json = r#"{
            "handoff_id": "minibox-1",
            "title": "Test item",
            "priority": "P1",
            "status": "open",
            "uuid": "abc-123"
        }"#;
        let item: HandoffItem = serde_json::from_str(json).expect("parse");
        assert_eq!(item.doob_uuid, "abc-123");
    }

    #[test]
    fn test_handoff_item_optional_fields_absent() {
        // description, files, and uuid are all optional — must not fail to parse
        let json = r#"{
            "handoff_id": "minibox-2",
            "title": "Minimal item",
            "priority": "P2",
            "status": "open"
        }"#;
        let item: HandoffItem = serde_json::from_str(json).expect("parse");
        assert!(item.description.is_none());
        assert!(item.files.is_empty());
        assert_eq!(item.doob_uuid, "");
    }

    #[test]
    fn test_handoff_item_empty_uuid_is_empty_string() {
        // empty doob_uuid means the key handler should return TabAction::None
        let json = r#"{
            "handoff_id": "minibox-3",
            "title": "No uuid",
            "priority": "P1",
            "status": "open",
            "uuid": ""
        }"#;
        let item: HandoffItem = serde_json::from_str(json).expect("parse");
        assert_eq!(item.doob_uuid, "");
    }

    #[test]
    fn test_handoff_item_unknown_fields_ignored() {
        // doob may add fields in future — unknown fields must not break parsing
        let json = r#"{
            "handoff_id": "minibox-4",
            "title": "Future item",
            "priority": "P0",
            "status": "blocked",
            "uuid": "xyz-999",
            "unknown_future_field": true,
            "another_field": 42
        }"#;
        let item: HandoffItem = serde_json::from_str(json).expect("parse");
        assert_eq!(item.handoff_id, "minibox-4");
        assert_eq!(item.doob_uuid, "xyz-999");
    }
}
