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
    clippy::struct_excessive_bools,
    clippy::duration_suboptimal_units,
    clippy::incompatible_msrv,
    clippy::suspicious_map,
    clippy::unnecessary_map_or,
    clippy::let_unit_value,
    clippy::ignored_unit_patterns
)]

//! Benchmarks for daemon protocol request and response codecs.
use criterion::{Criterion, criterion_main};
use minibox_core::protocol::{
    ContainerInfo, DaemonRequest, DaemonResponse, decode_request, decode_response, encode_request,
    encode_response,
};
use std::hint::black_box;

fn small_run_request() -> DaemonRequest {
    minibox_macros::test_run!()
}

fn large_run_request() -> DaemonRequest {
    let command: Vec<String> = (0..24)
        .map(|i| format!("arg-{}-{}", i, "x".repeat(16)))
        .collect();
    minibox_macros::test_run!(
        image: "library/some-really-long-image-name-for-benchmarking".to_string(),
        tag: Some("2026.03.16-benchmarks".to_string()),
        command: command,
        memory_limit_bytes: Some(512 * 1024 * 1024),
        cpu_weight: Some(7500),
    )
}

fn small_pull_request() -> DaemonRequest {
    DaemonRequest::Pull {
        image: "alpine".to_string(),
        tag: None,
        platform: None,
    }
}

fn large_pull_request() -> DaemonRequest {
    DaemonRequest::Pull {
        image: "library/some-really-long-image-name-for-benchmarking".to_string(),
        tag: Some("2026.03.16-benchmarks".to_string()),
        platform: None,
    }
}

fn small_stop_request() -> DaemonRequest {
    DaemonRequest::Stop {
        id: "deadbeefdeadbeef".to_string(),
    }
}

fn large_stop_request() -> DaemonRequest {
    DaemonRequest::Stop {
        id: "deadbeefdeadbeefdeadbeefdeadbeef".to_string(),
    }
}

fn small_remove_request() -> DaemonRequest {
    DaemonRequest::Remove {
        id: "deadbeefdeadbeef".to_string(),
    }
}

fn large_remove_request() -> DaemonRequest {
    DaemonRequest::Remove {
        id: "deadbeefdeadbeefdeadbeefdeadbeef".to_string(),
    }
}

fn list_request() -> DaemonRequest {
    DaemonRequest::List
}

fn small_container_created_response() -> DaemonResponse {
    DaemonResponse::ContainerCreated {
        id: "deadbeefdeadbeef".to_string(),
    }
}

fn large_container_created_response() -> DaemonResponse {
    DaemonResponse::ContainerCreated {
        id: "deadbeefdeadbeefdeadbeefdeadbeef".to_string(),
    }
}

fn small_success_response() -> DaemonResponse {
    DaemonResponse::Success {
        message: "ok".to_string(),
    }
}

fn large_success_response() -> DaemonResponse {
    DaemonResponse::Success {
        message: "operation completed successfully with additional context".to_string(),
    }
}

fn small_error_response() -> DaemonResponse {
    DaemonResponse::Error {
        message: "error".to_string(),
    }
}

fn large_error_response() -> DaemonResponse {
    DaemonResponse::Error {
        message: "error: failed to perform operation due to invalid state".to_string(),
    }
}

fn small_container_list_response() -> DaemonResponse {
    DaemonResponse::ContainerList {
        containers: vec![make_container_info(0)],
    }
}

fn large_container_list_response() -> DaemonResponse {
    DaemonResponse::ContainerList {
        containers: (0..100).map(make_container_info).collect(),
    }
}

fn make_container_info(i: usize) -> ContainerInfo {
    ContainerInfo {
        id: format!("{:016x}", i),
        name: None,
        image: format!("library/image-{}", i),
        command: format!("echo hello {}", i),
        state: if i.is_multiple_of(2) {
            "running"
        } else {
            "stopped"
        }
        .to_string(),
        created_at: format!("2026-03-16T12:{:02}:00Z", i % 60),
        pid: Some(1000 + i as u32),
    }
}

fn bench_encode_request(c: &mut Criterion, name: &str, req: &DaemonRequest) {
    let bench_name = format!("protocol_encode_{}", name);
    c.bench_function(&bench_name, |b| {
        b.iter(|| {
            let encoded = encode_request(req).expect("encode request");
            black_box(encoded);
        })
    });
}

fn bench_decode_request(c: &mut Criterion, name: &str, req: &DaemonRequest) {
    let bench_name = format!("protocol_decode_{}", name);
    let encoded = encode_request(req).expect("encode request");
    c.bench_function(&bench_name, |b| {
        b.iter(|| {
            let decoded = decode_request(&encoded).expect("decode request");
            black_box(decoded);
        })
    });
}

fn bench_encode_response(c: &mut Criterion, name: &str, resp: &DaemonResponse) {
    let bench_name = format!("protocol_encode_{}", name);
    c.bench_function(&bench_name, |b| {
        b.iter(|| {
            let encoded = encode_response(resp).expect("encode response");
            black_box(encoded);
        })
    });
}

fn bench_decode_response(c: &mut Criterion, name: &str, resp: &DaemonResponse) {
    let bench_name = format!("protocol_decode_{}", name);
    let encoded = encode_response(resp).expect("encode response");
    c.bench_function(&bench_name, |b| {
        b.iter(|| {
            let decoded = decode_response(&encoded).expect("decode response");
            black_box(decoded);
        })
    });
}

fn bench_decode_invalid_request(c: &mut Criterion) {
    let encoded = b"{not-json\n";
    c.bench_function("protocol_decode_invalid_request", |b| {
        b.iter(|| {
            let decoded = decode_request(encoded);
            black_box(decoded.is_err());
        })
    });
}

fn bench_decode_invalid_response(c: &mut Criterion) {
    let encoded = b"{\"type\":\"Unknown\"}\n";
    c.bench_function("protocol_decode_invalid_response", |b| {
        b.iter(|| {
            let decoded = decode_response(encoded);
            black_box(decoded.is_err());
        })
    });
}

fn bench_requests(c: &mut Criterion) {
    let small_run = small_run_request();
    let large_run = large_run_request();
    let small_pull = small_pull_request();
    let large_pull = large_pull_request();
    let small_stop = small_stop_request();
    let large_stop = large_stop_request();
    let small_remove = small_remove_request();
    let large_remove = large_remove_request();
    let list = list_request();

    bench_encode_request(c, "run_small", &small_run);
    bench_decode_request(c, "run_small", &small_run);
    bench_encode_request(c, "run_large", &large_run);
    bench_decode_request(c, "run_large", &large_run);

    bench_encode_request(c, "pull_small", &small_pull);
    bench_decode_request(c, "pull_small", &small_pull);
    bench_encode_request(c, "pull_large", &large_pull);
    bench_decode_request(c, "pull_large", &large_pull);

    bench_encode_request(c, "stop_small", &small_stop);
    bench_decode_request(c, "stop_small", &small_stop);
    bench_encode_request(c, "stop_large", &large_stop);
    bench_decode_request(c, "stop_large", &large_stop);

    bench_encode_request(c, "remove_small", &small_remove);
    bench_decode_request(c, "remove_small", &small_remove);
    bench_encode_request(c, "remove_large", &large_remove);
    bench_decode_request(c, "remove_large", &large_remove);

    bench_encode_request(c, "list", &list);
    bench_decode_request(c, "list", &list);
}

fn bench_responses(c: &mut Criterion) {
    let small_created = small_container_created_response();
    let large_created = large_container_created_response();
    let small_success = small_success_response();
    let large_success = large_success_response();
    let small_error = small_error_response();
    let large_error = large_error_response();
    let small_list = small_container_list_response();
    let large_list = large_container_list_response();

    bench_encode_response(c, "container_created_small", &small_created);
    bench_decode_response(c, "container_created_small", &small_created);
    bench_encode_response(c, "container_created_large", &large_created);
    bench_decode_response(c, "container_created_large", &large_created);

    bench_encode_response(c, "success_small", &small_success);
    bench_decode_response(c, "success_small", &small_success);
    bench_encode_response(c, "success_large", &large_success);
    bench_decode_response(c, "success_large", &large_success);

    bench_encode_response(c, "error_small", &small_error);
    bench_decode_response(c, "error_small", &small_error);
    bench_encode_response(c, "error_large", &large_error);
    bench_decode_response(c, "error_large", &large_error);

    bench_encode_response(c, "container_list_small", &small_list);
    bench_decode_response(c, "container_list_small", &small_list);
    bench_encode_response(c, "container_list_large", &large_list);
    bench_decode_response(c, "container_list_large", &large_list);
}

minibox_bench::documented_criterion_group!(
    "Runs daemon protocol codec benchmarks.",
    protocol_codec,
    bench_requests,
    bench_responses,
    bench_decode_invalid_request,
    bench_decode_invalid_response,
);
criterion_main!(protocol_codec);
