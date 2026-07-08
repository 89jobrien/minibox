//! Overlay rootfs setup/cleanup benchmarks (Linux, root-only).
//!
//! Measures a full `OverlayFilesystem::setup_rootfs` + `cleanup` cycle over
//! pre-extracted fixture layers. Layers are extracted once, outside the
//! timing loop, into per-layer directories under the adapter's images base
//! (required by `validate_layer_path`); each iteration mounts the overlay
//! into a fresh container directory and unmounts it again.
//!
//! Non-Linux platforms register a no-op bench so criterion `--test` mode
//! passes everywhere. On Linux without root the benches skip at runtime.
//!
//! Run with: `cargo bench -p minibox-bench --bench linux_rootfs` (as root)

// Bench setup code: panicking on setup failure is the correct behaviour.
#![allow(clippy::unwrap_used, clippy::expect_used)]

#[cfg(not(target_os = "linux"))]
use criterion::Criterion;
use criterion::{criterion_group, criterion_main};

#[cfg(target_os = "linux")]
mod real {
    use criterion::{BatchSize, Criterion, SamplingMode};
    use minibox::adapters::OverlayFilesystem;
    use minibox_bench::fixtures::{LayerSpec, build_layer_tar_gz};
    use minibox_core::domain::RootfsSetup;
    use minibox_core::image::layer::extract_layer;
    use std::hint::black_box;
    use std::path::PathBuf;
    use tempfile::TempDir;

    const KIB: usize = 1024;

    /// Layer-stack depths to measure (plan Task 9: 1, 4, 16).
    const LAYER_COUNTS: &[usize] = &[1, 4, 16];

    /// Shape of each synthetic layer: modest tree so the mount/unmount cycle
    /// dominates rather than fixture extraction size.
    const LAYER_SPEC: LayerSpec = LayerSpec {
        file_count: 64,
        file_size_bytes: 4 * KIB,
        dir_depth: 2,
    };

    /// Extract `count` fixture layers into per-layer dirs under `images_base`.
    ///
    /// `validate_layer_path` requires every lowerdir to canonicalize inside
    /// the adapter's images base, so the layers must live under it.
    fn extract_layers(images_base: &TempDir, count: usize) -> Vec<PathBuf> {
        let bytes = build_layer_tar_gz(&LAYER_SPEC);
        (0..count)
            .map(|i| {
                let dir = images_base.path().join(format!("layer-{i:02}"));
                std::fs::create_dir_all(&dir).expect("create layer dir");
                let mut reader = bytes.as_slice();
                extract_layer(&mut reader, &dir).expect("extract fixture layer");
                dir
            })
            .collect()
    }

    pub fn bench_linux_rootfs(c: &mut Criterion) {
        if !minibox_bench::is_root() {
            eprintln!("SKIP: linux_rootfs benches require root; benching nothing");
            return;
        }

        // Extract the deepest stack once; shallower scenarios reuse prefixes.
        let images_base = TempDir::new().expect("images base temp dir");
        let layer_dirs = extract_layers(&images_base, 16);

        // Parent for per-iteration container dirs.
        let containers_base = TempDir::new().expect("containers base temp dir");
        let fs = OverlayFilesystem::new_with_base(images_base.path());

        let mut group = c.benchmark_group("linux_rootfs");
        // Iterations are mount/unmount syscall work: flat sampling with a
        // small sample count keeps the full suite runtime bounded.
        group.sampling_mode(SamplingMode::Flat);
        group.sample_size(20);

        for &count in LAYER_COUNTS {
            let layers: Vec<PathBuf> = layer_dirs[..count].to_vec();

            group.bench_function(format!("setup_cleanup_{count}_layers"), |b| {
                b.iter_batched(
                    // Fresh container dir per iteration, created outside the
                    // timed section.
                    || TempDir::new_in(containers_base.path()).expect("container dir"),
                    |container| {
                        let layout = fs
                            .setup_rootfs(&layers, container.path())
                            .expect("setup overlay rootfs");
                        black_box(&layout);
                        fs.cleanup(container.path())
                            .expect("cleanup overlay rootfs");
                        // Return the TempDir so its removal happens outside
                        // the timed section.
                        black_box(container)
                    },
                    BatchSize::PerIteration,
                );
            });
        }

        group.finish();
    }
}

#[cfg(target_os = "linux")]
use real::bench_linux_rootfs;

/// No-op on non-Linux platforms so criterion `--test` mode still passes.
#[cfg(not(target_os = "linux"))]
fn bench_linux_rootfs(_c: &mut Criterion) {}

criterion_group!(linux_rootfs, bench_linux_rootfs);
criterion_main!(linux_rootfs);
