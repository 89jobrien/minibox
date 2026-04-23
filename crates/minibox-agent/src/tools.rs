//! Tool execution port and built-in adapter.
//!
//! # Architecture
//!
//! - Port: [`ToolExecutor`] — domain trait; implementations live in adapters.
//! - Adapter: [`BuiltinToolExecutor`] — bash, read, write, glob, edit operations.
//! - Test double: [`InMemoryToolExecutor`] — returns canned responses; used in
//!   unit tests so no real filesystem or shell is touched.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Error from a tool call.
#[derive(Debug, Error, Clone)]
pub enum ToolError {
    /// The requested tool name is not registered.
    #[error("unknown tool: {0}")]
    UnknownTool(String),
    /// Tool execution failed with a descriptive message.
    #[error("tool execution failed: {0}")]
    ExecutionFailed(String),
}

/// Input to a single tool invocation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolInput {
    /// The tool name, e.g. `"bash"` or `"read"`.
    pub name: String,
    /// Arbitrary JSON arguments for the tool.
    pub args: serde_json::Value,
}

/// Output from a tool invocation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolOutput {
    /// The tool name that produced this output.
    pub name: String,
    /// Plain-text or JSON result content.
    pub content: String,
    /// `true` if the tool signalled an error in `content` (non-fatal).
    pub is_error: bool,
}

impl ToolOutput {
    /// Construct a successful output.
    pub fn ok(name: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            content: content.into(),
            is_error: false,
        }
    }

    /// Construct an error output.
    pub fn err(name: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            content: content.into(),
            is_error: true,
        }
    }
}

/// Port: domain trait for executing named tools.
///
/// Implement this to provide a concrete tool backend. The trait is
/// object-safe so it can be stored as `Box<dyn ToolExecutor>`.
pub trait ToolExecutor: Send + Sync {
    /// Execute the given tool with its arguments.
    ///
    /// Returns [`ToolOutput`] on success (which may itself carry
    /// `is_error = true` for tool-level errors). Returns [`ToolError`]
    /// only for hard failures (unknown tool, executor crash).
    fn execute(&self, input: ToolInput) -> Result<ToolOutput, ToolError>;
}

// ── In-memory test double ─────────────────────────────────────────────────────

/// Test double: returns canned responses keyed on tool name.
///
/// Pre-register responses with [`InMemoryToolExecutor::register`].
/// Any tool name without a registered response returns [`ToolError::UnknownTool`].
#[derive(Default)]
pub struct InMemoryToolExecutor {
    responses: HashMap<String, ToolOutput>,
}

impl InMemoryToolExecutor {
    /// Create an empty executor.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a canned [`ToolOutput`] for the given tool name.
    pub fn register(&mut self, name: impl Into<String>, output: ToolOutput) {
        self.responses.insert(name.into(), output);
    }
}

impl ToolExecutor for InMemoryToolExecutor {
    fn execute(&self, input: ToolInput) -> Result<ToolOutput, ToolError> {
        self.responses
            .get(&input.name)
            .cloned()
            .ok_or_else(|| ToolError::UnknownTool(input.name.clone()))
    }
}

// ── BuiltinToolExecutor ───────────────────────────────────────────────────────

/// Adapter: executes bash, read, write, glob, and edit tools against the
/// real filesystem / shell.
///
/// This adapter lives here in the domain module as a "thin" adapter with
/// no external crate deps — it only uses `std`. More exotic backends (HTTP,
/// Docker exec) would live in `infra/`.
pub struct BuiltinToolExecutor;

impl ToolExecutor for BuiltinToolExecutor {
    fn execute(&self, input: ToolInput) -> Result<ToolOutput, ToolError> {
        match input.name.as_str() {
            "bash" => run_bash(&input),
            "read" => run_read(&input),
            "write" => run_write(&input),
            "glob" => run_glob(&input),
            "edit" => run_edit(&input),
            other => Err(ToolError::UnknownTool(other.to_owned())),
        }
    }
}

fn run_bash(input: &ToolInput) -> Result<ToolOutput, ToolError> {
    let cmd = input
        .args
        .get("command")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::ExecutionFailed("bash: missing 'command' arg".into()))?
        .to_owned();

    let output = std::process::Command::new("sh")
        .args(["-c", &cmd])
        .output()
        .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let content = if stderr.is_empty() {
        stdout
    } else {
        format!("{stdout}\nstderr: {stderr}")
    };
    Ok(ToolOutput {
        name: input.name.clone(),
        content,
        is_error: !output.status.success(),
    })
}

fn run_read(input: &ToolInput) -> Result<ToolOutput, ToolError> {
    let path = input
        .args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::ExecutionFailed("read: missing 'path' arg".into()))?;

    let content = std::fs::read_to_string(path)
        .map_err(|e| ToolError::ExecutionFailed(format!("read {path}: {e}")))?;
    Ok(ToolOutput::ok(input.name.clone(), content))
}

fn run_write(input: &ToolInput) -> Result<ToolOutput, ToolError> {
    let path = input
        .args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::ExecutionFailed("write: missing 'path' arg".into()))?;
    let content = input
        .args
        .get("content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::ExecutionFailed("write: missing 'content' arg".into()))?;

    std::fs::write(path, content)
        .map_err(|e| ToolError::ExecutionFailed(format!("write {path}: {e}")))?;
    Ok(ToolOutput::ok(input.name.clone(), format!("wrote {path}")))
}

fn run_glob(input: &ToolInput) -> Result<ToolOutput, ToolError> {
    let pattern = input
        .args
        .get("pattern")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::ExecutionFailed("glob: missing 'pattern' arg".into()))?;

    let paths: Vec<String> = glob::glob(pattern)
        .map_err(|e| ToolError::ExecutionFailed(format!("glob pattern error: {e}")))?
        .filter_map(|p| p.ok())
        .map(|p| p.display().to_string())
        .collect();

    Ok(ToolOutput::ok(input.name.clone(), paths.join("\n")))
}

fn run_edit(input: &ToolInput) -> Result<ToolOutput, ToolError> {
    let path = input
        .args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::ExecutionFailed("edit: missing 'path' arg".into()))?;
    let old = input
        .args
        .get("old_string")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::ExecutionFailed("edit: missing 'old_string' arg".into()))?;
    let new = input
        .args
        .get("new_string")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::ExecutionFailed("edit: missing 'new_string' arg".into()))?;

    let original = std::fs::read_to_string(path)
        .map_err(|e| ToolError::ExecutionFailed(format!("edit read {path}: {e}")))?;

    if !original.contains(old) {
        return Err(ToolError::ExecutionFailed(format!(
            "edit: old_string not found in {path}"
        )));
    }

    let updated = original.replacen(old, new, 1);
    std::fs::write(path, &updated)
        .map_err(|e| ToolError::ExecutionFailed(format!("edit write {path}: {e}")))?;
    Ok(ToolOutput::ok(input.name.clone(), format!("edited {path}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_memory_executor_returns_registered_output() {
        let mut exec = InMemoryToolExecutor::new();
        exec.register("bash", ToolOutput::ok("bash", "hello"));

        let input = ToolInput {
            name: "bash".into(),
            args: serde_json::json!({"command": "echo hello"}),
        };
        let out = exec.execute(input).expect("should succeed");
        assert_eq!(out.content, "hello");
        assert!(!out.is_error);
    }

    #[test]
    fn in_memory_executor_unknown_tool_returns_error() {
        let exec = InMemoryToolExecutor::new();
        let input = ToolInput {
            name: "unknown".into(),
            args: serde_json::json!({}),
        };
        let err = exec.execute(input).unwrap_err();
        assert!(matches!(err, ToolError::UnknownTool(_)));
    }

    #[test]
    fn tool_output_ok_sets_is_error_false() {
        let out = ToolOutput::ok("read", "file contents");
        assert!(!out.is_error);
        assert_eq!(out.content, "file contents");
    }

    #[test]
    fn tool_output_err_sets_is_error_true() {
        let out = ToolOutput::err("bash", "command not found");
        assert!(out.is_error);
    }
}
