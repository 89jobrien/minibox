//! `minibox rmi <image>` — remove a specific image.

use minibox_core::protocol::DaemonRequest;
use std::path::Path;

pub async fn execute(image_ref: String, socket_path: &Path) -> anyhow::Result<()> {
    super::send_request(DaemonRequest::RemoveImage { image_ref }, socket_path).await
}
