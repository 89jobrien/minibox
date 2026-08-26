//! Layer extraction hot-path benchmarks.
//!
//! Measures `minibox_core::image::layer::extract_layer` over synthetic
//! gzipped tar layers of varying shape. Layer bytes are built once outside
//! the timing loop; each iteration extracts into a fresh `TempDir` so no
//! iteration observes another's filesystem state.
//!
//! Run with: `cargo bench -p minibox-bench --bench layer_extract`

// Bench setup code: panicking on setup failure is the correct behaviour.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use criterion::{BatchSize, Criterion, SamplingMode, criterion_main};
use minibox_bench::fixtures::{LayerSpec, build_layer_tar_gz};
use minibox_core::image::layer::extract_layer;
use std::hint::black_box;
use tempfile::TempDir;

const KIB: usize = 1024;
const MIB: usize = 1024 * 1024;

/// Scenario grid: name -> layer shape (see plan Task 7).
const SCENARIOS: &[(&str, LayerSpec)] = &[
    (
        // Many small files in a shallow tree (~4 MiB total).
        "extract_small_many",
        LayerSpec {
            file_count: 1024,
            file_size_bytes: 4 * KIB,
            dir_depth: 4,
        },
    ),
    (
        // Few large files (~32 MiB total): dominated by payload I/O.
        "extract_large_few",
        LayerSpec {
            file_count: 4,
            file_size_bytes: 8 * MIB,
            dir_depth: 1,
        },
    ),
    (
        // Deep directory nesting (~8 MiB total): path handling cost.
        "extract_deep_tree",
        LayerSpec {
            file_count: 512,
            file_size_bytes: 16 * KIB,
            dir_depth: 16,
        },
    ),
];

fn bench_layer_extract(c: &mut Criterion) {
    let mut group = c.benchmark_group("layer_extract");
    // Iterations are milliseconds-scale filesystem work: flat sampling with a
    // small sample count keeps the full suite runtime bounded.
    group.sampling_mode(SamplingMode::Flat);
    group.sample_size(20);

    for (name, spec) in SCENARIOS {
        // Build the layer bytes once, outside the timing loop.
        let bytes = build_layer_tar_gz(spec);

        group.bench_function(*name, |b| {
            b.iter_batched(
                || TempDir::new().expect("temp dir for extraction dest"),
                |dest| {
                    let mut reader = bytes.as_slice();
                    extract_layer(&mut reader, dest.path()).expect("extract fixture layer");
                    // Return the TempDir so its cleanup happens outside the
                    // timed section (criterion drops routine output lazily).
                    black_box(dest)
                },
                BatchSize::PerIteration,
            );
        });
    }

    group.finish();
}

minibox_bench::documented_criterion_group!(
    "Runs OCI layer extraction benchmarks.",
    layer_extract,
    bench_layer_extract
);
criterion_main!(layer_extract);
