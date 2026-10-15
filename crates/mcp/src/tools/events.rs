//! Event-stream MCP tool implementation.

use crate::client::MiniboxDaemonClient;
use crate::error::Result;
use crate::policy::AgentPolicy;
use crate::types::{ContainerEventOutput, EventsInput, EventsOutput};
use minibox_core::protocol::{DaemonRequest, DaemonResponse};

const DEFAULT_EVENTS_DURATION_MS: u64 = 2_000;
const MAX_EVENTS_DURATION_MS: u64 = 10_000;

/// Collect container lifecycle events for a bounded time window.
///
/// # Errors
///
/// Returns an error if the daemon call fails.
pub async fn events(
    client: &MiniboxDaemonClient,
    policy: &AgentPolicy,
    input: EventsInput,
) -> Result<EventsOutput> {
    let duration_ms = input
        .duration_ms
        .unwrap_or(DEFAULT_EVENTS_DURATION_MS)
        .clamp(1, MAX_EVENTS_DURATION_MS);
    let result = client
        .call_for_duration(
            DaemonRequest::SubscribeEvents,
            policy.max_output_bytes,
            std::time::Duration::from_millis(duration_ms),
        )
        .await?;

    let events: Vec<ContainerEventOutput> = result
        .responses
        .into_iter()
        .filter_map(|response| match response {
            DaemonResponse::Event { event } => Some(ContainerEventOutput::from(event)),
            _ => None,
        })
        .collect();
    let count = events.len();

    Ok(EventsOutput { events, count })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[tokio::test]
    async fn events_reports_connection_error_for_unreachable_daemon() {
        let client = MiniboxDaemonClient::new(PathBuf::from("/nonexistent/minibox.sock"));
        let policy = AgentPolicy::safe_default();

        let result = events(&client, &policy, EventsInput::default()).await;

        assert!(result.is_err());
    }
}
