//! Image-related MCP tool implementations.

use crate::client::MiniboxDaemonClient;
use crate::error::{McpServerError, Result};
use crate::policy::AgentPolicy;
use crate::types::{ImagesOutput, PullImageInput, PullImageOutput, require_non_empty};
use minibox_core::protocol::{DaemonRequest, DaemonResponse};

/// List cached images.
///
/// # Errors
///
/// Returns an error if the daemon call fails or returns an unexpected response.
pub async fn list_images(
    client: &MiniboxDaemonClient,
    policy: &AgentPolicy,
) -> Result<ImagesOutput> {
    let result = client
        .call_limited(DaemonRequest::ListImages, policy.max_output_bytes)
        .await?;
    result
        .responses
        .into_iter()
        .find_map(|response| match response {
            DaemonResponse::ImageList { images } => Some(ImagesOutput { images }),
            _ => None,
        })
        .ok_or_else(|| McpServerError::UnexpectedResponse {
            tool: "minibox_images",
            response: format!("{:?}", result.raw_responses),
        })
}

/// Pull an image.
///
/// # Errors
///
/// Returns an error if the input is invalid, daemon call fails, or response is unexpected.
pub async fn pull_image(
    client: &MiniboxDaemonClient,
    policy: &AgentPolicy,
    input: PullImageInput,
) -> Result<PullImageOutput> {
    // Pull mutates daemon state (network fetch + disk write), so it sits
    // behind the same mutation gate as stop/rm.
    policy.validate_mutation("minibox_pull")?;
    require_non_empty(&input.image, "image")?;

    let result = client
        .call_limited(
            DaemonRequest::Pull {
                image: input.image,
                tag: input.tag,
                platform: input.platform,
            },
            policy.max_output_bytes,
        )
        .await?;
    let message = result
        .responses
        .iter()
        .find_map(|response| match response {
            DaemonResponse::Success { message } => Some(message.clone()),
            _ => None,
        })
        .ok_or_else(|| McpServerError::UnexpectedResponse {
            tool: "minibox_pull",
            response: format!("{:?}", result.raw_responses),
        })?;

    Ok(PullImageOutput {
        message,
        daemon_responses: result.raw_responses,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::PullImageInput;
    use std::path::PathBuf;

    #[tokio::test]
    async fn pull_is_denied_by_default_before_any_daemon_contact() {
        let client = MiniboxDaemonClient::new(PathBuf::from("/nonexistent/minibox.sock"));
        let policy = AgentPolicy::safe_default();
        let input = PullImageInput {
            image: "alpine".to_string(),
            ..PullImageInput::default()
        };

        // A PolicyDenied error (not DaemonConnection) proves the gate fires
        // before the unreachable socket is ever touched.
        assert!(matches!(
            pull_image(&client, &policy, input).await,
            Err(McpServerError::PolicyDenied {
                tool: "minibox_pull",
                ..
            })
        ));
    }
}
