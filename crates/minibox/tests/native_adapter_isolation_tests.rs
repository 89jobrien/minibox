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
        // Must not swallow this: the process is inside the cgroup being
        // cleaned up, and a failed move-back surfaces later as a confusing
        // EBUSY from `cleanup` rather than as the cause.
        fs::write(&parent, my_pid.to_string()).unwrap_or_else(|e| {
            panic!("failed to move pid {my_pid} back to parent cgroup {parent:?}: {e}");
        });
    }

    env.cleanup_now();
}

// ---------------------------------------------------------------------------
// Mount Containment
// ---------------------------------------------------------------------------

/// Every path the overlay mount uses must stay inside the base the caller
/// supplied.
///
/// This replaces an earlier `overlay_path_traversal_attempt_does_not_escape_container_dir`,
/// which walked `..` out of `merged_dir` and asserted nothing appeared outside
/// the container dir. Running it on Linux showed the write *succeeding* — and
/// that is correct behaviour, not a bug: `merged_dir` is an ordinary directory
/// on the container's filesystem, and overlayfs provides no path containment.
/// Confining a process to the container is `pivot_root` in the runtime's
/// `child_init`, not this adapter's job.
///
/// So the property this layer can actually own is narrower and worth asserting:
/// the mount must not reference any path outside the tree it was handed. If
/// `upper_dir` (or a lower layer) escaped `images_base`, the container would
/// expose host state through the mount, and no amount of `pivot_root` would
/// stop it. Runtime-level containment is covered by the system suite.
#[rstest]
fn overlay_mount_paths_stay_inside_the_supplied_base(overlay_env: OverlayEnv) {
    let caps = probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, overlay_fs, "requires overlay FS");

    let base = overlay_env.tmp.path();

    let Some(minibox_core::domain::BackendRootfsMetadata::Overlay { upper_dir, .. }) =
        overlay_env.merged.rootfs_metadata.as_ref()
    else {
        panic!("overlay setup must report Overlay rootfs metadata");
    };

    let upper = upper_dir.to_path_buf();

    assert!(
        upper.starts_with(&overlay_env.container_dir),
        "upper_dir {upper:?} must live under container_dir {:?}",
        overlay_env.container_dir
    );
    assert!(
        upper.starts_with(base),
        "upper_dir {upper:?} escaped the supplied base {base:?} — the mount would \
         expose host state"
    );
    assert!(upper.exists(), "upper_dir {upper:?} must exist on disk");

    // The merged root must be the mount, not the lower layer itself: if merged
    // *were* the layer, the image would be directly writable.
    assert_ne!(
        overlay_env.merged_dir(),
        overlay_env.layer,
        "merged must be a distinct mount, not the lower layer"
    );
    assert!(
        overlay_env
            .merged_dir()
            .starts_with(&overlay_env.container_dir),
        "merged_dir must live under container_dir"
    );
}

// NOTE: the previous `overlay_symlink_outside_upper_does_not_escape` was removed
// here. It created `/etc_link -> /etc` directly on the filesystem and then
// expected mounting the overlay to rewrite it. overlayfs does no such rewriting
// — absolute symlink targets are rewritten at *layer extraction* time by
// `rewrite_absolute_symlink` (crates/minibox-core/src/image/layer.rs:122), and
// the test bypassed that path entirely. It therefore asserted a property this
// adapter does not have, and failed on a real Linux run.
//
// The security property is real and is covered at the layer that implements it:
//
//   crates/minibox-core/src/image/layer.rs:671  cross_dir_absolute_symlink_rewritten
//   crates/minibox-core/src/image/layer.rs:692  absolute_symlink_rewritten_to_relative
//   crates/minibox-core/src/image/layer.rs:711  absolute_symlink_with_parent_traversal_rejected
//   crates/minibox/tests/security_regression.rs:290  regression_absolute_symlink_with_traversal_is_rejected
//   crates/minibox/tests/security_regression.rs:319  regression_busybox_applet_symlink_is_rewritten_not_rejected
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
        // Must not swallow this: the process is inside the cgroup being
        // cleaned up, and a failed move-back surfaces later as a confusing
        // EBUSY from `cleanup` rather than as the cause.
        fs::write(&parent, my_pid.to_string()).unwrap_or_else(|e| {
            panic!("failed to move pid {my_pid} back to parent cgroup {parent:?}: {e}");
        });
    }

    // Cleanup: must remove the entire cgroup directory.
    limiter.cleanup(&id).expect("unwrap in test");

    assert!(
        !path.exists(),
        "cgroup dir must not exist after cleanup — all state removed"
    );
}
