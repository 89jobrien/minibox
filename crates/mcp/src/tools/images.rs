//! Image-related MCP tool implementations.

use crate::client::MiniboxDaemonClient;
use crate::error::{McpServerError, Result};
use crate::policy::AgentPolicy;
use crate::types::{ImagesOutput, PullImageInput, PullImageOutput};
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
    // TODO(review): pull mutates daemon state (network fetch + disk write) but calls no
    // policy check at all, unlike stop/rm which require MINIBOX_MCP_ALLOW_MUTATION. Gate
    // this behind validate_mutation("minibox_pull") or an explicit pull permission tier.
    if input.image.trim().is_empty() {
        return Err(McpServerError::InvalidInput(
            "image must not be empty".to_string(),
        ));
    }

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
