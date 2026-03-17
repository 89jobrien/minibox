//! Cgroup v2 integration tests exercising the ResourceLimiter trait
//! against real cgroupfs.
//!
//! These tests verify that the CgroupV2Limiter adapter correctly
//! creates, configures, and cleans up cgroups via the domain trait.
//!
//! **Requirements:** Linux, root, cgroups v2
//!
//! **Running:**
//! ```bash
//! just test-integration
//! # or directly:
//! sudo -E cargo test -p miniboxd --test cgroup_tests -- --test-threads=1 --nocapture
//! ```

#![cfg(target_os = "linux")]

use minibox_lib::adapters::CgroupV2Limiter;
use minibox_lib::domain::{ResourceConfig, ResourceLimiter};
use minibox_lib::preflight;
use minibox_lib::require_capability;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Test infrastructure
// ---------------------------------------------------------------------------

/// RAII guard that creates an isolated cgroup subtree for testing
/// and cleans it up on drop.
struct CgroupTestGuard {
    /// Root path for this test's cgroups (set as MINIBOX_CGROUP_ROOT).
    root: PathBuf,
    /// Saved value of MINIBOX_CGROUP_ROOT to restore on drop.
    prev_env: Option<String>,
}

impl CgroupTestGuard {
    fn new() -> Self {
        let id = &uuid::Uuid::new_v4().to_string()[..8];
        let test_name = format!("minibox-test-{id}");

        // Try to create under the current process's cgroup (works on systemd hosts)
        let root = Self::create_test_cgroup(&test_name);

        // Save and override env (unsafe in Rust 2024 edition — process-wide mutation)
        let prev_env = std::env::var("MINIBOX_CGROUP_ROOT").ok();
        // SAFETY: cgroup tests run with --test-threads=1, so no concurrent env access.
        unsafe { std::env::set_var("MINIBOX_CGROUP_ROOT", &root) };

        Self { root, prev_env }
    }

    fn create_test_cgroup(name: &str) -> PathBuf {
        // Read our current cgroup
        let self_cgroup = std::fs::read_to_string("/proc/self/cgroup")
            .unwrap_or_default();
        let cgroup_rel = self_cgroup
            .lines()
            .find_map(|l| l.strip_prefix("0::"))
            .unwrap_or("/")
            .trim()
            .to_string();
        let relative = cgroup_rel.strip_prefix('/').unwrap_or(&cgroup_rel);

        let base = PathBuf::from("/sys/fs/cgroup").join(relative);

        // Create a leaf cgroup for our test process first (so the parent
        // is free to enable subtree_control).
        let test_leaf = base.join(format!("{name}-leaf"));
        let _ = std::fs::create_dir_all(&test_leaf);
        // Move ourselves into the leaf
        let _ = std::fs::write(test_leaf.join("cgroup.procs"), std::process::id().to_string());

        // Now create the test root as a sibling
        let root = base.join(name);
        std::fs::create_dir_all(&root)
            .unwrap_or_else(|e| panic!("failed to create test cgroup at {}: {e}", root.display()));

        // Enable controllers on parent so our test root can use them
        let subtree_ctl = base.join("cgroup.subtree_control");
        for controller in ["+memory", "+cpu", "+pids", "+io"] {
            let _ = std::fs::write(&subtree_ctl, controller);
        }

        root
    }

    /// Path to this test's cgroup root.
    fn root(&self) -> &Path {
        &self.root
    }
}

impl Drop for CgroupTestGuard {
    fn drop(&mut self) {
        // Restore env (unsafe in Rust 2024 — process-wide mutation)
        // SAFETY: cgroup tests run with --test-threads=1
        match &self.prev_env {
            Some(val) => unsafe { std::env::set_var("MINIBOX_CGROUP_ROOT", val) },
            None => unsafe { std::env::remove_var("MINIBOX_CGROUP_ROOT") },
        }

        // Clean up: remove child cgroups first, then the root
        if self.root.exists() {
            // Remove any children (container cgroups created by tests)
            if let Ok(entries) = std::fs::read_dir(&self.root) {
                for entry in entries.flatten() {
                    if entry.path().is_dir() {
                        let _ = std::fs::remove_dir(&entry.path());
                    }
                }
            }
            let _ = std::fs::remove_dir(&self.root);
        }

        // Also clean up the leaf cgroup we created for ourselves
        let leaf_name = self.root
            .file_name()
            .map(|n| format!("{}-leaf", n.to_string_lossy()))
            .unwrap_or_default();
        let leaf = self.root.parent().unwrap_or(Path::new("/")).join(leaf_name);
        // Move ourselves back to the parent first
        if let Some(parent) = self.root.parent() {
            let procs = parent.join("cgroup.procs");
            let _ = std::fs::write(&procs, std::process::id().to_string());
        }
        let _ = std::fs::remove_dir(&leaf);
    }
}

/// Create a ResourceLimiter backed by real cgroups v2.
fn real_limiter() -> Arc<dyn ResourceLimiter> {
    Arc::new(CgroupV2Limiter::new())
}

// ---------------------------------------------------------------------------
// Env var override test
// ---------------------------------------------------------------------------

#[test]
fn test_cgroup_root_env_override() {
    let caps = preflight::probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, cgroups_v2, "requires cgroups v2");

    let guard = CgroupTestGuard::new();
    let limiter = real_limiter();

    let config = ResourceConfig {
        memory_limit_bytes: Some(64 * 1024 * 1024),
        ..Default::default()
    };

    let cgroup_path = limiter.create("env-override-test", &config)
        .expect("create should succeed");

    // Verify the cgroup was created under our test root, not the default
    assert!(
        cgroup_path.starts_with(&guard.root().to_string_lossy().to_string()),
        "cgroup path {} should be under test root {}",
        cgroup_path,
        guard.root().display()
    );

    // Cleanup
    limiter.cleanup("env-override-test").expect("cleanup should succeed");
}

// ---------------------------------------------------------------------------
// Cgroup lifecycle tests
// ---------------------------------------------------------------------------

#[test]
fn test_cgroup_create_and_verify_directory() {
    let caps = preflight::probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, cgroups_v2, "requires cgroups v2");

    let _guard = CgroupTestGuard::new();
    let limiter = real_limiter();
    let config = ResourceConfig::default();

    let cgroup_path = limiter.create("test-create", &config)
        .expect("create should succeed");

    assert!(
        Path::new(&cgroup_path).exists(),
        "cgroup directory should exist at {}",
        cgroup_path
    );

    limiter.cleanup("test-create").expect("cleanup");
}

#[test]
fn test_cgroup_memory_limit_written_and_readable() {
    let caps = preflight::probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, cgroups_v2, "requires cgroups v2");

    let _guard = CgroupTestGuard::new();
    let limiter = real_limiter();

    let config = ResourceConfig {
        memory_limit_bytes: Some(128 * 1024 * 1024), // 128MB
        ..Default::default()
    };

    let cgroup_path = limiter.create("test-mem", &config).expect("create");

    let memory_max = std::fs::read_to_string(format!("{}/memory.max", cgroup_path))
        .expect("read memory.max");
    assert_eq!(memory_max.trim(), "134217728", "memory.max should be 128MB in bytes");

    limiter.cleanup("test-mem").expect("cleanup");
}

#[test]
fn test_cgroup_cpu_weight_written_and_readable() {
    let caps = preflight::probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, cgroups_v2, "requires cgroups v2");

    let _guard = CgroupTestGuard::new();
    let limiter = real_limiter();

    let config = ResourceConfig {
        cpu_weight: Some(250),
        ..Default::default()
    };

    let cgroup_path = limiter.create("test-cpu", &config).expect("create");

    let cpu_weight = std::fs::read_to_string(format!("{}/cpu.weight", cgroup_path))
        .expect("read cpu.weight");
    assert_eq!(cpu_weight.trim(), "250");

    limiter.cleanup("test-cpu").expect("cleanup");
}

#[test]
fn test_cgroup_pids_max_default() {
    let caps = preflight::probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, cgroups_v2, "requires cgroups v2");

    let _guard = CgroupTestGuard::new();
    let limiter = real_limiter();

    // No pids_max set — should default to 1024
    let config = ResourceConfig::default();

    let cgroup_path = limiter.create("test-pids-default", &config).expect("create");

    let pids_max = std::fs::read_to_string(format!("{}/pids.max", cgroup_path))
        .expect("read pids.max");
    assert_eq!(pids_max.trim(), "1024", "default pids.max should be 1024");

    limiter.cleanup("test-pids-default").expect("cleanup");
}

#[test]
fn test_cgroup_pids_max_custom() {
    let caps = preflight::probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, cgroups_v2, "requires cgroups v2");

    let _guard = CgroupTestGuard::new();
    let limiter = real_limiter();

    let config = ResourceConfig {
        pids_max: Some(512),
        ..Default::default()
    };

    let cgroup_path = limiter.create("test-pids-custom", &config).expect("create");

    let pids_max = std::fs::read_to_string(format!("{}/pids.max", cgroup_path))
        .expect("read pids.max");
    assert_eq!(pids_max.trim(), "512");

    limiter.cleanup("test-pids-custom").expect("cleanup");
}

#[test]
fn test_cgroup_io_max_written() {
    let caps = preflight::probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, cgroups_v2, "requires cgroups v2");

    // io controller may not be available on all hosts
    if !caps.cgroup_controllers.iter().any(|c| c == "io") {
        eprintln!("SKIPPED: io controller not available");
        return;
    }

    let _guard = CgroupTestGuard::new();
    let limiter = real_limiter();

    let config = ResourceConfig {
        io_max_bytes_per_sec: Some(10 * 1024 * 1024), // 10MB/s
        ..Default::default()
    };

    let cgroup_path = limiter.create("test-io", &config).expect("create");

    let io_max = std::fs::read_to_string(format!("{}/io.max", cgroup_path))
        .expect("read io.max");
    assert!(
        io_max.contains("rbps=10485760"),
        "io.max should contain rbps=10485760, got: {}",
        io_max.trim()
    );

    limiter.cleanup("test-io").expect("cleanup");
}

#[test]
fn test_cgroup_add_process() {
    let caps = preflight::probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, cgroups_v2, "requires cgroups v2");

    let _guard = CgroupTestGuard::new();
    let limiter = real_limiter();
    let config = ResourceConfig::default();

    let cgroup_path = limiter.create("test-add-proc", &config).expect("create");

    // Fork a child process that sleeps
    let child = Command::new("sleep")
        .arg("10")
        .spawn()
        .expect("spawn sleep");
    let child_pid = child.id();

    // Add the child to the cgroup
    limiter.add_process("test-add-proc", child_pid).expect("add_process");

    // Verify the PID appears in cgroup.procs
    let procs = std::fs::read_to_string(format!("{}/cgroup.procs", cgroup_path))
        .expect("read cgroup.procs");
    assert!(
        procs.contains(&child_pid.to_string()),
        "cgroup.procs should contain PID {}, got: {}",
        child_pid,
        procs.trim()
    );

    // Kill the child and cleanup
    let mut child = child;
    let _ = child.kill();
    let _ = child.wait();
    limiter.cleanup("test-add-proc").expect("cleanup");
}

#[test]
fn test_cgroup_cleanup_removes_directory() {
    let caps = preflight::probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, cgroups_v2, "requires cgroups v2");

    let _guard = CgroupTestGuard::new();
    let limiter = real_limiter();
    let config = ResourceConfig::default();

    let cgroup_path = limiter.create("test-cleanup", &config).expect("create");
    assert!(Path::new(&cgroup_path).exists());

    limiter.cleanup("test-cleanup").expect("cleanup");
    assert!(
        !Path::new(&cgroup_path).exists(),
        "cgroup directory should be removed after cleanup"
    );
}

#[test]
fn test_cgroup_cleanup_idempotent() {
    let caps = preflight::probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, cgroups_v2, "requires cgroups v2");

    let _guard = CgroupTestGuard::new();
    let limiter = real_limiter();
    let config = ResourceConfig::default();

    limiter.create("test-idempotent", &config).expect("create");
    limiter.cleanup("test-idempotent").expect("first cleanup");

    // Second cleanup on already-removed cgroup should not error
    let result = limiter.cleanup("test-idempotent");
    assert!(result.is_ok(), "second cleanup should succeed (idempotent)");
}

// ---------------------------------------------------------------------------
// Controller delegation tests
// ---------------------------------------------------------------------------

#[test]
fn test_subtree_controllers_enabled() {
    let caps = preflight::probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, cgroups_v2, "requires cgroups v2");

    let guard = CgroupTestGuard::new();
    let limiter = real_limiter();
    let config = ResourceConfig::default();

    // Creating a cgroup should enable subtree controllers on the parent
    let cgroup_path = limiter.create("test-subtree", &config).expect("create");

    let subtree_ctl = std::fs::read_to_string(
        guard.root().join("cgroup.subtree_control"),
    )
    .unwrap_or_default();

    // At minimum, pids and memory should be enabled (cpu and io may vary)
    assert!(
        subtree_ctl.contains("memory"),
        "subtree_control should contain 'memory', got: {}",
        subtree_ctl.trim()
    );
    assert!(
        subtree_ctl.contains("pids"),
        "subtree_control should contain 'pids', got: {}",
        subtree_ctl.trim()
    );

    limiter.cleanup("test-subtree").expect("cleanup");
}

#[test]
fn test_cgroup_in_delegated_subtree() {
    let caps = preflight::probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, cgroups_v2, "requires cgroups v2");

    let _guard = CgroupTestGuard::new();
    let limiter = real_limiter();

    // Create a cgroup with limits — this exercises the full delegation path
    let config = ResourceConfig {
        memory_limit_bytes: Some(32 * 1024 * 1024),
        cpu_weight: Some(100),
        pids_max: Some(256),
        ..Default::default()
    };

    let cgroup_path = limiter.create("test-delegated", &config).expect("create");

    // Verify all limit files were written
    assert!(Path::new(&format!("{}/memory.max", cgroup_path)).exists());
    assert!(Path::new(&format!("{}/cpu.weight", cgroup_path)).exists());
    assert!(Path::new(&format!("{}/pids.max", cgroup_path)).exists());

    limiter.cleanup("test-delegated").expect("cleanup");
}

// ---------------------------------------------------------------------------
// Validation / error tests
// ---------------------------------------------------------------------------

#[test]
fn test_cgroup_rejects_memory_below_minimum() {
    let caps = preflight::probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, cgroups_v2, "requires cgroups v2");

    let _guard = CgroupTestGuard::new();
    let limiter = real_limiter();

    let config = ResourceConfig {
        memory_limit_bytes: Some(100), // Below 4096 minimum
        ..Default::default()
    };

    let result = limiter.create("test-mem-low", &config);
    assert!(result.is_err(), "should reject memory < 4096 bytes");

    // Cleanup in case it partially created
    let _ = limiter.cleanup("test-mem-low");
}

#[test]
fn test_cgroup_rejects_invalid_cpu_weight() {
    let caps = preflight::probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, cgroups_v2, "requires cgroups v2");

    let _guard = CgroupTestGuard::new();
    let limiter = real_limiter();

    // Test zero
    let config = ResourceConfig {
        cpu_weight: Some(0),
        ..Default::default()
    };
    let result = limiter.create("test-cpu-zero", &config);
    assert!(result.is_err(), "should reject cpu_weight 0");
    let _ = limiter.cleanup("test-cpu-zero");

    // Test above max
    let config = ResourceConfig {
        cpu_weight: Some(10001),
        ..Default::default()
    };
    let result = limiter.create("test-cpu-high", &config);
    assert!(result.is_err(), "should reject cpu_weight 10001");
    let _ = limiter.cleanup("test-cpu-high");
}

#[test]
fn test_cgroup_add_process_invalid_pid() {
    let caps = preflight::probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, cgroups_v2, "requires cgroups v2");

    let _guard = CgroupTestGuard::new();
    let limiter = real_limiter();
    let config = ResourceConfig::default();

    limiter.create("test-bad-pid", &config).expect("create");

    // PID 0 is invalid — kernel rejects it
    let result = limiter.add_process("test-bad-pid", 0);
    assert!(result.is_err(), "adding PID 0 should fail");

    // Very large PID that doesn't exist
    let result = limiter.add_process("test-bad-pid", 4_000_000);
    assert!(result.is_err(), "adding nonexistent PID should fail");

    let _ = limiter.cleanup("test-bad-pid");
}

// ---------------------------------------------------------------------------
// Controller availability test
// ---------------------------------------------------------------------------

#[test]
fn test_cgroup_io_controller_unavailable() {
    let caps = preflight::probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, cgroups_v2, "requires cgroups v2");

    // This test documents behavior when io controller is missing.
    // If io IS available, we can't test the "unavailable" path directly,
    // so we just verify create() succeeds regardless of io availability.
    let _guard = CgroupTestGuard::new();
    let limiter = real_limiter();

    let config = ResourceConfig {
        io_max_bytes_per_sec: Some(1024 * 1024),
        ..Default::default()
    };

    let result = limiter.create("test-io-avail", &config);
    if caps.cgroup_controllers.iter().any(|c| c == "io") {
        // io controller available — create should succeed and write io.max
        assert!(result.is_ok(), "create should succeed when io controller is available");
    } else {
        // io controller unavailable — create will fail when writing io.max
        // This documents the current behavior: io.max write propagates errors.
        // A future improvement could make this non-fatal.
        assert!(result.is_err(), "create fails when io controller is unavailable and io limit is set");
    }

    let _ = limiter.cleanup("test-io-avail");
}
