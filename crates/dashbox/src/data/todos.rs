// dashbox/src/data/todos.rs
use anyhow::{Context, Result};
use std::process::Command;

use super::DataSource;
use crate::data::items::HandoffItem;

#[derive(Debug, Clone)]
pub struct Todo {
    pub content: String,
    pub status: String,
    pub priority: String,
    pub tags: Vec<String>,
    #[allow(dead_code)]
    pub doob_uuid: String,
}

#[derive(Debug, Clone)]
pub struct TodosData {
    pub todos: Vec<Todo>,
    pub total: usize,
    pub pending: usize,
    pub completed: usize,
}

pub struct TodosSource;

impl TodosSource {
    pub fn new() -> Self {
        Self
    }
}

fn map_handoff_to_todo(item: HandoffItem) -> Todo {
    let status = match item.status.as_str() {
        "open" => "pending",
        "done" => "completed",
        "blocked" => "blocked",
        other => other,
    };
    Todo {
        content: item.title,
        status: status.to_string(),
        priority: item.priority,
        tags: vec![item.handoff_id],
        doob_uuid: item.doob_uuid,
    }
}

impl DataSource for TodosSource {
    type Data = TodosData;

    fn load(&self) -> Result<TodosData> {
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

        let filtered: Vec<HandoffItem> = all
            .into_iter()
            .filter(|i| i.handoff_id.starts_with("minibox-"))
            .collect();

        let todos: Vec<Todo> = filtered.into_iter().map(map_handoff_to_todo).collect();

        let total = todos.len();
        let pending = todos.iter().filter(|t| t.status == "pending").count();
        let completed = todos.iter().filter(|t| t.status == "completed").count();

        // Sort: pending first, then by priority
        let mut sorted = todos;
        sorted.sort_by(|a, b| {
            let p_ord = |p: &str| match p {
                "P0" => 0,
                "P1" => 1,
                _ => 2,
            };
            let a_pending = a.status == "pending";
            let b_pending = b.status == "pending";
            b_pending
                .cmp(&a_pending)
                .then(p_ord(&a.priority).cmp(&p_ord(&b.priority)))
        });

        Ok(TodosData {
            todos: sorted,
            total,
            pending,
            completed,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_todos_source_maps_open_to_pending() {
        let item = HandoffItem {
            handoff_id: "minibox-1".into(),
            title: "Fix the thing".into(),
            description: None,
            priority: "P1".into(),
            status: "open".into(),
            files: vec![],
            doob_uuid: "uuid-xyz".into(),
        };
        let todo = map_handoff_to_todo(item);
        assert_eq!(todo.status, "pending");
        assert_eq!(todo.content, "Fix the thing");
        assert_eq!(todo.priority, "P1");
        assert_eq!(todo.tags, vec!["minibox-1".to_string()]);
        assert_eq!(todo.doob_uuid, "uuid-xyz");
    }
}
