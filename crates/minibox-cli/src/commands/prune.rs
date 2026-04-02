//! `minibox prune [--dry-run]` — remove unused images.
use anyhow::Context;
use minibox_client::DaemonClient;
use minibox_core::protocol::{DaemonRequest, DaemonResponse};
use std::path::Path;

pub async fn execute(dry_run: bool, socket_path: &Path) -> anyhow::Result<()> {
    let client = DaemonClient::with_socket(socket_path);
    let mut stream = client
        .call(DaemonRequest::Prune { dry_run })
        .await
        .context("failed to call daemon")?;

    if let Some(response) = stream.next().await.context("stream error")? {
        match response {
            DaemonResponse::Pruned {
                removed,
                freed_bytes,
                dry_run,
            } => {
                let prefix = if dry_run { "[dry-run] " } else { "" };
                for r in &removed {
                    println!("{prefix}Deleted: {r}");
                }
                let freed_mb = freed_bytes as f64 / 1_048_576.0;
                println!(
                    "{prefix}Total freed: {freed_mb:.1} MB ({} image{})",
                    removed.len(),
                    if removed.len() == 1 { "" } else { "s" }
                );
                Ok(())
            }
            DaemonResponse::Error { message } => {
                eprintln!("error: {message}");
                std::process::exit(1);
            }
            other => {
                eprintln!("unexpected response: {other:?}");
                std::process::exit(1);
            }
        }
    } else {
        eprintln!("no response from daemon");
        std::process::exit(1);
    }
}
