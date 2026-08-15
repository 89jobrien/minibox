#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::doc_markdown,
    clippy::redundant_field_names,
    clippy::uninlined_format_args,
    clippy::redundant_clone,
    clippy::redundant_closure,
    clippy::single_char_pattern,
    clippy::unwrap_in_result,
    clippy::collapsible_if,
    clippy::match_same_arms,
    clippy::only_used_in_recursion,
    clippy::used_underscore_binding,
    clippy::map_unwrap_or,
    clippy::manual_assert,
    clippy::as_ptr_cast_mut,
    clippy::ptr_as_ptr,
    clippy::must_use_candidate,
    clippy::used_underscore_items,
    clippy::missing_const_for_fn,
    clippy::manual_string_new,
    clippy::semicolon_if_nothing_returned,
    clippy::unreadable_literal,
    clippy::default_constructed_unit_structs,
    clippy::ref_as_ptr,
    clippy::allow_attributes_without_reason,
    clippy::redundant_closure_for_method_calls,
    clippy::needless_raw_string_hashes,
    clippy::manual_is_variant_and,
    clippy::ignore_without_reason,
    clippy::default_trait_access,
    clippy::cast_lossless,
    clippy::match_wild_err_arm,
    clippy::format_push_string,
    clippy::bool_assert_comparison,
    clippy::struct_excessive_bools
)]
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

    let addr: SocketAddr = "127.0.0.1:0".parse().expect("unwrap in test");

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
