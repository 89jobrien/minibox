//! minibox-crux-plugin — crux JSON-RPC plugin for minibox container operations.
//!
//! Reads `Request` (newline-delimited JSON) from stdin, writes `Response` to stdout.

use anyhow::{Context, Result};
use minibox_crux_plugin::protocol::{Request, Response};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tracing::{debug, info};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env().add_directive(
                "minibox_crux_plugin=info"
                    .parse()
                    .expect("valid static directive"),
            ),
        )
        .init();

    info!("minibox-crux-plugin starting");

    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let mut reader = BufReader::new(stdin).lines();
    let mut writer = tokio::io::BufWriter::new(stdout);

    while let Some(line) = reader.next_line().await.context("read stdin")? {
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }

        debug!(line = %line, "received request");

        let request: Request = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(error = %e, "failed to parse request — skipping");
                continue;
            }
        };

        if matches!(request, Request::Shutdown) {
            info!("shutdown requested");
            let ack =
                serde_json::to_string(&Response::ShutdownAck).context("serialize ShutdownAck")?;
            writer
                .write_all(format!("{ack}\n").as_bytes())
                .await
                .context("write ShutdownAck")?;
            writer.flush().await.context("flush")?;
            break;
        }

        if let Some(response) = minibox_crux_plugin::process_request(request).await {
            let encoded = serde_json::to_string(&response).context("serialize response")?;
            writer
                .write_all(format!("{encoded}\n").as_bytes())
                .await
                .context("write response")?;
            writer.flush().await.context("flush response")?;
        }
    }

    info!("minibox-crux-plugin exiting");
    Ok(())
}
