#![cfg(feature = "metrics")]
use minibox::daemon::telemetry::PrometheusMetricsRecorder;
use minibox::daemon::telemetry::server::run_metrics_server;
use minibox_core::domain::MetricsRecorder;
use std::net::SocketAddr;
use std::sync::Arc;

#[tokio::test]
async fn metrics_endpoint_returns_prometheus_format() {
    let recorder = Arc::new(PrometheusMetricsRecorder::new());
    recorder.increment_counter("test_counter_total", &[("label", "value")]);

    let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();

    let (actual_addr, server_handle) = run_metrics_server(addr, recorder)
        .await
        .expect("server start");

    let url = format!("http://{actual_addr}/metrics");
    let body = reqwest::get(&url)
        .await
        .expect("GET /metrics")
        .text()
        .await
        .expect("body");

    assert!(
        body.contains("test_counter_total"),
        "body should contain metric name; got:\n{body}"
    );

    server_handle.abort();
}
