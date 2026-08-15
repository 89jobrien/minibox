//! `mcp` — stdio MCP server for minibox.

use mcp::MiniboxMcpServer;
use miette::{IntoDiagnostic as _, WrapErr as _};
use rmcp::{ServiceExt, transport::stdio};

#[tokio::main]
async fn main() -> miette::Result<()> {
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("mcp=info"));
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(env_filter)
        .init();

    let service = MiniboxMcpServer::from_env()
        .serve(stdio())
        .await
        .into_diagnostic()
        .wrap_err("start minibox MCP stdio server")?;
    service
        .waiting()
        .await
        .into_diagnostic()
        .wrap_err("run minibox MCP server")?;

    Ok(())
}
