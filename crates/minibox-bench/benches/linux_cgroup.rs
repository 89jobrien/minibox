//! Cgroup v2 limiter lifecycle bench (Linux + root only).
//!
//! Benches `CgroupV2Limiter::create` + `cleanup` per iteration. `add_process`
//! is excluded: it needs a live child process (covered by the spawn bench).
//! On non-Linux targets the criterion group is a no-op so `--test` mode still
//! passes; on Linux without root the benches skip at runtime.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use criterion::criterion_main;

#[cfg(target_os = "linux")]
mod linux {
    use criterion::Criterion;
    use minibox::adapters::CgroupV2Limiter;
    use minibox::domain::{ResourceConfig, ResourceLimiter};
    use std::sync::atomic::{AtomicU64, Ordering};

    /// Monotonic counter for unique container ids (no randomness, no clock).
    static NEXT_ID: AtomicU64 = AtomicU64::new(0);

    fn next_container_id() -> String {
        format!(
            "bench-cgroup-{:06}",
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        )
    }

    fn create_cleanup(limiter: &CgroupV2Limiter, config: &ResourceConfig) {
        let id = next_container_id();
        limiter.create(&id, config).expect("create cgroup");
        limiter.cleanup(&id).expect("cleanup cgroup");
    }

    pub fn bench_cgroup(c: &mut Criterion) {
        if !minibox_bench::is_root() {
            eprintln!("SKIP: linux_cgroup benches require root (euid != 0)");
            return;
        }

        let limiter = CgroupV2Limiter::new();
        let limited = ResourceConfig {
            memory_limit_bytes: Some(64 << 20),
            cpu_weight: Some(100),
            pids_max: Some(64),
            io_max_bytes_per_sec: None,
        };
        let unlimited = ResourceConfig {
            memory_limit_bytes: None,
            cpu_weight: None,
            pids_max: None,
            io_max_bytes_per_sec: None,
        };

        let mut group = c.benchmark_group("linux_cgroup");
        group.bench_function("cgroup_create_cleanup", |b| {
            b.iter(|| create_cleanup(&limiter, &limited));
        });
        group.bench_function("cgroup_create_cleanup_unlimited", |b| {
            b.iter(|| create_cleanup(&limiter, &unlimited));
        });
        group.finish();
    }
}

#[cfg(target_os = "linux")]
use linux::bench_cgroup;

/// No-op on non-Linux targets so criterion `--test` mode passes everywhere.
#[cfg(not(target_os = "linux"))]
const fn bench_cgroup(_c: &mut criterion::Criterion) {}

minibox_bench::documented_criterion_group!("Runs Linux cgroup benchmarks.", benches, bench_cgroup);
criterion_main!(benches);
