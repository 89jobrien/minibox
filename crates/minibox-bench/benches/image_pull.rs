//! End-to-end image pull benchmarks.
//!
//! Drives `RegistryClient::pull_image` against a local wiremock OCI registry
//! (`BenchRegistry`): auth, manifest fetch, layer download, digest
//! verification, extraction, and manifest persistence. The mock server,
//! layer bytes, and client are set up once per scenario outside the timing
//! loop; each iteration pulls into a fresh `ImageStore` under a fresh
//! `TempDir`.
//!
//! Run with: `cargo bench -p minibox-bench --bench image_pull`

// Bench setup code: panicking on setup failure is the correct behaviour.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use criterion::{BatchSize, Criterion, SamplingMode, criterion_main};
use minibox_bench::fixtures::{BenchRegistry, LayerSpec, build_layer_tar_gz};
use minibox_core::ImageStore;
use std::hint::black_box;
use tempfile::TempDir;
use tokio::runtime::Runtime;

const IMAGE: &str = "bench/img";
const TAG: &str = "latest";
const KIB: usize = 1024;
const MIB: usize = 1024 * 1024;

/// A ~4 MiB layer; `dir_depth` varies the content so multi-layer scenarios
/// get distinct digests from otherwise identical shapes.
const fn layer_4mib(dir_depth: usize) -> LayerSpec {
    LayerSpec {
        file_count: 64,
        file_size_bytes: 64 * KIB,
        dir_depth,
    }
}

/// Scenario grid: name -> layer shapes (see plan Task 8).
fn scenarios() -> Vec<(&'static str, Vec<LayerSpec>)> {
    vec![
        ("pull_1_layer_4mib", vec![layer_4mib(2)]),
        ("pull_4_layers_4mib", (1..=4).map(layer_4mib).collect()),
        (
            "pull_1_layer_32mib",
            vec![LayerSpec {
                file_count: 4,
                file_size_bytes: 8 * MIB,
                dir_depth: 1,
            }],
        ),
    ]
}

fn bench_image_pull(c: &mut Criterion) {
    let rt = Runtime::new().expect("tokio runtime");
    let mut group = c.benchmark_group("image_pull");
    // Each iteration is a full HTTP pull (tens of ms): flat sampling with the
    // minimum sample count keeps the full suite runtime bounded.
    group.sampling_mode(SamplingMode::Flat);
    group.sample_size(10);

    for (name, specs) in scenarios() {
        let layers: Vec<Vec<u8>> = specs.iter().map(build_layer_tar_gz).collect();
        // Wiremock server + client live for the whole scenario, outside the
        // timing loop. The server task runs on `rt`, which outlives the group.
        let registry = rt
            .block_on(BenchRegistry::serve(IMAGE, TAG, layers))
            .expect("serve bench registry");
        let client = registry.client().expect("bench registry client");

        group.bench_function(name, |b| {
            b.to_async(&rt).iter_batched(
                || {
                    // Untimed: fresh store per iteration; clone the client
                    // (cheap Arc bumps) so the future is self-contained.
                    let tmp = TempDir::new().expect("temp dir for image store");
                    let store =
                        ImageStore::new(tmp.path().join("images")).expect("bench image store");
                    (tmp, store, client.clone())
                },
                |(tmp, store, client)| async move {
                    client
                        .pull_image(IMAGE, TAG, &store)
                        .await
                        .expect("pull_image against bench registry");
                    // Return the TempDir so its cleanup happens outside the
                    // timed section (criterion drops routine output lazily).
                    black_box(tmp)
                },
                BatchSize::PerIteration,
            );
        });
    }

    group.finish();
}

minibox_bench::documented_criterion_group!(
    "Runs OCI image pull benchmarks.",
    image_pull,
    bench_image_pull
);
criterion_main!(image_pull);
