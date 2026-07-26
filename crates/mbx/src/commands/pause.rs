//! `minibox pause` — freeze a running container.

use minibox_core::protocol::DaemonRequest;

/// Execute the `pause` subcommand.
///
/// Sends a `PauseContainer` request to the daemon, which freezes the container.
/// Prints the daemon's confirmation message on success or an error description on failure.
pub async fn execute(id: String, socket_path: &std::path::Path) -> anyhow::Result<()> {
    super::send_request(DaemonRequest::PauseContainer { id }, socket_path).await
}

#[cfg(test)]
mod tests {
    use super::super::test_helpers::setup;
    use super::*;
    use minibox_core::protocol::DaemonResponse;

    #[cfg(unix)]
    #[tokio::test]
    async fn execute_succeeds_on_success_response() {
        let (_tmp, socket_path) = setup(DaemonResponse::Success {
            message: "paused".to_string(),
        })
        .await;
        let result = execute("abc123".to_string(), &socket_path).await;
        assert!(result.is_ok(), "execute should succeed: {result:?}");
    }
}
