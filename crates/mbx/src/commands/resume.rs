//! `minibox resume` — thaw a paused container.

use minibox_core::protocol::DaemonRequest;

/// Execute the `resume` subcommand.
///
/// Sends a `ResumeContainer` request to the daemon, which thaws the container.
/// Prints the daemon's confirmation message on success or an error description on failure.
pub async fn execute(id: String, socket_path: &std::path::Path) -> anyhow::Result<()> {
    super::send_request(DaemonRequest::ResumeContainer { id }, socket_path).await
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
            message: "resumed".to_string(),
        })
        .await;
        let result = execute("abc123".to_string(), &socket_path).await;
        assert!(result.is_ok(), "execute should succeed: {result:?}");
    }
}
