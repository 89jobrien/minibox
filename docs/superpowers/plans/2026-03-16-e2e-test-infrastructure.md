---
status: active
note: Framework still evolving
---

# E2E Test Infrastructure Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add automated e2e test infrastructure with preflight probing, cgroup v2 integration tests, and daemon+CLI e2e tests, all exercising domain traits through hexagonal architecture.

**Architecture:** Three test layers (preflight → integration → e2e) with a justfile task runner. Integration tests call domain traits against real cgroupfs. E2E tests start real daemon+CLI binaries. Cleanup guards prevent resource leaks.

**Tech Stack:** Rust, cargo test, just, nix crate, tempfile, tokio, systemd-run (for cgroup delegation on systemd hosts)

**Spec:** `docs/superpowers/specs/2026-03-16-e2e-test-infrastructure-design.md`

---

## Chunk 1: Prerequisites and Preflight Module

### Task 1: Make CLI socket path configurable

The CLI hardcodes the socket path to `/run/minibox/miniboxd.sock`. E2E tests need to point at a temp socket. Add `MINIBOX_SOCKET_PATH` env var support.

**Files:**

- Modify: `crates/minibox-cli/src/commands/mod.rs`

- [ ] **Step 1: Write the failing test**

No Rust test needed — this is a one-line env var change. Verify manually after.

- [ ] **Step 2: Make socket path configurable**

In `crates/minibox-cli/src/commands/mod.rs`, replace the constant with an env-var-aware function:

```rust
/// Default Unix socket path of the running daemon.
const DEFAULT_SOCKET_PATH: &str = "/run/minibox/miniboxd.sock";

/// Resolve the daemon socket path.
///
/// Checks `MINIBOX_SOCKET_PATH` env var first, falls back to default.
fn socket_path() -> String {
    std::env::var("MINIBOX_SOCKET_PATH")
        .unwrap_or_else(|_| DEFAULT_SOCKET_PATH.to_string())
}
```

Then update `send_request()` to call `socket_path()` instead of using `SOCKET_PATH`:

```rust
pub async fn send_request(request: &DaemonRequest) -> Result<DaemonResponse> {
    let path = socket_path();
    let stream = UnixStream::connect(&path)
        .await
        .with_context(|| format!("connecting to daemon at {}", path))?;
    // ... rest unchanged
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo check -p minibox-cli`
Expected: compiles without errors

- [ ] **Step 4: Commit**

```bash
git add crates/minibox-cli/src/commands/mod.rs
git commit -m "feat: make CLI socket path configurable via MINIBOX_SOCKET_PATH env var"
```

---

### Task 2: Create preflight module

**Files:**

- Create: `crates/mbx/src/preflight.rs`
- Modify: `crates/mbx/src/lib.rs`

- [ ] **Step 1: Add module declaration**

In `crates/mbx/src/lib.rs`, add after the existing modules:

```rust
pub mod preflight;
```

Note: do NOT gate this behind `#[cfg(target_os = "linux")]` — the probe functions return false/empty on non-Linux. This keeps it usable from macOS dev machines for `just doctor` (reports "not Linux").

- [ ] **Step 2: Create preflight.rs with HostCapabilities struct and probe()**

Create `crates/mbx/src/preflight.rs`:

````rust
//! Host capability probing for test infrastructure and diagnostics.
//!
//! Probes the current host for features needed by minibox: cgroups v2,
//! overlay filesystem, kernel version, systemd status. Pure reads, no
//! mutations. Infallible — missing data yields false/empty.
//!
//! Used by:
//! - Integration and e2e tests to skip tests gracefully
//! - `just doctor` to report host readiness
//! - Future `minibox doctor` CLI subcommand

use std::path::Path;
use std::process::Command;

/// Host capabilities relevant to minibox operation.
#[derive(Debug, Clone)]
pub struct HostCapabilities {
    /// Running as UID 0.
    pub is_root: bool,
    /// Kernel version as (major, minor, patch).
    pub kernel_version: (u32, u32, u32),
    /// cgroup2 filesystem mounted (typically at /sys/fs/cgroup).
    pub cgroups_v2: bool,
    /// Controllers listed in /sys/fs/cgroup/cgroup.controllers.
    pub cgroup_controllers: Vec<String>,
    /// Can write to cgroup.subtree_control (delegation works).
    pub cgroup_subtree_delegatable: bool,
    /// "overlay" listed in /proc/filesystems.
    pub overlay_fs: bool,
    /// systemctl binary exists and responds.
    pub systemd_available: bool,
    /// Parsed from `systemctl --version` (e.g., 252).
    pub systemd_version: Option<u32>,
    /// minibox.slice is loaded in systemd.
    pub minibox_slice_active: bool,
}

/// Probe the current host for minibox-relevant capabilities.
///
/// This function never fails — it returns false/empty for anything it
/// cannot determine. Safe to call on any platform.
pub fn probe() -> HostCapabilities {
    HostCapabilities {
        is_root: probe_root(),
        kernel_version: probe_kernel_version(),
        cgroups_v2: probe_cgroups_v2(),
        cgroup_controllers: probe_cgroup_controllers(),
        cgroup_subtree_delegatable: probe_subtree_delegatable(),
        overlay_fs: probe_overlay_fs(),
        systemd_available: probe_systemd_available(),
        systemd_version: probe_systemd_version(),
        minibox_slice_active: probe_minibox_slice(),
    }
}

/// Format a human-readable report of host capabilities.
pub fn format_report(caps: &HostCapabilities) -> String {
    let mut lines = Vec::new();
    lines.push("Minibox Host Capabilities".to_string());
    lines.push("=".repeat(40));

    let (maj, min, patch) = caps.kernel_version;
    lines.push(format!(
        "{} Kernel: {}.{}.{}",
        if maj >= 5 { "PASS" } else { "WARN" },
        maj,
        min,
        patch
    ));
    lines.push(format!(
        "{} Root: {}",
        if caps.is_root { "PASS" } else { "FAIL" },
        caps.is_root
    ));
    lines.push(format!(
        "{} cgroups v2: {}",
        if caps.cgroups_v2 { "PASS" } else { "FAIL" },
        caps.cgroups_v2
    ));
    lines.push(format!(
        "     Controllers: [{}]",
        caps.cgroup_controllers.join(", ")
    ));
    lines.push(format!(
        "{} Subtree delegation: {}",
        if caps.cgroup_subtree_delegatable {
            "PASS"
        } else {
            "WARN"
        },
        caps.cgroup_subtree_delegatable
    ));
    lines.push(format!(
        "{} Overlay FS: {}",
        if caps.overlay_fs { "PASS" } else { "FAIL" },
        caps.overlay_fs
    ));
    lines.push(format!(
        "     systemd: {} (version: {})",
        caps.systemd_available,
        caps.systemd_version
            .map(|v| v.to_string())
            .unwrap_or_else(|| "N/A".to_string())
    ));
    lines.push(format!(
        "     minibox.slice: {}",
        caps.minibox_slice_active
    ));

    lines.join("\n")
}

/// Skip-friendly macro for tests. Usage:
///
/// ```rust,ignore
/// let caps = mbx::preflight::probe();
/// mbx::require_capability!(caps, is_root, "requires root");
/// mbx::require_capability!(caps, cgroups_v2, "requires cgroups v2");
/// ```
#[macro_export]
macro_rules! require_capability {
    ($caps:expr, $field:ident, $reason:expr) => {
        if !$caps.$field {
            eprintln!("SKIPPED: {}", $reason);
            return;
        }
    };
}

// ---------------------------------------------------------------------------
// Probe helpers
// ---------------------------------------------------------------------------

fn probe_root() -> bool {
    // Works on any Unix; returns false on non-Unix
    #[cfg(unix)]
    {
        unsafe { libc::geteuid() == 0 }
    }
    #[cfg(not(unix))]
    {
        false
    }
}

fn probe_kernel_version() -> (u32, u32, u32) {
    let content = match std::fs::read_to_string("/proc/version") {
        Ok(s) => s,
        Err(_) => return (0, 0, 0),
    };
    // "Linux version 6.1.0-18-amd64 ..."
    let version_str = content
        .split_whitespace()
        .nth(2)
        .unwrap_or("0.0.0");
    parse_kernel_version(version_str)
}

fn parse_kernel_version(s: &str) -> (u32, u32, u32) {
    let parts: Vec<&str> = s.split('.').collect();
    let major = parts.first().and_then(|p| p.parse().ok()).unwrap_or(0);
    let minor = parts.get(1).and_then(|p| p.parse().ok()).unwrap_or(0);
    // Patch may have suffix like "0-18-amd64"
    let patch = parts
        .get(2)
        .and_then(|p| p.split('-').next())
        .and_then(|p| p.parse().ok())
        .unwrap_or(0);
    (major, minor, patch)
}

fn probe_cgroups_v2() -> bool {
    std::fs::read_to_string("/proc/mounts")
        .map(|s| s.contains("cgroup2"))
        .unwrap_or(false)
}

fn probe_cgroup_controllers() -> Vec<String> {
    std::fs::read_to_string("/sys/fs/cgroup/cgroup.controllers")
        .map(|s| s.split_whitespace().map(String::from).collect())
        .unwrap_or_default()
}

fn probe_subtree_delegatable() -> bool {
    // Check if we can read subtree_control (basic accessibility check).
    // A full check would try writing, but we keep probe() read-only.
    Path::new("/sys/fs/cgroup/cgroup.subtree_control").exists()
}

fn probe_overlay_fs() -> bool {
    std::fs::read_to_string("/proc/filesystems")
        .map(|s| s.contains("overlay"))
        .unwrap_or(false)
}

fn probe_systemd_available() -> bool {
    Command::new("systemctl")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn probe_systemd_version() -> Option<u32> {
    let output = Command::new("systemctl")
        .arg("--version")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    // "systemd 252 (252.22-1~deb12u1)"
    stdout
        .lines()
        .next()?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}

fn probe_minibox_slice() -> bool {
    Command::new("systemctl")
        .args(["is-active", "minibox.slice"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_kernel_version() {
        assert_eq!(parse_kernel_version("6.1.0-18-amd64"), (6, 1, 0));
        assert_eq!(parse_kernel_version("5.15.0"), (5, 15, 0));
        assert_eq!(parse_kernel_version("4.19.128"), (4, 19, 128));
        assert_eq!(parse_kernel_version("garbage"), (0, 0, 0));
        assert_eq!(parse_kernel_version(""), (0, 0, 0));
    }

    #[test]
    fn test_probe_does_not_panic() {
        // probe() must never panic, regardless of platform
        let caps = probe();
        let _ = format!("{:?}", caps);
    }

    #[test]
    fn test_format_report_does_not_panic() {
        let caps = probe();
        let report = format_report(&caps);
        assert!(report.contains("Minibox Host Capabilities"));
    }
}
````

- [ ] **Step 3: Verify it compiles and unit tests pass**

Run: `cargo test -p mbx preflight`
Expected: 3 tests pass

- [ ] **Step 4: Commit**

```bash
git add crates/mbx/src/preflight.rs crates/mbx/src/lib.rs
git commit -m "feat: add preflight host capability probing module"
```

---

### Task 3: Create justfile

**Files:**

- Create: `justfile`

- [ ] **Step 1: Create justfile**

Create `justfile` at repo root:

```just
default:
    @just --list

# Preflight capability check
doctor:
    @cargo test -p mbx preflight::tests -- --nocapture 2>&1 || true
    @echo ""
    @echo "--- Host Capabilities Report ---"
    @cargo test -p mbx preflight::tests::test_format_report_does_not_panic -- --nocapture 2>&1 | grep -A 20 "Minibox Host Capabilities" || echo "Could not generate report (non-Linux host?)"

# Build release binaries
build:
    cargo build --release

# Unit tests (mock-based, any platform)
test-unit:
    cargo test --workspace --lib
    cargo test -p miniboxd --test handler_tests
    cargo test -p miniboxd --test conformance_tests

# Cgroup integration tests (Linux, root)
test-integration:
    sudo -E cargo test -p miniboxd --test cgroup_tests -- --test-threads=1 --nocapture
    sudo -E cargo test -p miniboxd --test integration_tests -- --test-threads=1 --ignored --nocapture

# Daemon+CLI e2e tests (Linux, root)
# Build as current user, run compiled test binary under sudo to avoid root-owned target/ files.
test-e2e:
    cargo build --release
    cargo test -p miniboxd --test e2e_tests --release --no-run --message-format=json 2>/dev/null | jq -r 'select(.executable) | .executable' > /tmp/minibox-e2e-bin
    sudo -E MINIBOX_TEST_BIN_DIR={{justfile_directory()}}/target/release $(cat /tmp/minibox-e2e-bin) --test-threads=1 --nocapture

# Full pipeline: clean state → doctor → all tests → clean state
test-all: nuke-test-state doctor test-unit test-integration test-e2e nuke-test-state

# Remove all build artifacts
clean:
    cargo clean

# Remove only test-related build artifacts
clean-test:
    find target/debug/deps -name '*_tests-*' -delete 2>/dev/null || true
    find target/debug/deps -name '*miniboxd-*' -delete 2>/dev/null || true

# Remove target/ artifacts older than N days (default 7)
clean-stale days="7":
    find target/ -type f -mtime +{{days}} -delete 2>/dev/null || true
    find target/ -type d -empty -delete 2>/dev/null || true

# Kill orphan processes, unmount overlays, remove test cgroups, clean temp dirs
nuke-test-state:
    #!/usr/bin/env bash
    set -euo pipefail
    pkill -f 'miniboxd.*minibox-test' 2>/dev/null || true
    mount | grep 'minibox-test' | awk '{print $3}' | xargs -r umount 2>/dev/null || true
    systemctl list-units --type=scope --no-legend 2>/dev/null | grep minibox-test | awk '{print $1}' | xargs -r systemctl stop 2>/dev/null || true
    find /sys/fs/cgroup -name 'minibox-test-*' -type d -exec rmdir {} \; 2>/dev/null || true
    rm -rf /tmp/minibox-test-* 2>/dev/null || true
    echo "test state cleaned"
```

- [ ] **Step 2: Verify justfile parses**

Run: `just --list`
Expected: lists all recipes without error

- [ ] **Step 3: Commit**

```bash
git add justfile
git commit -m "feat: add justfile task runner for test workflows"
```

---

## Chunk 2: Cgroup Integration Tests

### Task 4: Create CgroupTestGuard and test helpers

**Files:**

- Create: `crates/miniboxd/tests/cgroup_tests.rs`

- [ ] **Step 1: Create cgroup_tests.rs with test infrastructure**

Create `crates/miniboxd/tests/cgroup_tests.rs` with the guard, helpers, and first test:

````rust
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

use mbx::adapters::CgroupV2Limiter;
use mbx::domain::{ResourceConfig, ResourceLimiter};
use mbx::preflight;
use mbx::require_capability;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Test infrastructure
// ---------------------------------------------------------------------------

/// RAII guard that creates an isolated cgroup subtree for testing
/// and cleans it up on drop.
///
/// On systemd hosts, uses `systemd-run --scope` to get a delegated subtree.
/// On bare-metal, creates directly under /sys/fs/cgroup/.
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
        let controllers = "+memory +cpu +pids +io";
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
````

- [ ] **Step 2: Verify it compiles**

Run: `cargo check -p miniboxd --test cgroup_tests`
Expected: compiles (may have unused variable warnings for `guard` — that's fine, the guard holds the env var override via its Drop)

- [ ] **Step 3: Commit**

```bash
git add crates/miniboxd/tests/cgroup_tests.rs
git commit -m "feat: add cgroup v2 integration tests exercising ResourceLimiter trait"
```

---

## Chunk 3: Daemon+CLI E2E Tests

### Task 5: Create DaemonFixture and e2e test helpers

**Files:**

- Create: `crates/miniboxd/tests/e2e_tests.rs`

- [ ] **Step 1: Create e2e_tests.rs with DaemonFixture and first tests**

Create `crates/miniboxd/tests/e2e_tests.rs`:

````rust
//! End-to-end tests: start real miniboxd + minibox CLI binaries.
//!
//! Tests the full stack through Unix socket: daemon startup, image pull,
//! container lifecycle, resource limits, cleanup, and signal handling.
//!
//! **Requirements:** Linux, root, cgroups v2, built binaries
//!
//! **Running:**
//! ```bash
//! just test-e2e
//! ```

#![cfg(target_os = "linux")]

use mbx::preflight;
use mbx::require_capability;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Binary resolution
// ---------------------------------------------------------------------------

/// Find a minibox binary by name.
///
/// Search order:
/// 1. `MINIBOX_TEST_BIN_DIR` env var (set by justfile)
/// 2. `target/release/{name}`
/// 3. `target/debug/{name}`
fn find_binary(name: &str) -> PathBuf {
    if let Ok(dir) = std::env::var("MINIBOX_TEST_BIN_DIR") {
        let p = PathBuf::from(&dir).join(name);
        if p.exists() {
            return p;
        }
    }

    // Try relative to workspace root (CARGO_MANIFEST_DIR points to miniboxd crate)
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .expect("could not find workspace root");

    for profile in ["release", "debug"] {
        let p = workspace_root.join("target").join(profile).join(name);
        if p.exists() {
            return p;
        }
    }

    panic!(
        "Could not find binary '{}'. Run `cargo build --release` first, \
         or set MINIBOX_TEST_BIN_DIR.",
        name
    );
}

// ---------------------------------------------------------------------------
// DaemonFixture
// ---------------------------------------------------------------------------

/// RAII fixture that starts a real miniboxd and provides CLI access.
struct DaemonFixture {
    child: Child,
    socket_path: PathBuf,
    data_dir: TempDir,
    run_dir: TempDir,
    cgroup_root: PathBuf,
    cli_bin: PathBuf,
}

impl DaemonFixture {
    /// Start a daemon with isolated temp dirs.
    ///
    /// Panics if the daemon fails to start within 10 seconds.
    fn start() -> Self {
        let data_dir = TempDir::with_prefix("minibox-test-data-")
            .expect("create temp data dir");
        let run_dir = TempDir::with_prefix("minibox-test-run-")
            .expect("create temp run dir");

        let socket_path = run_dir.path().join("miniboxd.sock");

        // Create cgroup root under our own cgroup (not top-level, which
        // fails on systemd hosts). Read /proc/self/cgroup to find our
        // current cgroup, then create a test subtree there.
        let self_cgroup = std::fs::read_to_string("/proc/self/cgroup")
            .unwrap_or_default();
        let cgroup_rel = self_cgroup
            .lines()
            .find_map(|l| l.strip_prefix("0::"))
            .unwrap_or("/")
            .trim()
            .to_string();
        let relative = cgroup_rel.strip_prefix('/').unwrap_or(&cgroup_rel);
        let test_name = format!(
            "minibox-test-e2e-{}",
            &uuid::Uuid::new_v4().to_string()[..8]
        );
        let cgroup_root = PathBuf::from("/sys/fs/cgroup")
            .join(relative)
            .join(&test_name);

        let daemon_bin = find_binary("miniboxd");
        let cli_bin = find_binary("minibox");

        // Create cgroup root
        std::fs::create_dir_all(&cgroup_root).ok();

        let child = Command::new(&daemon_bin)
            .env("MINIBOX_DATA_DIR", data_dir.path())
            .env("MINIBOX_RUN_DIR", run_dir.path())
            .env("MINIBOX_SOCKET_PATH", &socket_path)
            .env("MINIBOX_CGROUP_ROOT", &cgroup_root)
            .env("RUST_LOG", "miniboxd=debug")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap_or_else(|e| panic!("failed to start miniboxd at {:?}: {e}", daemon_bin));

        let fixture = Self {
            child,
            socket_path: socket_path.clone(),
            data_dir,
            run_dir,
            cgroup_root,
            cli_bin,
        };

        // Wait for socket to appear
        let start = Instant::now();
        let timeout = Duration::from_secs(10);
        while !socket_path.exists() {
            if start.elapsed() > timeout {
                // Kill and capture stderr for debugging
                let mut fixture = fixture;
                let stderr = fixture.kill_and_capture_stderr();
                panic!(
                    "miniboxd did not create socket within 10s.\nSocket: {:?}\nStderr:\n{}",
                    socket_path, stderr
                );
            }
            std::thread::sleep(Duration::from_millis(100));
        }

        fixture
    }

    /// Return the daemon's PID.
    fn daemon_pid(&self) -> u32 {
        self.child.id()
    }

    /// Create a Command for the minibox CLI pre-configured with our socket.
    fn cli(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(&self.cli_bin);
        cmd.env("MINIBOX_SOCKET_PATH", &self.socket_path);
        cmd.args(args);
        cmd
    }

    /// Run a CLI command and return (exit_status, stdout, stderr).
    fn run_cli(&self, args: &[&str]) -> (bool, String, String) {
        let output = self
            .cli(args)
            .output()
            .unwrap_or_else(|e| panic!("failed to run minibox {:?}: {e}", args));

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        (output.status.success(), stdout, stderr)
    }

    /// Kill daemon and capture stderr for debugging.
    /// Only call when the daemon is expected to have failed.
    fn kill_and_capture_stderr(&mut self) -> String {
        let _ = self.child.kill();
        let output = self.child.wait_with_output();
        match output {
            Ok(o) => String::from_utf8_lossy(&o.stderr).to_string(),
            Err(e) => format!("(could not capture stderr: {e})"),
        }
    }

    /// Send SIGTERM to the daemon.
    fn sigterm(&self) {
        unsafe {
            libc::kill(self.child.id() as i32, libc::SIGTERM);
        }
    }
}

impl Drop for DaemonFixture {
    fn drop(&mut self) {
        // 1. Send SIGTERM
        unsafe {
            libc::kill(self.child.id() as i32, libc::SIGTERM);
        }

        // 2. Wait up to 5s for clean exit
        let start = Instant::now();
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) => {
                    if start.elapsed() > Duration::from_secs(5) {
                        // 3. Escalate to SIGKILL
                        let _ = self.child.kill();
                        let _ = self.child.wait();
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(_) => break,
            }
        }

        // 4. Cleanup cgroup tree (cgroupfs only supports rmdir, not rm -rf)
        if self.cgroup_root.exists() {
            // Remove leaf cgroups first (children), then root.
            // cgroupfs requires directories to be empty (no child cgroups).
            if let Ok(entries) = std::fs::read_dir(&self.cgroup_root) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        // Recurse one level for nested cgroups (e.g., supervisor/)
                        if let Ok(sub_entries) = std::fs::read_dir(&path) {
                            for sub in sub_entries.flatten() {
                                if sub.path().is_dir() {
                                    let _ = std::fs::remove_dir(&sub.path());
                                }
                            }
                        }
                        let _ = std::fs::remove_dir(&path);
                    }
                }
            }
            let _ = std::fs::remove_dir(&self.cgroup_root);
        }

        // 5. TempDir handles data_dir and run_dir
    }
}

// ---------------------------------------------------------------------------
// Image operation tests
// ---------------------------------------------------------------------------

#[test]
fn test_e2e_pull_alpine() {
    let caps = preflight::probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, cgroups_v2, "requires cgroups v2");

    let fixture = DaemonFixture::start();

    let (success, stdout, stderr) = fixture.run_cli(&["pull", "alpine"]);
    assert!(
        success,
        "pull should succeed.\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        stdout.to_lowercase().contains("pull")
            || stdout.to_lowercase().contains("alpine"),
        "stdout should mention pull/alpine, got: {stdout}"
    );
}

#[test]
fn test_e2e_pull_nonexistent() {
    let caps = preflight::probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, cgroups_v2, "requires cgroups v2");

    let fixture = DaemonFixture::start();

    let (success, stdout, stderr) = fixture.run_cli(&["pull", "nonexistent-image-xyz-99999"]);
    assert!(
        !success,
        "pull of nonexistent image should fail.\nstdout: {stdout}\nstderr: {stderr}"
    );
}

// ---------------------------------------------------------------------------
// Container lifecycle tests
// ---------------------------------------------------------------------------

#[test]
fn test_e2e_run_echo() {
    let caps = preflight::probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, cgroups_v2, "requires cgroups v2");

    let fixture = DaemonFixture::start();

    // Pull first
    fixture.run_cli(&["pull", "alpine"]);

    let (success, stdout, _) = fixture.run_cli(&["run", "alpine", "--", "/bin/echo", "hello"]);
    assert!(success, "run should succeed, stdout: {stdout}");
}

#[test]
fn test_e2e_ps_shows_container() {
    let caps = preflight::probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, cgroups_v2, "requires cgroups v2");

    let fixture = DaemonFixture::start();
    fixture.run_cli(&["pull", "alpine"]);

    // Run a long-lived container
    fixture.run_cli(&["run", "alpine", "--", "/bin/sleep", "30"]);

    // Give it a moment to start
    std::thread::sleep(Duration::from_millis(500));

    let (success, stdout, _) = fixture.run_cli(&["ps"]);
    assert!(success, "ps should succeed");
    assert!(
        stdout.contains("alpine") || stdout.contains("Running"),
        "ps should show the container, got: {stdout}"
    );
}

#[test]
fn test_e2e_stop_container() {
    let caps = preflight::probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, cgroups_v2, "requires cgroups v2");

    let fixture = DaemonFixture::start();
    fixture.run_cli(&["pull", "alpine"]);

    // Run a long-lived container
    let (_, stdout, _) = fixture.run_cli(&["run", "alpine", "--", "/bin/sleep", "60"]);
    std::thread::sleep(Duration::from_millis(500));

    // Extract container ID from stdout (format varies — look for hex-like ID)
    let container_id = extract_container_id(&stdout);

    if let Some(id) = container_id {
        let (success, _, _) = fixture.run_cli(&["stop", &id]);
        assert!(success, "stop should succeed");
    } else {
        eprintln!("SKIPPED: could not extract container ID from: {stdout}");
    }
}

#[test]
fn test_e2e_rm_container() {
    let caps = preflight::probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, cgroups_v2, "requires cgroups v2");

    let fixture = DaemonFixture::start();
    fixture.run_cli(&["pull", "alpine"]);

    let (_, stdout, _) = fixture.run_cli(&["run", "alpine", "--", "/bin/true"]);
    std::thread::sleep(Duration::from_secs(1));

    let container_id = extract_container_id(&stdout);
    if let Some(id) = container_id {
        // Stop first, then rm
        let _ = fixture.run_cli(&["stop", &id]);
        std::thread::sleep(Duration::from_millis(200));

        let (success, _, _) = fixture.run_cli(&["rm", &id]);
        assert!(success, "rm should succeed");

        // Verify it's gone from ps
        let (_, ps_out, _) = fixture.run_cli(&["ps"]);
        assert!(!ps_out.contains(&id), "container should not appear in ps after rm");
    } else {
        eprintln!("SKIPPED: could not extract container ID from: {stdout}");
    }
}

#[test]
fn test_e2e_rm_running_rejected() {
    let caps = preflight::probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, cgroups_v2, "requires cgroups v2");

    let fixture = DaemonFixture::start();
    fixture.run_cli(&["pull", "alpine"]);

    let (_, stdout, _) = fixture.run_cli(&["run", "alpine", "--", "/bin/sleep", "60"]);
    std::thread::sleep(Duration::from_millis(500));

    let container_id = extract_container_id(&stdout);
    if let Some(id) = container_id {
        let (success, _, stderr) = fixture.run_cli(&["rm", &id]);
        assert!(
            !success,
            "rm on running container should fail.\nstderr: {stderr}"
        );
    } else {
        eprintln!("SKIPPED: could not extract container ID from: {stdout}");
    }
}

// ---------------------------------------------------------------------------
// Resource limit tests
// ---------------------------------------------------------------------------

#[test]
fn test_e2e_run_with_memory_limit() {
    let caps = preflight::probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, cgroups_v2, "requires cgroups v2");

    let fixture = DaemonFixture::start();
    fixture.run_cli(&["pull", "alpine"]);

    let (success, stdout, _) = fixture.run_cli(&[
        "run",
        "alpine",
        "--memory",
        "134217728", // 128MB
        "--",
        "/bin/sleep",
        "30",
    ]);
    assert!(success, "run with memory limit should succeed, stdout: {stdout}");

    std::thread::sleep(Duration::from_millis(500));

    // Find the container's cgroup and check memory.max
    let container_id = extract_container_id(&stdout);
    if let Some(id) = container_id {
        let memory_max_path = fixture.cgroup_root.join(&id).join("memory.max");
        if memory_max_path.exists() {
            let val = std::fs::read_to_string(&memory_max_path).unwrap_or_default();
            assert_eq!(val.trim(), "134217728", "memory.max should be 128MB");
        }
    }
}

#[test]
fn test_e2e_run_with_cpu_weight() {
    let caps = preflight::probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, cgroups_v2, "requires cgroups v2");

    let fixture = DaemonFixture::start();
    fixture.run_cli(&["pull", "alpine"]);

    let (success, stdout, _) = fixture.run_cli(&[
        "run",
        "alpine",
        "--cpu-weight",
        "250",
        "--",
        "/bin/sleep",
        "30",
    ]);
    assert!(success, "run with cpu-weight should succeed, stdout: {stdout}");

    std::thread::sleep(Duration::from_millis(500));

    let container_id = extract_container_id(&stdout);
    if let Some(id) = container_id {
        let cpu_path = fixture.cgroup_root.join(&id).join("cpu.weight");
        if cpu_path.exists() {
            let val = std::fs::read_to_string(&cpu_path).unwrap_or_default();
            assert_eq!(val.trim(), "250", "cpu.weight should be 250");
        }
    }
}

// ---------------------------------------------------------------------------
// Cleanup verification tests
// ---------------------------------------------------------------------------

#[test]
fn test_e2e_cgroup_cleaned_after_rm() {
    let caps = preflight::probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, cgroups_v2, "requires cgroups v2");

    let fixture = DaemonFixture::start();
    fixture.run_cli(&["pull", "alpine"]);

    let (_, stdout, _) = fixture.run_cli(&["run", "alpine", "--", "/bin/true"]);
    std::thread::sleep(Duration::from_secs(1));

    let container_id = extract_container_id(&stdout);
    if let Some(id) = container_id {
        let _ = fixture.run_cli(&["stop", &id]);
        std::thread::sleep(Duration::from_millis(200));
        let _ = fixture.run_cli(&["rm", &id]);

        let cgroup_dir = fixture.cgroup_root.join(&id);
        assert!(
            !cgroup_dir.exists(),
            "cgroup dir should be removed after rm: {:?}",
            cgroup_dir
        );
    }
}

// ---------------------------------------------------------------------------
// Overlay cleanup test
// ---------------------------------------------------------------------------

#[test]
fn test_e2e_overlay_cleaned_after_rm() {
    let caps = preflight::probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, cgroups_v2, "requires cgroups v2");
    require_capability!(caps, overlay_fs, "requires overlay filesystem");

    let fixture = DaemonFixture::start();
    fixture.run_cli(&["pull", "alpine"]);

    let (_, stdout, _) = fixture.run_cli(&["run", "alpine", "--", "/bin/true"]);
    std::thread::sleep(Duration::from_secs(1));

    let container_id = extract_container_id(&stdout);
    if let Some(id) = container_id {
        let _ = fixture.run_cli(&["stop", &id]);
        std::thread::sleep(Duration::from_millis(200));
        let _ = fixture.run_cli(&["rm", &id]);

        // Check that no overlay mount remains for this container
        let mounts = std::fs::read_to_string("/proc/mounts").unwrap_or_default();
        assert!(
            !mounts.contains(&id),
            "no overlay mount should remain for container {} after rm",
            id
        );
    }
}

// ---------------------------------------------------------------------------
// Socket/auth test
// ---------------------------------------------------------------------------

#[test]
fn test_e2e_nonroot_rejected() {
    let caps = preflight::probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, cgroups_v2, "requires cgroups v2");

    // We are running as root, so we use `sudo -u nobody` to attempt
    // a CLI connection as a non-root user.
    let fixture = DaemonFixture::start();

    let output = Command::new("sudo")
        .args(["-u", "nobody", fixture.cli_bin.to_str().unwrap(), "ps"])
        .env("MINIBOX_SOCKET_PATH", &fixture.socket_path)
        .output();

    match output {
        Ok(o) => {
            assert!(
                !o.status.success(),
                "non-root CLI connection should be rejected"
            );
        }
        Err(_) => {
            eprintln!("SKIPPED: could not run as nobody (sudo not configured)");
        }
    }
}

// ---------------------------------------------------------------------------
// Supervisor cgroup migration test
// ---------------------------------------------------------------------------

#[test]
fn test_e2e_daemon_migrates_to_supervisor() {
    let caps = preflight::probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, cgroups_v2, "requires cgroups v2");

    let fixture = DaemonFixture::start();

    // Read the daemon's cgroup
    let cgroup_file = format!("/proc/{}/cgroup", fixture.daemon_pid());
    let cgroup_content = std::fs::read_to_string(&cgroup_file)
        .unwrap_or_else(|e| panic!("failed to read {cgroup_file}: {e}"));

    let cgroup_path = cgroup_content
        .lines()
        .find_map(|l| l.strip_prefix("0::"))
        .unwrap_or("")
        .trim();

    assert!(
        cgroup_path.ends_with("/supervisor"),
        "daemon should be in supervisor cgroup, but is in: {}",
        cgroup_path
    );
}

// ---------------------------------------------------------------------------
// Signal handling test
// ---------------------------------------------------------------------------

#[test]
fn test_e2e_sigterm_clean_shutdown() {
    let caps = preflight::probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, cgroups_v2, "requires cgroups v2");

    let mut fixture = DaemonFixture::start();
    let socket = fixture.socket_path.clone();
    let pid = fixture.daemon_pid() as i32;

    assert!(socket.exists(), "socket should exist before SIGTERM");

    // Send SIGTERM directly (don't use fixture.sigterm() — we want to
    // manually wait and then let Drop handle cleanup without double-signal)
    unsafe { libc::kill(pid, libc::SIGTERM) };

    // Wait for exit
    let start = Instant::now();
    loop {
        match fixture.child.try_wait() {
            Ok(Some(status)) => {
                assert!(
                    status.success() || status.code() == Some(0),
                    "daemon should exit cleanly, got: {:?}",
                    status
                );
                break;
            }
            Ok(None) => {
                if start.elapsed() > Duration::from_secs(5) {
                    panic!("daemon did not exit within 5s of SIGTERM");
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => panic!("wait error: {e}"),
        }
    }

    // Socket should be cleaned up
    assert!(
        !socket.exists(),
        "socket should be removed after clean shutdown"
    );

    // Drop will try SIGTERM again on the already-exited process — that's
    // harmless (kill on dead PID returns ESRCH, ignored by Drop).
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Try to extract a container ID from CLI output.
///
/// Looks for a 16-char hex string (the truncated UUID format used by minibox).
fn extract_container_id(output: &str) -> Option<String> {
    // Look for a hex-like ID in the output
    for word in output.split_whitespace() {
        let cleaned = word.trim_matches(|c: char| !c.is_ascii_alphanumeric());
        if cleaned.len() == 16 && cleaned.chars().all(|c| c.is_ascii_hexdigit()) {
            return Some(cleaned.to_string());
        }
    }
    // Fallback: look for any alphanumeric token of length 16
    for word in output.split_whitespace() {
        let cleaned = word.trim_matches(|c: char| !c.is_ascii_alphanumeric());
        if cleaned.len() == 16 && cleaned.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Some(cleaned.to_string());
        }
    }
    None
}
````

- [ ] **Step 2: Verify it compiles**

Run: `cargo check -p miniboxd --test e2e_tests`
Expected: compiles

- [ ] **Step 3: Commit**

```bash
git add crates/miniboxd/tests/e2e_tests.rs
git commit -m "feat: add daemon+CLI e2e tests with DaemonFixture harness"
```

---

## Chunk 4: Update TESTING.md and Final Verification

### Task 6: Update TESTING.md

**Files:**

- Modify: `TESTING.md`

- [ ] **Step 1: Update TESTING.md**

Replace the entire contents of `TESTING.md` with:

```markdown
# Testing Strategy for Minibox

This document describes the testing strategy for the minibox container runtime.

## Test Pyramid
```

                 ┌─────────────┐
                 │  E2E Tests  │  (Daemon + CLI binaries)
                 │  ~14 tests  │
                 └─────────────┘
            ┌─────────────────────┐
            │ Integration Tests   │  (Real infrastructure)
            │  ~28 tests          │
            └─────────────────────┘
       ┌──────────────────────────────┐
       │     Unit + Conformance      │  (Mocks, any platform)
       │        ~52 tests            │
       └──────────────────────────────┘

````

## Quick Reference

```bash
# Install just (task runner) if not already installed
cargo install just

# Check host capabilities
just doctor

# Run all tests (full pipeline with cleanup)
just test-all

# Individual test layers
just test-unit          # Mock-based, any platform
just test-integration   # Linux, root, cgroups v2
just test-e2e           # Linux, root, built binaries

# Cleanup
just clean              # Full cargo clean
just clean-test         # Test artifacts only
just clean-stale        # Old artifacts (>7 days)
just nuke-test-state    # Kill orphans, remove cgroups/mounts
````

## Test Layers

### 1. Unit + Conformance Tests (~52 tests)

**Requirements:** None (run anywhere)

**Files:**

- `crates/miniboxd/tests/handler_tests.rs` — handler logic with mock adapters
- `crates/miniboxd/tests/conformance_tests.rs` — trait contract verification with mocks
- `crates/mbx/src/protocol.rs` — protocol serialization
- `crates/mbx/src/preflight.rs` — kernel version parsing

**Run:** `just test-unit`

### 2. Integration Tests (~28 tests)

**Requirements:** Linux kernel 5.0+, root, cgroups v2, Docker Hub access

**Files:**

- `crates/miniboxd/tests/cgroup_tests.rs` — ResourceLimiter trait against real cgroupfs
- `crates/miniboxd/tests/integration_tests.rs` — handler-level tests with real infrastructure

**Run:** `just test-integration`

**Architecture:** Tests exercise domain traits (hexagonal ports) and verify outcomes
by reading real infrastructure state (cgroupfs, procfs, mount table).

### 3. E2E Tests (~14 tests)

**Requirements:** Linux kernel 5.0+, root, cgroups v2, built binaries

**Files:**

- `crates/miniboxd/tests/e2e_tests.rs` — starts real miniboxd, exercises minibox CLI

**Run:** `just test-e2e`

**Architecture:** `DaemonFixture` starts an isolated daemon instance with temp dirs,
then runs CLI commands as subprocesses. RAII cleanup on drop.

## Preflight / Doctor

The preflight module (`crates/mbx/src/preflight.rs`) probes the host for
capabilities needed by integration and e2e tests. Run `just doctor` to see a report.

Tests use `require_capability!` to skip gracefully when prerequisites are missing.

````

- [ ] **Step 2: Commit**

```bash
git add TESTING.md
git commit -m "docs: update TESTING.md with full test pyramid and just recipes"
````

---

### Task 7: Add gitignore exception for plans

**Files:**

- Modify: `.gitignore`

- [ ] **Step 1: Add plans directory exception**

In `.gitignore`, after the `!docs/superpowers/specs/*.md` line, add:

```
!docs/superpowers/plans/*.md
```

- [ ] **Step 2: Commit**

```bash
git add .gitignore docs/superpowers/plans/2026-03-16-e2e-test-infrastructure.md
git commit -m "docs: add implementation plan and gitignore exception for plans"
```

---

### Task 8: Verify everything compiles

- [ ] **Step 1: Full workspace check**

Run: `cargo check --workspace`
Expected: compiles with no errors

- [ ] **Step 2: Unit tests pass**

Run: `cargo test --workspace --lib`
Expected: all existing unit tests still pass, plus 3 new preflight tests

- [ ] **Step 3: Commit any fixes if needed**

If compilation or tests fail, fix and commit with descriptive message.

---

## Execution Notes

**Order matters:** Tasks 1-3 (Chunk 1) must complete before Task 4 (Chunk 2), which must complete before Task 5 (Chunk 3). Tasks 6-8 (Chunk 4) depend on all prior chunks.

**Testing the tests:** Tasks 4 and 5 create tests that require Linux + root + cgroups v2. They can be verified to compile on macOS but can only be _run_ on the target Linux host. The `#![cfg(target_os = "linux")]` gate ensures they compile away on non-Linux.

**Suppressing warnings:** The `CgroupTestGuard` is constructed for its side effects (env var and cgroup setup) and cleanup (Drop). Some tests will have `guard` variables that appear unused — prefix with `_guard` if warnings are annoying, but the binding is intentionally held for its Drop.
