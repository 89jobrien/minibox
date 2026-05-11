//! `mbx manifest` and `mbx verify` — execution manifest inspection and
//! policy verification.

use anyhow::Context;
use minibox_core::client::DaemonClient;
use minibox_core::protocol::{DaemonRequest, DaemonResponse};
use std::path::Path;

/// Print the execution manifest for a container as pretty JSON.
pub async fn execute(id: String, socket_path: &Path) -> anyhow::Result<()> {
    let request = DaemonRequest::GetManifest { id };

    let client = DaemonClient::with_socket(socket_path);
    let mut stream = client
        .call(request)
        .await
        .context("failed to call daemon")?;

    if let Some(response) = stream.next().await.context("stream error")? {
        match response {
            DaemonResponse::Manifest { manifest } => {
                let pretty =
                    serde_json::to_string_pretty(&manifest).context("format manifest JSON")?;
                println!("{pretty}");
            }
            DaemonResponse::Error { message } => {
                anyhow::bail!("{message}");
            }
            other => {
                anyhow::bail!("unexpected response: {other:?}");
            }
        }
    }

    Ok(())
}

/// Verify a container's manifest against a policy file.
pub async fn verify(id: String, policy_path: String, socket_path: &Path) -> anyhow::Result<()> {
    let policy_json = std::fs::read_to_string(&policy_path)
        .with_context(|| format!("read policy file: {policy_path}"))?;

    let request = DaemonRequest::VerifyManifest { id, policy_json };

    let client = DaemonClient::with_socket(socket_path);
    let mut stream = client
        .call(request)
        .await
        .context("failed to call daemon")?;

    if let Some(response) = stream.next().await.context("stream error")? {
        match response {
            DaemonResponse::VerifyResult { allowed, reason } => {
                if allowed {
                    println!("ALLOWED");
                } else {
                    let reason = reason.unwrap_or_else(|| "no reason given".to_string());
                    println!("DENIED: {reason}");
                    std::process::exit(1);
                }
            }
            DaemonResponse::Error { message } => {
                anyhow::bail!("{message}");
            }
            other => {
                anyhow::bail!("unexpected response: {other:?}");
            }
        }
    }

    Ok(())
}
