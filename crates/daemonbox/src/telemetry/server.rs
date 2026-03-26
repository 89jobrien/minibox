//! Prometheus metrics HTTP server.
//!
//! Exposes a `/metrics` endpoint that returns Prometheus text exposition format.
//! Spawned as a separate Tokio task from the composition root.

use super::PrometheusMetricsRecorder;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::task::JoinHandle;

/// Start the metrics HTTP server.
///
/// Takes an `Arc<PrometheusMetricsRecorder>` — the same instance injected into
/// handlers — and encodes its registry on each `/metrics` request.
///
/// Returns the actual bound address (useful when port 0 is used in tests)
/// and a `JoinHandle` for the server task.
pub async fn run_metrics_server(
    bind_addr: SocketAddr,
    recorder: Arc<PrometheusMetricsRecorder>,
) -> anyhow::Result<(SocketAddr, JoinHandle<()>)> {
    use axum::routing::get;

    let app = axum::Router::new().route(
        "/metrics",
        get(move || {
            let recorder = recorder.clone();
            async move { recorder.encode_metrics() }
        }),
    );

    let listener = tokio::net::TcpListener::bind(bind_addr)
        .await
        .map_err(|e| anyhow::anyhow!("metrics server bind {bind_addr}: {e}"))?;
    let actual_addr = listener.local_addr()?;

    let handle = tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            tracing::error!(error = %e, "metrics server exited with error");
        }
    });

    Ok((actual_addr, handle))
}
