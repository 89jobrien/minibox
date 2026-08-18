//! Container-related MCP tool implementations.

use crate::client::MiniboxDaemonClient;
use crate::error::{McpServerError, Result};
use crate::policy::AgentPolicy;
use crate::types::{
    ContainerIdInput, ContainerInfoOutput, ImagesOutput, LogEntry, LogsInput, LogsOutput,
    ManifestOutput, MountInput, PsOutput, RunContainerInput, RunContainerOutput, SimpleOutput,
    parse_network_mode, require_non_empty,
};
use base64::Engine as _;
use minibox_core::domain::BindMount;
use minibox_core::protocol::{DaemonRequest, DaemonResponse, OutputStreamKind};
use std::path::{Component, Path, PathBuf};

/// List containers known to the daemon.
///
/// # Errors
///
/// Returns an error if the daemon call fails or returns an unexpected response.
pub async fn ps(client: &MiniboxDaemonClient, policy: &AgentPolicy) -> Result<PsOutput> {
    let result = client
        .call_limited(DaemonRequest::List, policy.max_output_bytes)
        .await?;
    result
        .responses
        .into_iter()
        .find_map(|response| match response {
            DaemonResponse::ContainerList { containers } => Some(PsOutput {
                containers: containers
                    .into_iter()
                    .map(ContainerInfoOutput::from)
                    .collect(),
            }),
            _ => None,
        })
        .ok_or_else(|| McpServerError::UnexpectedResponse {
            tool: "minibox_ps",
            response: format!("{:?}", result.raw_responses),
        })
}

/// Fetch cached container logs.
///
/// # Errors
///
/// Returns an error if the input is invalid or the daemon call fails.
pub async fn logs(
    client: &MiniboxDaemonClient,
    policy: &AgentPolicy,
    input: LogsInput,
) -> Result<LogsOutput> {
    require_non_empty(&input.id, "id")?;
    let result = client
        .call_limited(
            DaemonRequest::ContainerLogs {
                container_id: input.id,
                follow: false,
            },
            policy.max_output_bytes,
        )
        .await?;
    let lines = result
        .responses
        .iter()
        .filter_map(|response| match response {
            DaemonResponse::LogLine { stream, line } => Some(LogEntry {
                stream: stream_name(stream).to_string(),
                line: line.clone(),
            }),
            _ => None,
        })
        .collect();

    Ok(LogsOutput {
        lines,
        daemon_responses: result.raw_responses,
    })
}

/// Retrieve an execution manifest.
///
/// # Errors
///
/// Returns an error if the input is invalid, daemon call fails, or response is unexpected.
pub async fn manifest(
    client: &MiniboxDaemonClient,
    policy: &AgentPolicy,
    input: ContainerIdInput,
) -> Result<ManifestOutput> {
    require_non_empty(&input.id, "id")?;
    let result = client
        .call_limited(
            DaemonRequest::GetManifest { id: input.id },
            policy.max_output_bytes,
        )
        .await?;
    result
        .responses
        .into_iter()
        .find_map(|response| match response {
            DaemonResponse::Manifest { manifest } => Some(ManifestOutput { manifest }),
            _ => None,
        })
        .ok_or_else(|| McpServerError::UnexpectedResponse {
            tool: "minibox_manifest",
            response: format!("{:?}", result.raw_responses),
        })
}

/// Run a container with safe agent defaults.
///
/// # Errors
///
/// Returns an error if policy rejects the input, request mapping fails, or the daemon call fails.
pub async fn run(
    client: &MiniboxDaemonClient,
    policy: &AgentPolicy,
    input: RunContainerInput,
) -> Result<RunContainerOutput> {
    // Deliberate asymmetry with stop/rm/pull: ephemeral, unprivileged,
    // network-isolated runs are the core agent workflow and stay available by
    // default; everything that escalates (privileged, mounts, host network) or
    // mutates shared daemon state is gated. Documented in this crate's README.
    policy.validate_run(&input)?;
    let request = run_request(input, policy)?;
    let result = client
        .call_limited(request, policy.max_output_bytes)
        .await?;
    normalize_run_output(
        result.responses,
        result.raw_responses,
        policy.max_output_bytes,
    )
}

/// Stop a container.
///
/// # Errors
///
/// Returns an error if policy rejects the operation, input is invalid, or daemon call fails.
pub async fn stop(
    client: &MiniboxDaemonClient,
    policy: &AgentPolicy,
    input: ContainerIdInput,
) -> Result<SimpleOutput> {
    policy.validate_mutation("minibox_stop")?;
    simple_id_request(client, policy, input, "minibox_stop", |id| {
        DaemonRequest::Stop { id }
    })
    .await
}

/// Remove a stopped container.
///
/// # Errors
///
/// Returns an error if policy rejects the operation, input is invalid, or daemon call fails.
pub async fn rm(
    client: &MiniboxDaemonClient,
    policy: &AgentPolicy,
    input: ContainerIdInput,
) -> Result<SimpleOutput> {
    policy.validate_mutation("minibox_rm")?;
    simple_id_request(client, policy, input, "minibox_rm", |id| {
        DaemonRequest::Remove { id }
    })
    .await
}

/// Re-export image list output for docs/tests that group container and image tools together.
#[must_use]
pub const fn empty_images_output() -> ImagesOutput {
    ImagesOutput { images: Vec::new() }
}

fn run_request(input: RunContainerInput, policy: &AgentPolicy) -> Result<DaemonRequest> {
    // policy.validate_run() has already parsed and gated the same network
    // string; this re-parse cannot disagree with the gate's decision.
    let network_mode = parse_network_mode(input.network.as_deref())?;
    let mounts = input
        .mounts
        .into_iter()
        .map(parse_mount)
        .collect::<Result<Vec<_>>>()?;

    Ok(DaemonRequest::Run {
        image: input.image,
        tag: input.tag,
        command: input.command,
        memory_limit_bytes: input
            .memory_limit_bytes
            .or(policy.default_memory_limit_bytes),
        cpu_weight: input.cpu_weight.or(policy.default_cpu_weight),
        ephemeral: true,
        network: Some(network_mode),
        env: input.env,
        mounts,
        privileged: input.privileged.unwrap_or(false),
        name: input.name,
        tty: false,
        entrypoint: None,
        user: None,
        auto_remove: input.auto_remove.unwrap_or(true),
        priority: None,
        urgency: None,
        execution_context: None,
        platform: input.platform,
        cgroup_parent: None,
    })
}

fn parse_mount(input: MountInput) -> Result<BindMount> {
    let host_path = PathBuf::from(input.host_path);
    validate_absolute_clean_path(&host_path, "host_path")?;
    let container_path = PathBuf::from(input.container_path);
    validate_absolute_clean_path(&container_path, "container_path")?;
    Ok(BindMount {
        host_path,
        container_path,
        read_only: input.read_only,
    })
}

fn validate_absolute_clean_path(path: &Path, field: &'static str) -> Result<()> {
    if !path.is_absolute() {
        return Err(McpServerError::InvalidInput(format!(
            "{field} must be absolute"
        )));
    }
    if path
        .components()
        .any(|component| component == Component::ParentDir)
    {
        return Err(McpServerError::InvalidInput(format!(
            "{field} must not contain '..' components"
        )));
    }
    Ok(())
}

fn normalize_run_output(
    responses: Vec<DaemonResponse>,
    raw_responses: Vec<serde_json::Value>,
    max_output_bytes: usize,
) -> Result<RunContainerOutput> {
    let mut container_id = None;
    let mut stdout = String::new();
    let mut stderr = String::new();
    let mut exit_code = None;
    let mut truncated = false;

    for response in responses {
        match response {
            DaemonResponse::ContainerCreated { id } => {
                container_id = Some(id);
            }
            DaemonResponse::ContainerOutput { stream, data } => {
                let decoded = base64::engine::general_purpose::STANDARD
                    .decode(data)
                    .map_err(|e| McpServerError::InvalidInput(e.to_string()))?;
                append_output(
                    match stream {
                        OutputStreamKind::Stdout => &mut stdout,
                        OutputStreamKind::Stderr => &mut stderr,
                    },
                    &decoded,
                    max_output_bytes,
                    &mut truncated,
                );
            }
            DaemonResponse::ContainerStopped { exit_code: code } => {
                exit_code = Some(code);
            }
            other if other.is_terminal() => {
                tracing::warn!(
                    response = ?other,
                    "run: ignoring unexpected terminal response; output may be incomplete"
                );
            }
            other => {
                return Err(McpServerError::UnexpectedResponse {
                    tool: "minibox_run",
                    response: format!("{other:?}"),
                });
            }
        }
    }

    Ok(RunContainerOutput {
        container_id,
        stdout,
        stderr,
        exit_code,
        truncated,
        daemon_responses: raw_responses,
    })
}

fn append_output(target: &mut String, bytes: &[u8], max_len: usize, truncated: &mut bool) {
    if target.len() >= max_len {
        *truncated = true;
        return;
    }
    let remaining = max_len - target.len();
    let take = bytes.len().min(remaining);
    target.push_str(&String::from_utf8_lossy(&bytes[..take]));
    if take < bytes.len() {
        *truncated = true;
    }
}

async fn simple_id_request(
    client: &MiniboxDaemonClient,
    policy: &AgentPolicy,
    input: ContainerIdInput,
    tool: &'static str,
    request_builder: impl FnOnce(String) -> DaemonRequest,
) -> Result<SimpleOutput> {
    require_non_empty(&input.id, "id")?;
    let result = client
        .call_limited(request_builder(input.id), policy.max_output_bytes)
        .await?;
    let message = result
        .responses
        .iter()
        .find_map(|response| match response {
            DaemonResponse::Success { message } => Some(message.clone()),
            DaemonResponse::ContainerStopped { exit_code } => {
                Some(format!("container stopped with exit code {exit_code}"))
            }
            DaemonResponse::ContainerPaused { id } => Some(format!("{id} paused")),
            DaemonResponse::ContainerResumed { id } => Some(format!("{id} resumed")),
            _ => None,
        })
        .ok_or_else(|| McpServerError::UnexpectedResponse {
            tool,
            response: format!("{:?}", result.raw_responses),
        })?;

    Ok(SimpleOutput {
        message,
        daemon_responses: result.raw_responses,
    })
}

const fn stream_name(stream: &OutputStreamKind) -> &'static str {
    match stream {
        OutputStreamKind::Stdout => "stdout",
        OutputStreamKind::Stderr => "stderr",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use minibox_core::domain::NetworkMode;

    #[test]
    fn run_request_defaults_to_ephemeral_auto_remove_and_no_network() {
        let policy = AgentPolicy::safe_default();
        let request = run_request(
            RunContainerInput {
                image: "alpine".to_string(),
                command: vec!["/bin/true".to_string()],
                ..RunContainerInput::default()
            },
            &policy,
        )
        .expect("request should build");

        let DaemonRequest::Run {
            ephemeral,
            auto_remove,
            network,
            memory_limit_bytes,
            cpu_weight,
            ..
        } = request
        else {
            panic!("expected Run request");
        };

        assert!(ephemeral);
        assert!(auto_remove);
        assert_eq!(network, Some(NetworkMode::None));
        assert_eq!(memory_limit_bytes, policy.default_memory_limit_bytes);
        assert_eq!(cpu_weight, policy.default_cpu_weight);
    }

    #[test]
    fn parse_mount_rejects_parent_dir() {
        let result = parse_mount(MountInput {
            host_path: "/tmp/../etc".to_string(),
            container_path: "/data".to_string(),
            read_only: true,
        });

        assert!(matches!(result, Err(McpServerError::InvalidInput(_))));
    }

    #[test]
    fn normalize_run_output_decodes_stdout_and_stderr() {
        let stdout = base64::engine::general_purpose::STANDARD.encode(b"hello\n");
        let stderr = base64::engine::general_purpose::STANDARD.encode(b"warn\n");
        let output = normalize_run_output(
            vec![
                DaemonResponse::ContainerCreated {
                    id: "abc123".to_string(),
                },
                DaemonResponse::ContainerOutput {
                    stream: OutputStreamKind::Stdout,
                    data: stdout,
                },
                DaemonResponse::ContainerOutput {
                    stream: OutputStreamKind::Stderr,
                    data: stderr,
                },
                DaemonResponse::ContainerStopped { exit_code: 0 },
            ],
            Vec::new(),
            1024,
        )
        .expect("normalize output");

        assert_eq!(output.container_id.as_deref(), Some("abc123"));
        assert_eq!(output.stdout, "hello\n");
        assert_eq!(output.stderr, "warn\n");
        assert_eq!(output.exit_code, Some(0));
        assert!(!output.truncated);
    }
}
