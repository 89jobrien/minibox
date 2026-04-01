//! Native adapter isolation tests (#22).
//!
//! Tests `OverlayFilesystem`, `CgroupV2Limiter`, and `LinuxNamespaceRuntime`
//! against real kernel infrastructure.
//!
//! **Linux only** — all tests are gated on `cfg(target_os = "linux")` and
//! skip gracefully via `require_capability!` if the host lacks the necessary
//! privileges or kernel features.
//!
//! Run via `just test-integration` (needs Linux + root + cgroup v2 slice).
//! On macOS use `just test-vz-isolation` which drives equivalent behavioral
//! assertions through an in-VM miniboxd agent (`macbox/tests/vz_isolation_tests.rs`).

#![cfg(target_os = "linux")]

use mbx::adapters::{CgroupV2Limiter, OverlayFilesystem};
use mbx::domain::{FilesystemProvider, ResourceConfig, ResourceLimiter};
use mbx::preflight::probe_capabilities;
use minibox_macros::require_capability;
use std::fs;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Populate `dir` with a minimal fake image layer (`bin/sh` empty file).
fn fake_layer(dir: &std::path::Path) {
    fs::create_dir_all(dir.join("bin")).unwrap();
    fs::write(dir.join("bin").join("sh"), b"").unwrap();
}

// ---------------------------------------------------------------------------
// OverlayFilesystem
// ---------------------------------------------------------------------------

#[test]
fn overlay_setup_creates_merged_upper_work_dirs() {
    let caps = probe_capabilities();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, has_overlay_fs, "requires overlay FS");

    let tmp = TempDir::new().unwrap();
    let layer = tmp.path().join("layer0");
    fake_layer(&layer);

    let container_dir = tmp.path().join("container");
    fs::create_dir_all(&container_dir).unwrap();

    let fs_adapter = OverlayFilesystem::new_with_base(tmp.path());
    let merged = fs_adapter
        .setup_rootfs(&[layer], &container_dir)
        .expect("setup_rootfs failed");

    assert!(merged.exists(), "merged dir must exist");
    assert!(container_dir.join("upper").exists(), "upper dir must exist");
    assert!(container_dir.join("work").exists(), "work dir must exist");
    assert!(
        merged.join("bin").join("sh").exists(),
        "layer content visible in merged"
    );
}

#[test]
fn overlay_write_goes_to_upper_not_lower() {
    let caps = probe_capabilities();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, has_overlay_fs, "requires overlay FS");

    let tmp = TempDir::new().unwrap();
    let layer = tmp.path().join("layer0");
    fake_layer(&layer);

    let container_dir = tmp.path().join("container");
    fs::create_dir_all(&container_dir).unwrap();

    let fs_adapter = OverlayFilesystem::new_with_base(tmp.path());
    let merged = fs_adapter
        .setup_rootfs(&[layer.clone()], &container_dir)
        .unwrap();

    fs::write(merged.join("newfile"), b"hello").unwrap();

    assert!(
        container_dir.join("upper").join("newfile").exists(),
        "write must land in upper"
    );
    assert!(
        !layer.join("newfile").exists(),
        "lower layer must be unmodified"
    );

    fs_adapter.cleanup(&container_dir).unwrap();
}

#[test]
fn overlay_multiple_layers_all_visible_in_merged() {
    let caps = probe_capabilities();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, has_overlay_fs, "requires overlay FS");

    let tmp = TempDir::new().unwrap();

    let layer0 = tmp.path().join("layer0");
    fs::create_dir_all(layer0.join("etc")).unwrap();
    fs::write(layer0.join("etc").join("os-release"), b"ID=test").unwrap();

    let layer1 = tmp.path().join("layer1");
    fs::create_dir_all(layer1.join("usr").join("bin")).unwrap();
    fs::write(layer1.join("usr").join("bin").join("env"), b"").unwrap();

    let container_dir = tmp.path().join("container");
    fs::create_dir_all(&container_dir).unwrap();

    let fs_adapter = OverlayFilesystem::new_with_base(tmp.path());
    let merged = fs_adapter
        .setup_rootfs(&[layer0, layer1], &container_dir)
        .unwrap();

    assert!(
        merged.join("etc").join("os-release").exists(),
        "layer0 content must be visible"
    );
    assert!(
        merged.join("usr").join("bin").join("env").exists(),
        "layer1 content must be visible"
    );

    fs_adapter.cleanup(&container_dir).unwrap();
}

#[test]
fn overlay_cleanup_unmounts_merged() {
    let caps = probe_capabilities();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, has_overlay_fs, "requires overlay FS");

    let tmp = TempDir::new().unwrap();
    let layer = tmp.path().join("layer0");
    fake_layer(&layer);

    let container_dir = tmp.path().join("container");
    fs::create_dir_all(&container_dir).unwrap();

    let fs_adapter = OverlayFilesystem::new_with_base(tmp.path());
    let merged = fs_adapter.setup_rootfs(&[layer], &container_dir).unwrap();
    assert!(merged.exists());

    fs_adapter.cleanup(&container_dir).unwrap();

    // After unmount the dir may still exist but must be empty (not mounted).
    if merged.exists() {
        let entries: Vec<_> = fs::read_dir(&merged).unwrap().collect();
        assert!(entries.is_empty(), "merged must be empty after unmount");
    }
}

#[test]
fn overlay_empty_layers_returns_error() {
    let caps = probe_capabilities();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, has_overlay_fs, "requires overlay FS");

    let tmp = TempDir::new().unwrap();
    let container_dir = tmp.path().join("container");
    fs::create_dir_all(&container_dir).unwrap();

    let fs_adapter = OverlayFilesystem::new_with_base(tmp.path());
    assert!(
        fs_adapter.setup_rootfs(&[], &container_dir).is_err(),
        "empty layer list must fail"
    );
}

// ---------------------------------------------------------------------------
// CgroupV2Limiter
// ---------------------------------------------------------------------------

#[test]
fn cgroup_create_and_cleanup_lifecycle() {
    let caps = probe_capabilities();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, has_cgroup_v2, "requires cgroup v2");

    let id = format!("test-isolation-create-{}", std::process::id());
    let limiter = CgroupV2Limiter::new();

    let path_str = limiter
        .create(&id, &ResourceConfig::default())
        .expect("create failed");
    let path = std::path::PathBuf::from(&path_str);

    assert!(path.exists(), "cgroup dir must exist after create");
    assert!(
        path.join("cgroup.procs").exists(),
        "cgroup.procs must exist"
    );

    limiter.cleanup(&id).unwrap();
    assert!(!path.exists(), "cgroup dir must be removed after cleanup");
}

#[test]
fn cgroup_memory_limit_written_correctly() {
    let caps = probe_capabilities();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, has_cgroup_v2, "requires cgroup v2");

    let id = format!("test-isolation-mem-{}", std::process::id());
    let limiter = CgroupV2Limiter::new();
    let limit: u64 = 128 * 1024 * 1024;

    let path_str = limiter
        .create(
            &id,
            &ResourceConfig {
                memory_limit_bytes: Some(limit),
                ..ResourceConfig::default()
            },
        )
        .unwrap();
    let path = std::path::PathBuf::from(path_str);

    let mem_max = path.join("memory.max");
    if mem_max.exists() {
        let content = fs::read_to_string(&mem_max).unwrap();
        assert_eq!(content.trim(), limit.to_string(), "memory.max mismatch");
    }

    limiter.cleanup(&id).unwrap();
}

#[test]
fn cgroup_cpu_weight_written_correctly() {
    let caps = probe_capabilities();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, has_cgroup_v2, "requires cgroup v2");

    let id = format!("test-isolation-cpu-{}", std::process::id());
    let limiter = CgroupV2Limiter::new();

    let path_str = limiter
        .create(
            &id,
            &ResourceConfig {
                cpu_weight: Some(500),
                ..ResourceConfig::default()
            },
        )
        .unwrap();
    let path = std::path::PathBuf::from(path_str);

    let cpu_weight = path.join("cpu.weight");
    if cpu_weight.exists() {
        let content = fs::read_to_string(&cpu_weight).unwrap();
        assert_eq!(content.trim(), "500", "cpu.weight mismatch");
    }

    limiter.cleanup(&id).unwrap();
}

#[test]
fn cgroup_pids_max_written_correctly() {
    let caps = probe_capabilities();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, has_cgroup_v2, "requires cgroup v2");

    let id = format!("test-isolation-pids-{}", std::process::id());
    let limiter = CgroupV2Limiter::new();

    let path_str = limiter
        .create(
            &id,
            &ResourceConfig {
                pids_max: Some(32),
                ..ResourceConfig::default()
            },
        )
        .unwrap();
    let path = std::path::PathBuf::from(path_str);

    let pids_max = path.join("pids.max");
    if pids_max.exists() {
        let content = fs::read_to_string(&pids_max).unwrap();
        assert_eq!(content.trim(), "32", "pids.max mismatch");
    }

    limiter.cleanup(&id).unwrap();
}

#[test]
fn cgroup_add_process_writes_pid_to_cgroup_procs() {
    let caps = probe_capabilities();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, has_cgroup_v2, "requires cgroup v2");

    let id = format!("test-isolation-addpid-{}", std::process::id());
    let limiter = CgroupV2Limiter::new();

    let path_str = limiter.create(&id, &ResourceConfig::default()).unwrap();
    let path = std::path::PathBuf::from(&path_str);

    let my_pid = std::process::id();
    limiter.add_process(&id, my_pid).unwrap();

    let procs = fs::read_to_string(path.join("cgroup.procs")).unwrap();
    assert!(
        procs.lines().any(|l| l.trim() == my_pid.to_string()),
        "cgroup.procs must contain PID {my_pid}"
    );

    // Move self back to parent before rmdir (avoids EBUSY).
    let parent = std::path::PathBuf::from(
        std::env::var("MINIBOX_CGROUP_ROOT")
            .unwrap_or_else(|_| "/sys/fs/cgroup/minibox.slice/miniboxd.service".to_string()),
    )
    .join("cgroup.procs");
    if parent.exists() {
        let _ = fs::write(&parent, my_pid.to_string());
    }

    limiter.cleanup(&id).unwrap();
}
