//! Doctor tool implementation.

use crate::client::MiniboxDaemonClient;
use crate::types::{DoctorInput, DoctorOutput};
use minibox_core::protocol::DaemonRequest;

/// Check daemon socket connectivity.
// TODO(review): no #[cfg(test)] module in this file — doctor() is the tool an agent is
// expected to call first when debugging connectivity, but the daemon-unreachable path
// (socket missing/stale) is never exercised by a test anywhere in the crate.
pub async fn doctor(client: &MiniboxDaemonClient, _input: DoctorInput) -> DoctorOutput {
    match client.call(DaemonRequest::List).await {
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
