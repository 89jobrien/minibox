//! `mbx pipeline` — run, list, and show pipeline executions.

use anyhow::{Context as _, Result, bail};
use minibox_core::client::DaemonClient;
use minibox_core::protocol::{DaemonRequest, DaemonResponse};
use std::path::Path;

/// Execute `mbx pipeline run <file>`.
///
/// Sends a `RunPipeline` request to the daemon, streams `ContainerOutput`
/// chunks to stdout/stderr, and prints the final trace on completion.
pub async fn execute_run(
    pipeline_path: String,
    input: Option<String>,
    image: Option<String>,
    socket_path: &Path,
) -> Result<()> {
    let input_json = match input {
        Some(s) => {
            let v: serde_json::Value =
                serde_json::from_str(&s).context("--input must be valid JSON")?;
            Some(v)
        }
        None => None,
    };

    let client = DaemonClient::with_socket(socket_path);
    let mut stream = client
        .call(DaemonRequest::RunPipeline {
            pipeline_path,
            input: input_json,
            image,
            budget: None,
            env: vec![],
            max_depth: 3,
            priority: None,
            urgency: None,
            execution_context: None,
        })
        .await
        .context("failed to call daemon")?;

    while let Some(response) = stream.next().await.context("stream error")? {
        match response {
            DaemonResponse::ContainerOutput { data, .. } => {
                print!("{data}");
            }
            DaemonResponse::PipelineComplete {
                trace,
                container_id,
                exit_code,
            } => {
                if exit_code != 0 {
                    bail!("pipeline failed (container {container_id}, exit code {exit_code})");
                }
                println!(
                    "{}",
                    serde_json::to_string_pretty(&trace).unwrap_or_else(|_| format!("{trace:?}"))
                );
                return Ok(());
            }
            DaemonResponse::Error { message } => {
                bail!("daemon error: {message}");
            }
            _ => {}
        }
    }

    bail!("no response from daemon");
}

/// Execute `mbx pipeline list`.
///
/// Sends a `ListPipelines` request and prints a table of pipeline runs.
pub async fn execute_list(
    limit: Option<usize>,
    pipeline: Option<String>,
    socket_path: &Path,
) -> Result<()> {
    let client = DaemonClient::with_socket(socket_path);
    let mut stream = client
        .call(DaemonRequest::ListPipelines { limit, pipeline })
        .await
        .context("failed to call daemon")?;

    if let Some(response) = stream.next().await.context("stream error")? {
        match response {
            DaemonResponse::PipelineList { pipelines } => {
                if pipelines.is_empty() {
                    println!("no pipeline runs found");
                    return Ok(());
                }
                let header = format!(
                    "{:<16} {:<40} {:>4} {:>5} {}",
                    "ID", "PIPELINE", "EXIT", "STEPS", "TIMESTAMP"
                );
                println!("{header}");
                for p in &pipelines {
                    println!(
                        "{:<16} {:<40} {:>4} {:>5} {}",
                        truncate_id(&p.id, 16),
                        truncate_id(&p.pipeline, 40),
                        p.exit_code,
                        p.step_count,
                        p.timestamp,
                    );
                }
                Ok(())
            }
            DaemonResponse::Error { message } => {
                bail!("daemon error: {message}");
            }
            other => {
                bail!("unexpected response: {other:?}");
            }
        }
    } else {
        bail!("no response from daemon");
    }
}

/// Execute `mbx pipeline show <id>`.
///
/// Sends a `ShowPipeline` request and prints the full trace JSON.
pub async fn execute_show(id: String, socket_path: &Path) -> Result<()> {
    let client = DaemonClient::with_socket(socket_path);
    let mut stream = client
        .call(DaemonRequest::ShowPipeline { id })
        .await
        .context("failed to call daemon")?;

    if let Some(response) = stream.next().await.context("stream error")? {
        match response {
            DaemonResponse::PipelineDetail { trace, .. } => {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&trace).unwrap_or_else(|_| format!("{trace:?}"))
                );
                Ok(())
            }
            DaemonResponse::Error { message } => {
                bail!("daemon error: {message}");
            }
            other => {
                bail!("unexpected response: {other:?}");
            }
        }
    } else {
        bail!("no response from daemon");
    }
}

fn truncate_id(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max.saturating_sub(3)])
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_helpers::{setup, setup_multi};
    use super::*;
    use minibox_core::protocol::DaemonResponse;
    use minibox_core::trace::TraceSummary;

    #[cfg(unix)]
    #[tokio::test]
    async fn execute_list_succeeds_on_pipeline_list_response() {
        let (_tmp, socket_path) = setup(DaemonResponse::PipelineList {
            pipelines: vec![TraceSummary {
                id: "trace-1".to_string(),
                pipeline: "test.cruxx".to_string(),
                timestamp: "1700000000".to_string(),
                exit_code: 0,
                step_count: 3,
            }],
        })
        .await;
        let result = execute_list(None, None, &socket_path).await;
        assert!(result.is_ok(), "execute_list should succeed: {result:?}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn execute_show_succeeds_on_pipeline_detail_response() {
        let (_tmp, socket_path) = setup(DaemonResponse::PipelineDetail {
            id: "trace-1".to_string(),
            trace: serde_json::json!({"steps": []}),
        })
        .await;
        let result = execute_show("trace-1".to_string(), &socket_path).await;
        assert!(result.is_ok(), "execute_show should succeed: {result:?}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn execute_run_succeeds_on_pipeline_complete_response() {
        let (_tmp, socket_path) = setup_multi(vec![
            DaemonResponse::ContainerOutput {
                stream: minibox_core::protocol::OutputStreamKind::Stdout,
                data: "hello\n".to_string(),
            },
            DaemonResponse::PipelineComplete {
                trace: serde_json::json!({"steps": []}),
                container_id: "ctr-1".to_string(),
                exit_code: 0,
            },
        ])
        .await;
        let result = execute_run("/tmp/test.cruxx".to_string(), None, None, &socket_path).await;
        assert!(result.is_ok(), "execute_run should succeed: {result:?}");
    }

    #[test]
    fn truncate_id_short_string() {
        assert_eq!(truncate_id("abc", 16), "abc");
    }

    #[test]
    fn truncate_id_long_string() {
        let long = "a".repeat(20);
        let result = truncate_id(&long, 16);
        assert!(result.len() <= 16);
        assert!(result.ends_with("..."));
    }
}
