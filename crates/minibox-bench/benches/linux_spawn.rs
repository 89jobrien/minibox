//! Container spawn lifecycle bench (Linux + root only).
//!
//! Benches `LinuxNamespaceRuntime::spawn_process` + `wait_for_exit` for a
//! trivial `/bin/true` workload: clone with new namespaces, cgroup attach,
//! `pivot_root`, `execve`, and reap. Setup (rootfs build, cgroup create) happens
//! once outside the timing loop; each iteration is one full spawn+wait.
//!
//! The bench rootfs is built by copying the host's `/bin/true` and its
//! ldd-reported dynamic libraries (no-op for static/busybox `true`), mirroring
//! the minimal-rootfs approach of the Linux isolation tests. The cgroup dir is
//! created via `CgroupV2Limiter` exactly as `native_adapter_isolation_tests.rs`
//! does.
//!
//! On non-Linux targets the criterion group is a no-op so `--test` mode still
//! passes; on Linux without root the benches skip at runtime.
//!
//! Run with: `cargo bench -p minibox-bench --bench linux_spawn`

// Bench setup code: panicking on setup failure is the correct behaviour.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use criterion::criterion_main;

#[cfg(target_os = "linux")]
mod linux {
    use criterion::{Criterion, SamplingMode};
    use minibox::adapters::{CgroupV2Limiter, LinuxNamespaceRuntime};
    use minibox::domain::{
        ContainerHooks, ContainerRuntime, ContainerSpawnConfig, ResourceConfig, ResourceLimiter,
    };
    use std::fs;
    use std::hint::black_box;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use tempfile::TempDir;
    use tokio::runtime::Runtime;

    /// Absolute library paths `ldd` reports for `binary`.
    ///
    /// Empty when the binary is statically linked (busybox-style `true`) or
    /// `ldd` is unavailable — in both cases the bare binary is sufficient.
    fn dynamic_deps(binary: &Path) -> Vec<PathBuf> {
        let Ok(out) = Command::new("ldd").arg(binary).output() else {
            return Vec::new();
        };
        if !out.status.success() {
            return Vec::new();
        }
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|line| {
                line.split_whitespace()
                    .find(|tok| tok.starts_with('/'))
                    .map(PathBuf::from)
            })
            .filter(|path| path.exists())
            .collect()
    }

    /// Build a minimal rootfs able to exec `/bin/true`: the host binary plus
    /// its dynamic libraries (and ELF interpreter) at their original paths.
    fn build_true_rootfs(root: &Path) {
        let host_true = Path::new("/bin/true");
        let bin_dir = root.join("bin");
        fs::create_dir_all(&bin_dir).expect("create rootfs bin dir");
        // fs::copy follows symlinks (usr-merge, busybox) and preserves mode.
        fs::copy(host_true, bin_dir.join("true")).expect("copy /bin/true into rootfs");

        for lib in dynamic_deps(host_true) {
            let rel = lib
                .strip_prefix("/")
                .expect("ldd-reported paths are absolute");
            let dest = root.join(rel);
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent).expect("create rootfs lib dir");
            }
            fs::copy(&lib, &dest).expect("copy library into rootfs");
        }
    }

    pub fn bench_spawn(c: &mut Criterion) {
        if !minibox_bench::is_root() {
            eprintln!("SKIP: linux_spawn benches require root (euid != 0)");
            return;
        }

        // Untimed setup: rootfs with a runnable /bin/true, one real cgroup
        // dir (the child attaches itself per spawn; reaping detaches it).
        let tmp = TempDir::new().expect("temp dir for bench rootfs");
        let rootfs = tmp.path().join("rootfs");
        build_true_rootfs(&rootfs);

        let limiter = CgroupV2Limiter::new();
        let cgroup_id = format!("bench-spawn-{}", std::process::id());
        let cgroup_path = limiter
            .create(&cgroup_id, &ResourceConfig::default())
            .expect("create bench cgroup");

        let runtime = LinuxNamespaceRuntime::new();
        let config = ContainerSpawnConfig {
            rootfs: rootfs.clone().into(),
            command: "/bin/true".to_string(),
            args: vec![],
            env: vec![],
            hostname: "bench".to_string(),
            cgroup_path: PathBuf::from(&cgroup_path).into(),
            capture_output: false,
            hooks: ContainerHooks::default(),
            skip_network_namespace: true,
            mounts: vec![],
            privileged: false,
            image_ref: None,
        };

        let rt = Runtime::new().expect("tokio runtime");
        let mut group = c.benchmark_group("linux_spawn");
        // Each iteration is a full clone/pivot_root/execve/wait cycle (ms
        // scale): flat sampling with the minimum sample count keeps the
        // suite runtime bounded.
        group.sampling_mode(SamplingMode::Flat);
        group.sample_size(10);

        group.bench_function("spawn_wait_true", |b| {
            b.to_async(&rt).iter(|| async {
                let spawn = runtime
                    .spawn_process(&config)
                    .await
                    .expect("spawn /bin/true in bench rootfs");
                let code = runtime
                    .wait_for_exit(None, spawn.pid)
                    .await
                    .expect("wait for /bin/true exit");
                assert_eq!(code, 0, "/bin/true must exit 0 — broken bench rootfs?");
                black_box(code)
            });
        });

        group.finish();
        limiter.cleanup(&cgroup_id).expect("cleanup bench cgroup");
    }
}

#[cfg(target_os = "linux")]
use linux::bench_spawn;

/// No-op on non-Linux targets so criterion `--test` mode passes everywhere.
#[cfg(not(target_os = "linux"))]
const fn bench_spawn(_c: &mut criterion::Criterion) {}

minibox_bench::documented_criterion_group!(
    "Runs Linux container spawn benchmarks.",
    benches,
    bench_spawn
);
criterion_main!(benches);
