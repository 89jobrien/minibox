//! Doctor tool implementation.

use crate::client::MiniboxDaemonClient;
use crate::policy::AgentPolicy;
use crate::types::{DoctorInput, DoctorOutput};
use minibox_core::protocol::DaemonRequest;

/// Check daemon socket connectivity.
pub async fn doctor(
    client: &MiniboxDaemonClient,
    policy: &AgentPolicy,
    _input: DoctorInput,
) -> DoctorOutput {
    match client
        .call_limited(DaemonRequest::List, policy.max_output_bytes)
        .await
    {
        Ok(_) => DoctorOutput {
            socket_path: client.socket_path.display().to_string(),
            connected: true,
            error: None,
        },
        Err(error) => DoctorOutput {
            socket_path: client.socket_path.display().to_string(),
            connected: false,
            error: Some(error.to_string()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[tokio::test]
    async fn doctor_reports_unreachable_daemon() {
        let socket = PathBuf::from("/nonexistent/minibox.sock");
        let client = MiniboxDaemonClient::new(socket.clone());
        let policy = AgentPolicy::safe_default();

        let output = doctor(&client, &policy, DoctorInput::default()).await;

        assert_eq!(output.socket_path, socket.display().to_string());
        assert!(!output.connected);
        assert!(output.error.is_some());
    }
}
