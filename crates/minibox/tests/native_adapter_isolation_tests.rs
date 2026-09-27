//! Native adapter isolation tests (#22).
//!
//! Tests `OverlayFilesystem` and `CgroupV2Limiter` against real kernel
//! infrastructure. `LinuxNamespaceRuntime` has no real-kernel coverage here
//! yet — namespace isolation is exercised end-to-end by `system_tests.rs`.
//! Lifecycle failure paths live in
//! `native_adapter_lifecycle_failure_tests.rs` (#74).
//!
//! **Linux only** — all tests are gated on `cfg(target_os = "linux")` and
//! skip gracefully via `require_capability!` if the host lacks the necessary
//! privileges or kernel features.
//!
//! Run via `just test-integration` (needs Linux + root + cgroup v2 slice).
//! On macOS use `just test-vz-isolation` which drives equivalent behavioral
//! assertions through an in-VM miniboxd agent (`macbox/tests/vz_isolation_tests.rs`).

#![cfg(target_os = "linux")]

use minibox::adapters::{CgroupV2Limiter, OverlayFilesystem};
use minibox::domain::{ResourceConfig, ResourceLimiter, RootfsSetup};
use minibox::preflight::probe;
use minibox_macros::require_capability;
use rstest::{fixture, rstest};
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// A single-layer overlay rootfs, set up and mounted, with the tempdir that
/// owns it.
///
/// Fixtures own *setup* only. Capability gating deliberately stays in each test
/// body: `require_capability!` expands to `return;`, so inside a fixture it
/// would return from the fixture and let the test proceed against a
/// half-constructed environment instead of skipping.
struct OverlayEnv {
    tmp: TempDir,
    layer: PathBuf,
    container_dir: PathBuf,
    adapter: OverlayFilesystem,
    merged: minibox::domain::RootfsLayout,
}

#[fixture]
fn overlay_env() -> OverlayEnv {
    let tmp = TempDir::new().expect("unwrap in test");
    let layer = tmp.path().join("layer0");
    fs::create_dir_all(layer.join("bin")).expect("unwrap in test");
    fs::write(layer.join("bin").join("sh"), b"").expect("unwrap in test");

    let container_dir = tmp.path().join("container");
    fs::create_dir_all(&container_dir).expect("unwrap in test");

    let adapter = OverlayFilesystem::new_with_base(tmp.path());
    let merged = adapter
        .setup_rootfs(std::slice::from_ref(&layer), &container_dir)
        .expect("setup_rootfs failed");

    OverlayEnv {
        tmp,
        layer,
        container_dir,
        adapter,
        merged,
    }
}

impl OverlayEnv {
    /// Mounted, writable view of the rootfs.
    fn merged_dir(&self) -> &std::path::Path {
        &self.merged.merged_dir
    }

    fn upper(&self) -> PathBuf {
        self.container_dir.join("upper")
    }

    fn work(&self) -> PathBuf {
        self.container_dir.join("work")
    }

    /// Unmount. Borrowed rather than consuming so a test can still inspect
    /// the merged dir afterwards to assert it is empty rather than mounted.
    fn teardown(&self) {
        self.adapter
            .cleanup(&self.container_dir)
            .expect("unwrap in test");
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Populate `dir` with a minimal fake image layer (`bin/sh` empty file).
fn fake_layer(dir: &std::path::Path) {
    fs::create_dir_all(dir.join("bin")).expect("unwrap in test");
    fs::write(dir.join("bin").join("sh"), b"").expect("unwrap in test");
}

// ---------------------------------------------------------------------------
// OverlayFilesystem
// ---------------------------------------------------------------------------

#[rstest]
fn overlay_write_goes_to_upper_not_lower(overlay_env: OverlayEnv) {
    let caps = probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, overlay_fs, "requires overlay FS");

    fs::write(overlay_env.merged_dir().join("newfile"), b"hello").expect("unwrap in test");

    assert!(
        overlay_env.upper().join("newfile").exists(),
        "write must land in upper"
    );
    assert!(
        !overlay_env.layer.join("newfile").exists(),
        "lower layer must be unmodified"
    );

    overlay_env.teardown();
}

/// Mount an overlay from `layers` under `tmp`.
///
/// Used by tests that need a layer set other than the single `bin/sh` layer
/// the `overlay_env` fixture provides.
fn mount_overlay(
    tmp: &std::path::Path,
    layers: &[PathBuf],
) -> (OverlayFilesystem, minibox::domain::RootfsLayout, PathBuf) {
    let container_dir = tmp.join("container");
    fs::create_dir_all(&container_dir).expect("unwrap in test");
    let adapter = OverlayFilesystem::new_with_base(tmp);
    let merged = adapter
        .setup_rootfs(layers, &container_dir)
        .expect("setup_rootfs failed");
    (adapter, merged, container_dir)
}

#[test]
fn overlay_multiple_layers_all_visible_in_merged() {
    let caps = probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, overlay_fs, "requires overlay FS");

    let tmp = TempDir::new().expect("unwrap in test");

    let layer0 = tmp.path().join("layer0");
    fs::create_dir_all(layer0.join("etc")).expect("unwrap in test");
    fs::write(layer0.join("etc").join("os-release"), b"ID=test").expect("unwrap in test");

    let layer1 = tmp.path().join("layer1");
    fs::create_dir_all(layer1.join("usr").join("bin")).expect("unwrap in test");
    fs::write(layer1.join("usr").join("bin").join("env"), b"").expect("unwrap in test");

    let (fs_adapter, merged, container_dir) = mount_overlay(tmp.path(), &[layer0, layer1]);

    assert!(
        merged.merged_dir.join("etc").join("os-release").exists(),
        "layer0 content must be visible"
    );
    assert!(
        merged
            .merged_dir
            .join("usr")
            .join("bin")
            .join("env")
            .exists(),
        "layer1 content must be visible"
    );

    fs_adapter.cleanup(&container_dir).expect("unwrap in test");
}

#[rstest]
fn overlay_cleanup_unmounts_merged(overlay_env: OverlayEnv) {
    let caps = probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, overlay_fs, "requires overlay FS");

    assert!(overlay_env.merged_dir().exists());
    overlay_env.teardown();

    // After unmount the dir may still exist but must be empty (not mounted).
    if overlay_env.merged_dir().exists() {
        let entries: Vec<_> = fs::read_dir(overlay_env.merged_dir())
            .expect("unwrap in test")
            .collect();
        assert!(entries.is_empty(), "merged must be empty after unmount");
    }
}

#[test]
fn overlay_empty_layers_returns_error() {
    let caps = probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, overlay_fs, "requires overlay FS");

    let tmp = TempDir::new().expect("unwrap in test");
    let container_dir = tmp.path().join("container");
    fs::create_dir_all(&container_dir).expect("unwrap in test");

    let fs_adapter = OverlayFilesystem::new_with_base(tmp.path());
    assert!(
        fs_adapter.setup_rootfs(&[], &container_dir).is_err(),
        "empty layer list must fail"
    );
}

// ---------------------------------------------------------------------------
// CgroupV2Limiter
// ---------------------------------------------------------------------------

/// A created cgroup. Cleaned up on drop so a failing assertion cannot leave a
/// stray directory under the system cgroup tree.
struct CgroupEnv {
    limiter: CgroupV2Limiter,
    id: String,
    path: PathBuf,
}

impl CgroupEnv {
    /// Read a cgroup control file, or `None` if the kernel did not expose it.
    fn read_control(&self, file: &str) -> Option<String> {
        let p = self.path.join(file);
        p.exists()
            .then(|| fs::read_to_string(p).expect("unwrap in test"))
    }

    fn cleanup_now(&self) {
        self.limiter.cleanup(&self.id).expect("unwrap in test");
    }
}

impl Drop for CgroupEnv {
    fn drop(&mut self) {
        // Best-effort: tests that assert on post-cleanup state already called
        // `cleanup_now`, so a second call here must stay quiet.
        let _ = self.limiter.cleanup(&self.id);
    }
}

/// Create a cgroup for `tag` with `cfg`, returning the environment.
fn cgroup_env(tag: &str, cfg: ResourceConfig) -> CgroupEnv {
    let limiter = CgroupV2Limiter::new();
    let id = format!("test-isolation-{tag}-{}", std::process::id());
    let path_str = limiter
        .create(&id, &cfg)
        .unwrap_or_else(|e| panic!("create failed: {e}"));
    CgroupEnv {
        limiter,
        id,
        path: PathBuf::from(path_str),
    }
}

#[test]
fn cgroup_create_and_cleanup_lifecycle() {
    let caps = probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, cgroups_v2, "requires cgroup v2");

    let env = cgroup_env("create", ResourceConfig::default());

    assert!(env.path.exists(), "cgroup dir must exist after create");
    assert!(
        env.path.join("cgroup.procs").exists(),
        "cgroup.procs must exist"
    );

    env.cleanup_now();
    assert!(
        !env.path.exists(),
        "cgroup dir must be removed after cleanup"
    );
}

/// The three limit tests differ only in which `ResourceConfig` field is set,
/// which control file the kernel exposes it in, and the expected value —
/// so they are one table, not three near-identical functions.
#[rstest]
#[case::memory(
    ResourceConfig { memory_limit_bytes: Some(128 * 1024 * 1024), ..ResourceConfig::default() },
    "memory.max",
    "134217728"
)]
#[case::cpu(
    ResourceConfig { cpu_weight: Some(500), ..ResourceConfig::default() },
    "cpu.weight",
    "500"
)]
#[case::pids(
    ResourceConfig { pids_max: Some(32), ..ResourceConfig::default() },
    "pids.max",
    "32"
)]
fn cgroup_limit_written_correctly(
    #[case] cfg: ResourceConfig,
    #[case] file: &str,
    #[case] expected: &str,
) {
    let caps = probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, cgroups_v2, "requires cgroup v2");

    let env = cgroup_env(&file.replace('.', "-"), cfg);

    if let Some(content) = env.read_control(file) {
        assert_eq!(content.trim(), expected, "{file} mismatch");
    }
}

#[test]
fn cgroup_add_process_writes_pid_to_cgroup_procs() {
    let caps = probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, cgroups_v2, "requires cgroup v2");

    let env = cgroup_env("addpid", ResourceConfig::default());
    let my_pid = std::process::id();
    env.limiter
        .add_process(&env.id, my_pid)
        .expect("unwrap in test");

    let procs = fs::read_to_string(env.path.join("cgroup.procs")).expect("unwrap in test");
    assert!(
        procs.lines().any(|l| l.trim() == my_pid.to_string()),
        "cgroup.procs must contain PID {my_pid}"
    );

    // Move self back to parent before rmdir (avoids EBUSY).
    let parent = PathBuf::from(
        std::env::var("MINIBOX_CGROUP_ROOT")
            .unwrap_or_else(|_| "/sys/fs/cgroup/minibox.slice/miniboxd.service".to_string()),
    )
    .join("cgroup.procs");
    if parent.exists() {
        let _ = fs::write(&parent, my_pid.to_string());
    }

    env.cleanup_now();
}

// ---------------------------------------------------------------------------
// Escape Attempt Detection Tests
// ---------------------------------------------------------------------------

#[test]
fn overlay_path_traversal_attempt_does_not_escape_container_dir() {
    let caps = probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, overlay_fs, "requires overlay FS");

    let tmp = TempDir::new().expect("unwrap in test");
    let layer = tmp.path().join("layer0");
    fake_layer(&layer);

    let container_dir = tmp.path().join("container");
    fs::create_dir_all(&container_dir).expect("unwrap in test");

    let fs_adapter = OverlayFilesystem::new_with_base(tmp.path());
    let merged = fs_adapter
        .setup_rootfs(&[layer], &container_dir)
        .expect("setup_rootfs failed");

    // `merged_dir` is `container_dir/merged` (see `setup_overlay_with_base`), so
    // `../..` climbs two levels out of the container directory entirely and
    // lands outside the sandbox this test creates.
    let escape_path = merged.merged_dir.join("..").join("..").join("escape");
    let escape_dir = escape_path
        .parent()
        .expect("escape path always has a parent")
        .to_path_buf();
    let escape_dir_resolved = escape_dir
        .canonicalize()
        .unwrap_or_else(|_| escape_dir.clone());

    // Guard the test itself: the traversal target must genuinely sit outside
    // the sandbox, otherwise a passing assertion would prove nothing.
    assert!(
        !escape_dir_resolved.starts_with(tmp.path()),
        "test setup is wrong: traversal target {escape_dir_resolved:?} resolved \
         inside the sandbox {tmp:?}, so this test would not exercise an escape"
    );
    assert!(
        !escape_path.exists(),
        "test setup is wrong: {escape_path:?} already exists before the attempt"
    );

    // Attempt the traversal. A write here must not create a file at the
    // resolved target.
    let write_result = fs::write(&escape_path, b"x");

    // If the write unexpectedly succeeded, the escape is real — fail with the
    // concrete target, and clean up so a real escape does not litter a shared
    // temporary directory.
    if write_result.is_ok() && escape_path.exists() {
        let _ = fs::remove_file(&escape_path);
        panic!(
            "path traversal escaped the container: writing {escape_path:?} \
             succeeded and created a file at {escape_dir_resolved:?}, outside \
             both the container dir ({container_dir:?}) and the sandbox ({tmp:?})"
        );
    }

    // Nothing may exist at the traversal target regardless of how the write
    // failed.
    assert!(
        !escape_path.exists(),
        "path traversal created {escape_path:?} outside the container dir"
    );

    fs_adapter.cleanup(&container_dir).expect("unwrap in test");
}

#[test]
fn overlay_symlink_outside_upper_does_not_escape() {
    let caps = probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, overlay_fs, "requires overlay FS");

    let tmp = TempDir::new().expect("unwrap in test");
    let layer = tmp.path().join("layer0");
    fs::create_dir_all(layer.join("tmp")).expect("unwrap in test");

    // Create a symlink that attempts absolute traversal: /tmp/symlink -> /etc
    #[cfg(unix)]
    std::os::unix::fs::symlink("/etc", layer.join("tmp").join("etc_link"))
        .expect("create symlink in layer");

    let container_dir = tmp.path().join("container");
    fs::create_dir_all(&container_dir).expect("unwrap in test");

    let fs_adapter = OverlayFilesystem::new_with_base(tmp.path());
    let merged = fs_adapter
        .setup_rootfs(&[layer], &container_dir)
        .expect("setup_rootfs failed");

    let symlink_in_merged = merged.merged_dir.join("tmp").join("etc_link");

    // Verify the symlink exists in merged.
    assert!(
        symlink_in_merged.exists() || symlink_in_merged.is_symlink(),
        "symlink should exist in merged overlay"
    );

    // Reading the symlink should NOT resolve to the host /etc.
    // Either: symlink was rewritten to relative (safe), or reading fails (safe).
    if let Ok(target) = fs::read_link(&symlink_in_merged) {
        // If symlink is absolute and points to host /etc, that's a failure.
        assert!(
            target != std::path::Path::new("/etc"),
            "symlink must not resolve to host /etc path"
        );

        // Target should be either relative or container-internal.
        if target.is_absolute() {
            // If absolute, it must not be the host /etc.
            assert!(
                !target.starts_with("/etc"),
                "absolute symlink target must not escape to host /etc"
            );
        }
    }

    fs_adapter.cleanup(&container_dir).expect("unwrap in test");
}

#[test]
fn cgroup_pid_zero_is_rejected() {
    let caps = probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, cgroups_v2, "requires cgroup v2");

    let id = format!("test-isolation-pid-zero-{}", std::process::id());
    let limiter = CgroupV2Limiter::new();

    let _path_str = limiter
        .create(&id, &ResourceConfig::default())
        .expect("create cgroup");

    // Attempt to add PID 0, which is invalid.
    let result = limiter.add_process(&id, 0);

    assert!(
        result.is_err(),
        "add_process with PID 0 must return Err, not silently accept"
    );

    limiter.cleanup(&id).expect("unwrap in test");
}

#[test]
fn cgroup_cleanup_removes_all_state() {
    let caps = probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, cgroups_v2, "requires cgroup v2");

    let id = format!("test-isolation-cleanup-{}", std::process::id());
    let limiter = CgroupV2Limiter::new();

    let memory_limit: u64 = 64 * 1024 * 1024; // 64 MB
    let path_str = limiter
        .create(
            &id,
            &ResourceConfig {
                memory_limit_bytes: Some(memory_limit),
                ..ResourceConfig::default()
            },
        )
        .expect("create cgroup with memory limit");
    let path = std::path::PathBuf::from(&path_str);

    assert!(path.exists(), "cgroup dir must exist after create");

    // Add self to the cgroup.
    let my_pid = std::process::id();
    limiter
        .add_process(&id, my_pid)
        .expect("add self to cgroup");

    // Move self back to parent cgroup (required before cleanup on some systems).
    let parent = std::path::PathBuf::from(
        std::env::var("MINIBOX_CGROUP_ROOT")
            .unwrap_or_else(|_| "/sys/fs/cgroup/minibox.slice/miniboxd.service".to_string()),
    )
    .join("cgroup.procs");
    if parent.exists() {
        let _ = fs::write(&parent, my_pid.to_string());
    }

    // Cleanup: must remove the entire cgroup directory.
    limiter.cleanup(&id).expect("unwrap in test");

    assert!(
        !path.exists(),
        "cgroup dir must not exist after cleanup — all state removed"
    );
}
