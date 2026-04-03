// dashbox/src/data/todos.rs
use anyhow::{Context, Result};
use serde::Deserialize;
use std::process::Command;

use super::DataSource;

#[derive(Debug, Clone, Deserialize)]
pub struct Todo {
    pub content: String,
    pub status: String,
    pub priority: u32,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    #[allow(dead_code)]
    pub created_at: String,
}

#[derive(Debug, Clone, Deserialize)]
struct TodoListResponse {
    count: usize,
    todos: Vec<Todo>,
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

impl DataSource for TodosSource {
    type Data = TodosData;

    fn load(&self) -> Result<TodosData> {
        let output = Command::new("doob")
            .args(["todo", "list", "--project", "minibox", "--json"])
            .output()
            .context("failed to run doob")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("doob exited with {}: {}", output.status, stderr.trim());
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let response: TodoListResponse =
            serde_json::from_str(&stdout).context("failed to parse doob output")?;

        let total = response.count;
        let pending = response
            .todos
            .iter()
            .filter(|t| t.status == "pending")
            .count();
        let completed = response
            .todos
            .iter()
            .filter(|t| t.status == "completed")
            .count();

        // Sort pending first by priority descending
        let mut todos = response.todos;
        todos.sort_by(|a, b| {
            let a_pending = a.status == "pending";
            let b_pending = b.status == "pending";
            b_pending.cmp(&a_pending).then(b.priority.cmp(&a.priority))
        });

        Ok(TodosData {
            todos,
            total,
            pending,
            completed,
        })
    }
}
