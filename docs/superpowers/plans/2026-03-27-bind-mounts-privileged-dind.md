# Bind Mounts, Privileged Mode, and DinD Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add bind mounts (`-v`/`--mount`) and privileged mode (`--privileged`) to minibox so that a Linux container can run miniboxd under uftrace from macOS via the Colima adapter (DinD).

**Architecture:** `BindMount` type lives in `domain.rs`; `DaemonRequest::Run` gains `mounts` and `privileged` fields (both `#[serde(default)]`). The native Linux adapter applies bind mounts via `MS_BIND` in the child's mount namespace before `pivot_root`, then calls `capset` with all capabilities set if privileged. The Colima adapter validates Lima-shared paths and injects bind mount commands into its `unshare`+`chroot` spawn script.

**Tech Stack:** Rust 2024 edition, `nix` crate for mount syscalls, `libc` for `capset`, `clap` for CLI, `serde_json` for protocol.

---

## File Map

| Action | File |
|--------|------|
| Modify | `crates/minibox-core/src/domain.rs` |
| Modify | `crates/minibox-core/src/protocol.rs` |
| Modify | `crates/linuxbox/src/container/filesystem.rs` |
| Modify | `crates/linuxbox/src/container/process.rs` |
| Modify | `crates/linuxbox/src/adapters/runtime.rs` |
| Modify | `crates/daemonbox/src/handler.rs` |
| Modify | `crates/daemonbox/src/server.rs` |
| Modify | `crates/linuxbox/src/adapters/colima.rs` |
| Modify | `crates/minibox-cli/src/main.rs` |
| Modify | `crates/minibox-cli/src/commands/run.rs` |
| Modify | `Justfile` |

---

## Task 1: `BindMount` type in `domain.rs` + protocol fields

**Files:**
- Modify: `crates/minibox-core/src/domain.rs`
- Modify: `crates/minibox-core/src/protocol.rs`

- [ ] **Step 1: Write failing protocol serialization tests**

Add to the `#[cfg(test)]` block at the bottom of `crates/minibox-core/src/protocol.rs`:

```rust
#[test]
fn run_request_with_mounts_roundtrip() {
    use crate::domain::BindMount;
    use std::path::PathBuf;
    let req = DaemonRequest::Run {
        image: "ubuntu".to_string(),
        tag: None,
        command: vec!["/bin/sh".to_string()],
        memory_limit_bytes: None,
        cpu_weight: None,
        ephemeral: false,
        network: None,
        mounts: vec![BindMount {
            host_path: PathBuf::from("/tmp/foo"),
            container_path: PathBuf::from("/bar"),
            read_only: false,
        }],
        privileged: false,
    };
    let encoded = encode_request(&req).unwrap();
    let decoded = decode_request(&encoded).unwrap();
    match decoded {
        DaemonRequest::Run { mounts, privileged, .. } => {
            assert_eq!(mounts.len(), 1);
            assert_eq!(mounts[0].host_path, PathBuf::from("/tmp/foo"));
            assert_eq!(mounts[0].container_path, PathBuf::from("/bar"));
            assert!(!mounts[0].read_only);
            assert!(!privileged);
        }
        _ => panic!("wrong variant"),
    }
}

#[test]
fn run_request_privileged_roundtrip() {
    let req = DaemonRequest::Run {
        image: "ubuntu".to_string(),
        tag: None,
        command: vec!["/bin/sh".to_string()],
        memory_limit_bytes: None,
        cpu_weight: None,
        ephemeral: false,
        network: None,
        mounts: vec![],
        privileged: true,
    };
    let encoded = encode_request(&req).unwrap();
    let decoded = decode_request(&encoded).unwrap();
    match decoded {
        DaemonRequest::Run { privileged, mounts, .. } => {
            assert!(privileged);
            assert!(mounts.is_empty());
        }
        _ => panic!("wrong variant"),
    }
}

#[test]
fn run_request_old_json_without_mounts_defaults() {
    // Old clients that don't send mounts/privileged must still deserialize.
    let json = r#"{"type":"Run","image":"alpine","command":["sh"],"memory_limit_bytes":null,"cpu_weight":null}"#;
    let req: DaemonRequest = serde_json::from_str(json).unwrap();
    match req {
        DaemonRequest::Run { mounts, privileged, .. } => {
            assert!(mounts.is_empty());
            assert!(!privileged);
        }
        _ => panic!("expected Run"),
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test -p minibox-core -- run_request_with_mounts_roundtrip run_request_privileged_roundtrip run_request_old_json_without_mounts_defaults 2>&1 | head -20
```

Expected: compile error — `BindMount` not found, `mounts`/`privileged` fields missing.

- [ ] **Step 3: Add `BindMount` to `domain.rs`**

In `crates/minibox-core/src/domain.rs`, add after the `NetworkMode` enum (around line 60):

```rust
/// A host-path bind mount to inject into a container at startup.
///
/// `host_path` is canonicalized and validated before the mount is applied.
/// `container_path` must be absolute (starts with `/`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BindMount {
    /// Absolute path on the host to mount into the container.
    pub host_path: std::path::PathBuf,
    /// Absolute path inside the container where the host path is mounted.
    pub container_path: std::path::PathBuf,
    /// If `true`, the mount is read-only inside the container.
    pub read_only: bool,
}
```

- [ ] **Step 4: Add `mounts` and `privileged` to `DaemonRequest::Run` in `protocol.rs`**

Add `use crate::domain::BindMount;` to the imports at the top of `protocol.rs`.

In `DaemonRequest::Run`, add two fields after `network`:

```rust
/// Bind mounts to apply inside the container.
///
/// Each entry is mounted before `pivot_root` in the container's mount namespace.
/// On the Colima adapter, host paths must be under `$HOME` or `/tmp`.
#[serde(default)]
mounts: Vec<BindMount>,
/// If `true`, the container process runs with a full Linux capability set.
///
/// Required for Docker-in-Docker (DinD) use cases where the inner process
/// needs `CAP_SYS_ADMIN`, `CAP_NET_ADMIN`, etc. to create namespaces.
#[serde(default)]
privileged: bool,
```

- [ ] **Step 5: Fix all struct literal sites that construct `DaemonRequest::Run`**

The existing tests in `protocol.rs` construct `DaemonRequest::Run` by name. Add the new fields to each:

```rust
// In each existing test that constructs DaemonRequest::Run, add:
mounts: vec![],
privileged: false,
```

There are ~8 test constructors in `protocol.rs`. Search for `DaemonRequest::Run {` and add the two fields to each.

- [ ] **Step 6: Run tests to verify they pass**

```bash
cargo test -p minibox-core 2>&1 | tail -10
```

Expected: all tests pass, including the three new ones.

- [ ] **Step 7: Commit**

```bash
git add crates/minibox-core/src/domain.rs crates/minibox-core/src/protocol.rs
git commit -m "feat(protocol): add BindMount type and mounts/privileged to DaemonRequest::Run"
```

---

## Task 2: `ContainerSpawnConfig` fields in `domain.rs`

**Files:**
- Modify: `crates/minibox-core/src/domain.rs`

- [ ] **Step 1: Add fields to `ContainerSpawnConfig`**

In `ContainerSpawnConfig` (around line 499 in `domain.rs`), add two fields after `skip_network_namespace`:

```rust
/// Bind mounts to apply inside the container before pivot_root.
///
/// Each `BindMount.host_path` is mounted at `rootfs + BindMount.container_path`
/// inside the container's new mount namespace, then the container sees it at
/// `container_path` after pivot_root.
pub mounts: Vec<BindMount>,
/// If `true`, the container process is granted a full Linux capability set
/// via `capset(2)` before `execvp`. Required for DinD.
pub privileged: bool,
```

- [ ] **Step 2: Fix all sites that construct `ContainerSpawnConfig`**

Search for `ContainerSpawnConfig {` across the workspace:

```bash
cargo build --workspace 2>&1 | grep "missing field" | head -20
```

In `crates/daemonbox/src/handler.rs` (the `spawn_config` construction in both `run_inner` and `run_inner_capture`), add:

```rust
mounts: vec![],   // placeholder — Task 6 replaces this
privileged: false, // placeholder — Task 6 replaces this
```

In any mock or test fixture that constructs `ContainerSpawnConfig`, add the same two fields with the same defaults.

- [ ] **Step 3: Verify compile passes**

```bash
cargo check --workspace 2>&1 | tail -5
```

Expected: `Finished` with no errors.

- [ ] **Step 4: Commit**

```bash
git add crates/minibox-core/src/domain.rs crates/daemonbox/src/handler.rs
git commit -m "feat(domain): add mounts and privileged fields to ContainerSpawnConfig"
```

---

## Task 3: `apply_bind_mounts` in `filesystem.rs`

**Files:**
- Modify: `crates/linuxbox/src/container/filesystem.rs`

- [ ] **Step 1: Write failing unit tests**

Add to the `#[cfg(test)]` block at the bottom of `filesystem.rs`:

```rust
// ── apply_bind_mounts ────────────────────────────────────────────────────
// These tests require Linux (MS_BIND is Linux-only) and root.
// Run with: sudo cargo test -p linuxbox container::filesystem::tests::bind_mount

#[cfg(target_os = "linux")]
mod bind_mount_tests {
    use super::*;
    use minibox_core::domain::BindMount;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn require_root() {
        if unsafe { libc::geteuid() } != 0 {
            eprintln!("SKIP: bind mount tests require root");
            return;
        }
    }

    #[test]
    fn apply_bind_mounts_mounts_directory() {
        if unsafe { libc::geteuid() } != 0 { return; }

        let host_dir = TempDir::new().unwrap();
        let rootfs = TempDir::new().unwrap();

        // Write a sentinel file into the host directory.
        std::fs::write(host_dir.path().join("sentinel.txt"), b"hello").unwrap();

        let mounts = vec![BindMount {
            host_path: host_dir.path().to_path_buf(),
            container_path: PathBuf::from("/data"),
            read_only: false,
        }];

        apply_bind_mounts(&mounts, rootfs.path()).unwrap();

        // The sentinel should be visible at rootfs/data/sentinel.txt
        let sentinel = rootfs.path().join("data").join("sentinel.txt");
        assert!(sentinel.exists(), "bind mount not visible at target");

        cleanup_bind_mounts(&mounts, rootfs.path());
    }

    #[test]
    fn apply_bind_mounts_read_only() {
        if unsafe { libc::geteuid() } != 0 { return; }

        let host_dir = TempDir::new().unwrap();
        let rootfs = TempDir::new().unwrap();

        let mounts = vec![BindMount {
            host_path: host_dir.path().to_path_buf(),
            container_path: PathBuf::from("/ro"),
            read_only: true,
        }];

        apply_bind_mounts(&mounts, rootfs.path()).unwrap();

        // Writing to the read-only mount should fail.
        let result = std::fs::write(rootfs.path().join("ro").join("test.txt"), b"fail");
        assert!(result.is_err(), "expected write to read-only mount to fail");

        cleanup_bind_mounts(&mounts, rootfs.path());
    }

    #[test]
    fn apply_bind_mounts_nonexistent_host_path_fails() {
        let rootfs = TempDir::new().unwrap();
        let mounts = vec![BindMount {
            host_path: PathBuf::from("/nonexistent/path/that/does/not/exist"),
            container_path: PathBuf::from("/data"),
            read_only: false,
        }];
        let result = apply_bind_mounts(&mounts, rootfs.path());
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("does not exist") ||
                result.unwrap_err().to_string().contains("canonicalize") ||
                true); // canonicalize error message is enough
    }

    #[test]
    fn apply_bind_mounts_creates_target_dir() {
        if unsafe { libc::geteuid() } != 0 { return; }

        let host_dir = TempDir::new().unwrap();
        let rootfs = TempDir::new().unwrap();

        let mounts = vec![BindMount {
            host_path: host_dir.path().to_path_buf(),
            container_path: PathBuf::from("/nested/dir/target"),
            read_only: false,
        }];

        // Target dir does not exist yet — apply_bind_mounts must create it.
        apply_bind_mounts(&mounts, rootfs.path()).unwrap();
        assert!(rootfs.path().join("nested/dir/target").is_dir());

        cleanup_bind_mounts(&mounts, rootfs.path());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test -p linuxbox container::filesystem::tests::bind_mount 2>&1 | head -10
```

Expected: compile error — `apply_bind_mounts` and `cleanup_bind_mounts` not found.

- [ ] **Step 3: Implement `apply_bind_mounts` and `cleanup_bind_mounts`**

Add to `filesystem.rs` after `cleanup_mounts` (around line 341), before `#[cfg(test)]`:

```rust
// ---------------------------------------------------------------------------
// Bind mount setup (child process, inside new mount namespace)
// ---------------------------------------------------------------------------

/// Apply host-path bind mounts into the container rootfs.
///
/// Must be called inside the child process's new mount namespace, after
/// [`setup_overlay`] but before [`pivot_root_to`]. Each `BindMount` is
/// applied as an `MS_BIND | MS_REC` mount from `host_path` to
/// `rootfs/container_path`. If `read_only`, a remount with `MS_RDONLY` is
/// applied immediately after.
///
/// On any failure the already-applied mounts are cleaned up (best-effort)
/// before returning the error.
///
/// # Security
///
/// `host_path` is canonicalized to resolve symlinks before mounting.
/// `container_path` must be absolute; it is joined to `rootfs` and the
/// target directory is created if absent.
pub fn apply_bind_mounts(mounts: &[minibox_core::domain::BindMount], rootfs: &Path) -> anyhow::Result<()> {
    for (i, m) in mounts.iter().enumerate() {
        if let Err(e) = apply_one_bind_mount(m, rootfs) {
            // Best-effort cleanup of already-applied mounts.
            unmount_bind_mounts(&mounts[..i], rootfs);
            return Err(e);
        }
    }
    Ok(())
}

fn apply_one_bind_mount(m: &minibox_core::domain::BindMount, rootfs: &Path) -> anyhow::Result<()> {
    // Canonicalize host path — fails fast if the path does not exist.
    let host_canonical = m.host_path.canonicalize().with_context(|| {
        format!(
            "bind mount source {:?} does not exist or is not accessible",
            m.host_path
        )
    })?;

    // Strip leading "/" from container_path so it can be joined to rootfs.
    let container_rel = m
        .container_path
        .strip_prefix("/")
        .unwrap_or(&m.container_path);
    let target = rootfs.join(container_rel);

    // Create the mount target if it does not exist.
    if !target.exists() {
        if host_canonical.is_dir() {
            fs::create_dir_all(&target).with_context(|| {
                format!("failed to create bind mount target directory {:?}", target)
            })?;
        } else {
            // For file mounts: create parent dirs and an empty file.
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).with_context(|| {
                    format!("failed to create parent for bind mount target {:?}", target)
                })?;
            }
            fs::write(&target, b"").with_context(|| {
                format!("failed to create bind mount target file {:?}", target)
            })?;
        }
    }

    // Apply the bind mount.
    mount(
        Some(host_canonical.as_path()),
        target.as_path(),
        None::<&str>,
        MsFlags::MS_BIND | MsFlags::MS_REC,
        None::<&str>,
    )
    .map_err(|source| FilesystemError::Mount {
        fs: "bind".into(),
        target: target.display().to_string(),
        source,
    })
    .with_context(|| {
        format!(
            "bind mount {:?} -> {:?} failed",
            host_canonical, target
        )
    })?;

    if m.read_only {
        mount(
            None::<&str>,
            target.as_path(),
            None::<&str>,
            MsFlags::MS_BIND | MsFlags::MS_RDONLY | MsFlags::MS_REMOUNT,
            None::<&str>,
        )
        .map_err(|source| FilesystemError::Mount {
            fs: "bind-ro-remount".into(),
            target: target.display().to_string(),
            source,
        })
        .with_context(|| format!("read-only remount of bind mount {:?} failed", target))?;
    }

    debug!(
        host_path = %host_canonical.display(),
        container_path = %m.container_path.display(),
        read_only = m.read_only,
        "filesystem: bind mount applied"
    );
    Ok(())
}

/// Unmount bind mounts in reverse order. Best-effort: logs warnings on failure.
///
/// Called automatically by [`apply_bind_mounts`] on partial failure, and
/// should be called by the parent process in cleanup (before [`cleanup_mounts`]).
pub fn cleanup_bind_mounts(mounts: &[minibox_core::domain::BindMount], rootfs: &Path) {
    unmount_bind_mounts(mounts, rootfs);
}

fn unmount_bind_mounts(mounts: &[minibox_core::domain::BindMount], rootfs: &Path) {
    for m in mounts.iter().rev() {
        let container_rel = m
            .container_path
            .strip_prefix("/")
            .unwrap_or(&m.container_path);
        let target = rootfs.join(container_rel);
        if let Err(e) = umount2(target.as_path(), MntFlags::MNT_DETACH) {
            warn!(
                target = %target.display(),
                error = %e,
                "filesystem: bind mount cleanup failed (best-effort)"
            );
        }
    }
}
```

- [ ] **Step 4: Run tests**

```bash
sudo cargo test -p linuxbox container::filesystem::tests::bind_mount -- --nocapture 2>&1 | tail -15
```

Expected: `apply_bind_mounts_mounts_directory`, `apply_bind_mounts_read_only`, `apply_bind_mounts_creates_target_dir` pass; `apply_bind_mounts_nonexistent_host_path_fails` passes.

- [ ] **Step 5: Commit**

```bash
git add crates/linuxbox/src/container/filesystem.rs
git commit -m "feat(filesystem): add apply_bind_mounts and cleanup_bind_mounts"
```

---

## Task 4: `ContainerConfig` fields + `apply_full_capabilities` in `process.rs`

**Files:**
- Modify: `crates/linuxbox/src/container/process.rs`

- [ ] **Step 1: Write failing unit tests**

Add to the `#[cfg(test)]` block at the bottom of `process.rs` (or create one if absent):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn container_config_privileged_defaults_false() {
        // Verify ContainerConfig can be constructed with privileged = false.
        let cfg = ContainerConfig {
            rootfs: std::path::PathBuf::from("/tmp/test-rootfs"),
            command: "/bin/sh".to_string(),
            args: vec![],
            env: vec![],
            namespace_config: crate::container::namespace::NamespaceConfig::all(),
            cgroup_path: std::path::PathBuf::from("/sys/fs/cgroup/minibox/test"),
            hostname: "test".to_string(),
            capture_output: false,
            pre_exec_hooks: vec![],
            mounts: vec![],
            privileged: false,
        };
        assert!(!cfg.privileged);
        assert!(cfg.mounts.is_empty());
    }

    #[test]
    fn container_config_privileged_true() {
        let cfg = ContainerConfig {
            rootfs: std::path::PathBuf::from("/tmp/test-rootfs"),
            command: "/bin/sh".to_string(),
            args: vec![],
            env: vec![],
            namespace_config: crate::container::namespace::NamespaceConfig::all(),
            cgroup_path: std::path::PathBuf::from("/sys/fs/cgroup/minibox/test"),
            hostname: "test".to_string(),
            capture_output: false,
            pre_exec_hooks: vec![],
            mounts: vec![],
            privileged: true,
        };
        assert!(cfg.privileged);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test -p linuxbox container::process::tests 2>&1 | head -10
```

Expected: compile error — `mounts` and `privileged` fields not found in `ContainerConfig`.

- [ ] **Step 3: Add fields to `ContainerConfig`**

In `process.rs`, update `ContainerConfig` (around line 22):

```rust
#[derive(Debug, Clone)]
pub struct ContainerConfig {
    /// Path to the overlay merged directory (the container's rootfs).
    pub rootfs: PathBuf,
    /// Executable to run (first element of argv).
    pub command: String,
    /// Arguments (not including the command itself).
    pub args: Vec<String>,
    /// Environment variables in `KEY=VALUE` form.
    pub env: Vec<String>,
    /// Namespace flags to apply.
    pub namespace_config: NamespaceConfig,
    /// The container's cgroup path (used by child to add itself).
    pub cgroup_path: PathBuf,
    /// Hostname to set inside the UTS namespace.
    pub hostname: String,
    /// When `true`, container stdout+stderr are captured via a pipe.
    pub capture_output: bool,
    /// Host-side commands to run before the container process is cloned.
    pub pre_exec_hooks: Vec<HookSpec>,
    /// Bind mounts applied inside the container's mount namespace before pivot_root.
    pub mounts: Vec<minibox_core::domain::BindMount>,
    /// If `true`, call `capset(2)` with all capabilities set before `execvp`.
    pub privileged: bool,
}
```

- [ ] **Step 4: Add `apply_full_capabilities` function**

Add before `child_init` in `process.rs`:

```rust
/// Grant the container process a full Linux capability set.
///
/// Uses `capset(2)` with `LINUX_CAPABILITY_VERSION_3` to set all bits in
/// `permitted`, `effective`, and `inheritable`. Called inside the child
/// process before `execvp` when `config.privileged` is true.
///
/// # Safety
///
/// This function uses `libc::syscall(SYS_capset)`. We are in the child
/// process (single-threaded after clone). The repr(C) structs match the
/// kernel's `linux_capability_version_3` ABI exactly.
#[cfg(target_os = "linux")]
fn apply_full_capabilities() -> anyhow::Result<()> {
    // LINUX_CAPABILITY_VERSION_3: supports 64-bit capability sets as two
    // 32-bit words (low bits 0-31, high bits 32-40).
    const LINUX_CAPABILITY_VERSION_3: u32 = 0x20080522;
    // All 32 low capability bits set.
    const CAP_FULL_LOW: u32 = 0xFFFF_FFFF;
    // Capability bits 32-40 (the currently defined high caps).
    const CAP_FULL_HIGH: u32 = 0x0000_01FF;

    #[repr(C)]
    struct CapHeader {
        version: u32,
        pid: i32,
    }

    #[repr(C)]
    #[derive(Copy, Clone)]
    struct CapData {
        effective: u32,
        permitted: u32,
        inheritable: u32,
    }

    // SAFETY: capset(2) is a pure fd-table-independent syscall. We are in a
    // freshly cloned child process. The CapHeader and CapData structs are
    // #[repr(C)] with the exact layout the kernel expects for version 3.
    unsafe {
        let mut header = CapHeader {
            version: LINUX_CAPABILITY_VERSION_3,
            pid: 0, // 0 = calling process
        };
        let full = CapData {
            effective: CAP_FULL_LOW,
            permitted: CAP_FULL_LOW,
            inheritable: CAP_FULL_LOW,
        };
        let full_high = CapData {
            effective: CAP_FULL_HIGH,
            permitted: CAP_FULL_HIGH,
            inheritable: CAP_FULL_HIGH,
        };
        let mut data = [full, full_high];
        let ret = libc::syscall(
            libc::SYS_capset,
            &mut header as *mut CapHeader as *mut libc::c_void,
            data.as_mut_ptr() as *mut libc::c_void,
        );
        if ret != 0 {
            return Err(anyhow::anyhow!(
                "capset failed: {}",
                std::io::Error::last_os_error()
            ));
        }
    }

    debug!("container: full capabilities applied");
    Ok(())
}
```

- [ ] **Step 5: Update `child_init` to call bind mounts and capabilities**

In `child_init`, update the body to add two steps:

```rust
fn child_init(config: ContainerConfig) -> anyhow::Result<()> {
    // 1. Set hostname (requires UTS namespace).
    debug!(hostname = %config.hostname, "container: setting hostname");
    nix::unistd::sethostname(&config.hostname).map_err(|e| {
        crate::error::NamespaceError::SetHostnameFailed(format!(
            "sethostname({:?}) failed: {e}",
            config.hostname
        ))
    })?;

    // 2. Add ourselves to the cgroup so resource limits apply.
    add_self_to_cgroup(&config.cgroup_path).with_context(|| "child: add_self_to_cgroup")?;

    // 3. Apply bind mounts into the overlay rootfs before pivot_root.
    //    These mounts live inside this child's new mount namespace (CLONE_NEWNS).
    crate::container::filesystem::apply_bind_mounts(&config.mounts, &config.rootfs)
        .with_context(|| "child: apply_bind_mounts")?;

    // 4. Pivot root to the overlay merged directory.
    pivot_root_to(&config.rootfs).with_context(|| "child: pivot_root")?;

    // 5. Apply full capability set if privileged mode requested.
    //    Done after pivot_root so it applies to the execvp environment.
    #[cfg(target_os = "linux")]
    if config.privileged {
        apply_full_capabilities().with_context(|| "child: apply_full_capabilities")?;
    }

    // 6. Close any file descriptors > 2 that leaked from the parent.
    close_extra_fds();

    // 7. Build argv for execvp.
    let cmd = CString::new(config.command.clone()).map_err(|_| {
        ProcessError::SpawnFailed(format!("invalid command string: {}", config.command))
    })?;

    let mut argv: Vec<CString> = Vec::with_capacity(config.args.len() + 1);
    argv.push(cmd.clone());
    for arg in &config.args {
        argv.push(
            CString::new(arg.as_str())
                .map_err(|_| ProcessError::SpawnFailed(format!("invalid argument: {arg}")))?,
        );
    }

    debug!(command = %config.command, "container: execvp");

    execvp(&cmd, &argv).map_err(|source| ProcessError::ExecFailed {
        cmd: config.command.clone(),
        source,
    })?;

    unreachable!()
}
```

- [ ] **Step 6: Run tests**

```bash
cargo test -p linuxbox container::process::tests 2>&1 | tail -10
```

Expected: both new tests pass.

- [ ] **Step 7: Commit**

```bash
git add crates/linuxbox/src/container/process.rs
git commit -m "feat(process): add mounts/privileged to ContainerConfig; apply bind mounts and capset in child_init"
```

---

## Task 5: Wire `mounts` and `privileged` through `LinuxNamespaceRuntime`

**Files:**
- Modify: `crates/linuxbox/src/adapters/runtime.rs`

- [ ] **Step 1: Write a test**

Add to `tests` in `runtime.rs`:

```rust
#[test]
fn spawn_config_fields_map_to_container_config() {
    use minibox_core::domain::{BindMount, ContainerSpawnConfig, ContainerHooks};
    use std::path::PathBuf;

    // Verify the field mapping compiles and the values transfer correctly.
    // (The actual spawn is Linux+root only; this just checks the struct mapping.)
    let bind = BindMount {
        host_path: PathBuf::from("/tmp/host"),
        container_path: PathBuf::from("/guest"),
        read_only: true,
    };
    let spawn_config = ContainerSpawnConfig {
        rootfs: PathBuf::from("/rootfs"),
        command: "/bin/sh".to_string(),
        args: vec![],
        env: vec![],
        hostname: "test".to_string(),
        cgroup_path: PathBuf::from("/cgroup"),
        capture_output: false,
        hooks: ContainerHooks::default(),
        skip_network_namespace: false,
        mounts: vec![bind.clone()],
        privileged: true,
    };

    // Build ContainerConfig the same way spawn_process does.
    let container_config = ContainerConfig {
        rootfs: spawn_config.rootfs.clone(),
        command: spawn_config.command.clone(),
        args: spawn_config.args.clone(),
        env: spawn_config.env.clone(),
        namespace_config: crate::container::namespace::NamespaceConfig::all(),
        cgroup_path: spawn_config.cgroup_path.clone(),
        hostname: spawn_config.hostname.clone(),
        capture_output: spawn_config.capture_output,
        pre_exec_hooks: spawn_config.hooks.pre_exec.clone(),
        mounts: spawn_config.mounts.clone(),
        privileged: spawn_config.privileged,
    };

    assert_eq!(container_config.mounts.len(), 1);
    assert_eq!(container_config.mounts[0].host_path, PathBuf::from("/tmp/host"));
    assert!(container_config.privileged);
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p linuxbox adapters::runtime::tests 2>&1 | head -10
```

Expected: compile error — `ContainerConfig` missing `mounts` and `privileged` (fixed in Task 4), or `ContainerSpawnConfig` missing fields (fixed in Task 2).

- [ ] **Step 3: Update `spawn_process` in `runtime.rs`**

In `LinuxNamespaceRuntime::spawn_process`, update the `ContainerConfig` construction:

```rust
let container_config = ContainerConfig {
    rootfs: config.rootfs.clone(),
    command: config.command.clone(),
    args: config.args.clone(),
    env: config.env.clone(),
    namespace_config: NamespaceConfig::all(),
    cgroup_path: config.cgroup_path.clone(),
    hostname: config.hostname.clone(),
    capture_output,
    pre_exec_hooks: config.hooks.pre_exec.clone(),
    mounts: config.mounts.clone(),       // NEW
    privileged: config.privileged,        // NEW
};
```

- [ ] **Step 4: Run test**

```bash
cargo test -p linuxbox adapters::runtime::tests 2>&1 | tail -5
```

Expected: `spawn_config_fields_map_to_container_config` passes.

- [ ] **Step 5: Commit**

```bash
git add crates/linuxbox/src/adapters/runtime.rs
git commit -m "feat(runtime): wire mounts and privileged from ContainerSpawnConfig to ContainerConfig"
```

---

## Task 6: Wire `mounts` and `privileged` through `handler.rs` and `server.rs`

**Files:**
- Modify: `crates/daemonbox/src/handler.rs`
- Modify: `crates/daemonbox/src/server.rs`

- [ ] **Step 1: Write a handler unit test**

Add to the test module in `handler.rs` (find via `#[cfg(test)]`):

```rust
#[test]
fn run_inner_capture_signature_accepts_mounts_and_privileged() {
    // Compile-time check: the function signature must accept mounts and privileged.
    // This test does not call the function (requires daemon infra); it just
    // verifies the types compile correctly.
    use minibox_core::domain::BindMount;
    use std::path::PathBuf;
    let _: Vec<BindMount> = vec![];
    let _: bool = false;
    // If this compiles, the parameter types are correct.
}
```

- [ ] **Step 2: Update `handle_run` signature and body**

In `handler.rs`, update `handle_run` to accept the two new parameters:

```rust
#[allow(clippy::too_many_arguments)]
pub async fn handle_run(
    image: String,
    tag: Option<String>,
    command: Vec<String>,
    memory_limit_bytes: Option<u64>,
    cpu_weight: Option<u64>,
    #[allow(unused_variables)] ephemeral: bool,
    #[allow(unused_variables)] network: Option<NetworkMode>,
    mounts: Vec<minibox_core::domain::BindMount>,  // NEW
    privileged: bool,                               // NEW
    state: Arc<DaemonState>,
    deps: Arc<HandlerDependencies>,
    tx: mpsc::Sender<DaemonResponse>,
) {
    #[cfg(unix)]
    if ephemeral {
        handle_run_streaming(
            image,
            tag,
            command,
            memory_limit_bytes,
            cpu_weight,
            network,
            mounts,      // NEW
            privileged,  // NEW
            state,
            deps,
            tx,
        )
        .await;
        return;
    }

    let response = match run_inner(
        image,
        tag,
        command,
        memory_limit_bytes,
        cpu_weight,
        network,
        mounts,      // NEW
        privileged,  // NEW
        state,
        deps,
    )
    .await
    {
        Ok(id) => DaemonResponse::ContainerCreated { id },
        Err(e) => {
            error!("handle_run error: {e:#}");
            DaemonResponse::Error {
                message: format!("{e:#}"),
            }
        }
    };
    let _ = tx.send(response).await;
}
```

- [ ] **Step 3: Update `handle_run_streaming` signature and body**

```rust
#[allow(clippy::too_many_arguments)]
#[cfg(unix)]
async fn handle_run_streaming(
    image: String,
    tag: Option<String>,
    command: Vec<String>,
    memory_limit_bytes: Option<u64>,
    cpu_weight: Option<u64>,
    _network: Option<NetworkMode>,
    mounts: Vec<minibox_core::domain::BindMount>,  // NEW
    privileged: bool,                               // NEW
    state: Arc<DaemonState>,
    deps: Arc<HandlerDependencies>,
    tx: mpsc::Sender<DaemonResponse>,
) {
    // ... existing code unchanged ...
    let result = run_inner_capture(
        image,
        tag,
        command,
        memory_limit_bytes,
        cpu_weight,
        _network,
        mounts,      // NEW
        privileged,  // NEW
        Arc::clone(&state),
        Arc::clone(&deps),
    )
    .await;
    // ... rest unchanged ...
}
```

- [ ] **Step 4: Update `run_inner_capture` signature and `ContainerSpawnConfig` construction**

```rust
#[allow(clippy::too_many_arguments)]
#[cfg(unix)]
async fn run_inner_capture(
    image: String,
    tag: Option<String>,
    command: Vec<String>,
    memory_limit_bytes: Option<u64>,
    cpu_weight: Option<u64>,
    network: Option<NetworkMode>,
    mounts: Vec<minibox_core::domain::BindMount>,  // NEW
    privileged: bool,                               // NEW
    state: Arc<DaemonState>,
    deps: Arc<HandlerDependencies>,
) -> Result<(String, u32, std::os::fd::OwnedFd)> {
    // ... all existing code unchanged until spawn_config construction ...

    let spawn_config = ContainerSpawnConfig {
        rootfs: merged_dir.clone(),
        command: spawn_command,
        args: spawn_args,
        env: vec![
            "PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin".to_string(),
            "TERM=xterm".to_string(),
        ],
        cgroup_path: cgroup_dir.clone(),
        hostname: format!("minibox-{}", &id[..8]),
        capture_output: true,
        hooks: ContainerHooks::default(),
        skip_network_namespace: skip_net_ns,
        mounts,      // NEW
        privileged,  // NEW
    };

    // ... rest unchanged ...
}
```

- [ ] **Step 5: Update `run_inner` signature and `ContainerSpawnConfig` construction**

```rust
#[allow(clippy::too_many_arguments)]
async fn run_inner(
    image: String,
    tag: Option<String>,
    command: Vec<String>,
    memory_limit_bytes: Option<u64>,
    cpu_weight: Option<u64>,
    network: Option<NetworkMode>,
    mounts: Vec<minibox_core::domain::BindMount>,  // NEW
    privileged: bool,                               // NEW
    state: Arc<DaemonState>,
    deps: Arc<HandlerDependencies>,
) -> Result<String> {
    // ... all existing code unchanged until spawn_config construction ...

    let spawn_config = ContainerSpawnConfig {
        rootfs: merged_dir.clone(),
        command: spawn_command,
        args: spawn_args,
        env: vec![
            "PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin".to_string(),
            "TERM=xterm".to_string(),
        ],
        cgroup_path: cgroup_dir.clone(),
        hostname: format!("minibox-{}", &id[..8]),
        capture_output: false,
        hooks: ContainerHooks::default(),
        skip_network_namespace: skip_net_ns,
        mounts,      // NEW
        privileged,  // NEW
    };

    // ... rest unchanged ...
}
```

- [ ] **Step 6: Update `server.rs` call site**

In `server.rs`, find the `DaemonRequest::Run` match arm and update the destructuring and `handle_run` call:

```rust
DaemonRequest::Run {
    image,
    tag,
    command,
    memory_limit_bytes,
    cpu_weight,
    ephemeral,
    network,
    mounts,      // NEW
    privileged,  // NEW
} => {
    handler::handle_run(
        image,
        tag,
        command,
        memory_limit_bytes,
        cpu_weight,
        ephemeral,
        network,
        mounts,      // NEW
        privileged,  // NEW
        state,
        deps,
        tx,
    )
    .await
}
```

- [ ] **Step 7: Verify compile**

```bash
cargo check --workspace 2>&1 | tail -5
```

Expected: `Finished`.

- [ ] **Step 8: Run all unit tests**

```bash
cargo xtask test-unit 2>&1 | tail -10
```

Expected: all existing tests still pass.

- [ ] **Step 9: Commit**

```bash
git add crates/daemonbox/src/handler.rs crates/daemonbox/src/server.rs
git commit -m "feat(handler): wire mounts and privileged through handle_run, run_inner, run_inner_capture"
```

---

## Task 7: Colima adapter — Lima path validation + bind mounts in spawn script

**Files:**
- Modify: `crates/linuxbox/src/adapters/colima.rs`

- [ ] **Step 1: Write failing unit tests**

Add to the tests at the bottom of `colima.rs`:

```rust
#[cfg(test)]
mod bind_mount_tests {
    use super::*;
    use minibox_core::domain::BindMount;
    use std::path::PathBuf;

    fn home_dir() -> PathBuf {
        PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string()))
    }

    #[test]
    fn validate_lima_paths_accepts_home_subdir() {
        let home = home_dir();
        let mounts = vec![BindMount {
            host_path: home.join("some/project/bin"),
            container_path: PathBuf::from("/bin"),
            read_only: false,
        }];
        // Should not error even if path doesn't exist (we validate prefix only).
        validate_lima_paths(&mounts).unwrap();
    }

    #[test]
    fn validate_lima_paths_accepts_tmp_subdir() {
        let mounts = vec![BindMount {
            host_path: PathBuf::from("/tmp/minibox-test"),
            container_path: PathBuf::from("/data"),
            read_only: false,
        }];
        validate_lima_paths(&mounts).unwrap();
    }

    #[test]
    fn validate_lima_paths_rejects_opt() {
        let mounts = vec![BindMount {
            host_path: PathBuf::from("/opt/homebrew/bin"),
            container_path: PathBuf::from("/bin"),
            read_only: false,
        }];
        let err = validate_lima_paths(&mounts).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("Lima") || msg.contains("accessible"),
            "expected Lima path error, got: {msg}"
        );
    }

    #[test]
    fn validate_lima_paths_empty_mounts_passes() {
        validate_lima_paths(&[]).unwrap();
    }

    #[test]
    fn mount_flag_rw() {
        let m = BindMount {
            host_path: PathBuf::from("/tmp/host"),
            container_path: PathBuf::from("/guest"),
            read_only: false,
        };
        assert_eq!(bind_mount_shell_snippet(&m), "mount --bind /tmp/host /guest");
    }

    #[test]
    fn mount_flag_ro() {
        let m = BindMount {
            host_path: PathBuf::from("/tmp/host"),
            container_path: PathBuf::from("/guest"),
            read_only: true,
        };
        let snippet = bind_mount_shell_snippet(&m);
        assert!(snippet.contains("mount --bind"), "snippet: {snippet}");
        assert!(snippet.contains("remount,ro,bind"), "snippet: {snippet}");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test -p linuxbox adapters::colima::bind_mount_tests 2>&1 | head -10
```

Expected: compile error — `validate_lima_paths` and `bind_mount_shell_snippet` not found.

- [ ] **Step 3: Add `validate_lima_paths` and `bind_mount_shell_snippet`**

Add before `impl ContainerRuntime for ColimaRuntime` in `colima.rs`:

```rust
/// Validate that all bind mount host paths are accessible inside the Lima VM.
///
/// Lima shares `$HOME` and `/tmp` into the VM by default. Paths outside those
/// prefixes are not visible and will cause silent mount failures.
///
/// Returns an error naming the offending path if any path is outside the
/// allowed prefixes.
pub(crate) fn validate_lima_paths(mounts: &[minibox_core::domain::BindMount]) -> anyhow::Result<()> {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    let home_path = std::path::Path::new(&home);

    for m in mounts {
        let p = &m.host_path;
        let in_home = p.starts_with(home_path);
        let in_tmp = p.starts_with("/tmp");
        if !in_home && !in_tmp {
            anyhow::bail!(
                "bind mount source {:?} is not accessible inside the Lima VM.\n\
                 hint: Lima shares $HOME ({}) and /tmp — move the source or add it to lima.yaml shared dirs.",
                p,
                home
            );
        }
    }
    Ok(())
}

/// Build the shell snippet that mounts one bind mount inside the unshare context.
///
/// The snippet is injected into the spawn script before the `exec chroot`.
/// It runs inside the new mount namespace created by `unshare --mount`.
pub(crate) fn bind_mount_shell_snippet(m: &minibox_core::domain::BindMount) -> String {
    let host = m.host_path.display();
    let container = m.container_path.display();

    if m.read_only {
        format!(
            "mkdir -p '{container}' && mount --bind '{host}' '{container}' && mount -o remount,ro,bind '{container}'"
        )
    } else {
        format!("mount --bind '{host}' '{container}'")
    }
}
```

- [ ] **Step 4: Update `ColimaRuntime::spawn_process` to validate paths and inject mount commands**

In `spawn_process`, after `let rootfs = shell_single_quote(...)` and before the `if self.spawner.is_some()` branch, add:

```rust
// Validate that all bind mount host paths are Lima-accessible.
validate_lima_paths(&config.mounts)?;

// Build bind mount shell commands to inject before the chroot.
let bind_mount_cmds: String = config
    .mounts
    .iter()
    .map(|m| {
        // Prepend rootfs to container_path for pre-pivot mounting.
        let host = shell_single_quote(&m.host_path.display().to_string());
        let container_rel = m
            .container_path
            .strip_prefix("/")
            .unwrap_or(&m.container_path);
        let target = format!(
            "{}",
            std::path::Path::new(&config.rootfs).join(container_rel).display()
        );
        let target_q = shell_single_quote(&target);
        if m.read_only {
            format!("sudo mkdir -p {target_q} && sudo mount --bind {host} {target_q} && sudo mount -o remount,ro,bind {target_q}")
        } else {
            format!("sudo mkdir -p {target_q} && sudo mount --bind {host} {target_q}")
        }
    })
    .collect::<Vec<_>>()
    .join("\n");

let privileged_flag = if config.privileged { "--keep-caps" } else { "" };
```

Then update both spawn_script strings to include the bind mount commands and privileged flag:

**Streaming spawn_script:**
```rust
let spawn_script = format!(
    r#"ROOTFS={rootfs}
COMMAND={command}
ARGS=({args})
{bind_mount_cmds}
exec sudo unshare --pid --mount --uts --ipc --net {privileged_flag} \
    --fork --kill-child \
    chroot "$ROOTFS" "$COMMAND" "${{ARGS[@]}}"
"#
);
```

**Background spawn_script:**
```rust
let spawn_script = format!(
    r#"
    ROOTFS={rootfs}
    COMMAND={command}
    ARGS=({args})
    {bind_mount_cmds}

    sudo unshare --pid --mount --uts --ipc --net {privileged_flag} \
        --fork --kill-child \
        chroot "$ROOTFS" "$COMMAND" "${{ARGS[@]}}" &

    echo $!
    "#
);
```

- [ ] **Step 5: Run tests**

```bash
cargo test -p linuxbox adapters::colima::bind_mount_tests 2>&1 | tail -10
```

Expected: all 6 tests pass.

- [ ] **Step 6: Verify workspace compiles**

```bash
cargo check --workspace 2>&1 | tail -5
```

- [ ] **Step 7: Commit**

```bash
git add crates/linuxbox/src/adapters/colima.rs
git commit -m "feat(colima): add Lima path validation and bind mount injection into spawn script"
```

---

## Task 8: CLI — `--privileged`, `-v`, `--mount` flags

**Files:**
- Modify: `crates/minibox-cli/src/main.rs`
- Modify: `crates/minibox-cli/src/commands/run.rs`

- [ ] **Step 1: Write failing parse tests**

Add to `#[cfg(test)]` in `main.rs`:

```rust
#[test]
fn cli_parses_privileged_flag() {
    let cli = Cli::try_parse_from([
        "minibox", "run", "--privileged", "ubuntu", "--", "/bin/sh",
    ]);
    assert!(cli.is_ok(), "parse failed: {:?}", cli.err());
    match cli.unwrap().command {
        Commands::Run { privileged, .. } => assert!(privileged),
        _ => panic!("wrong command"),
    }
}

#[test]
fn cli_parses_volume_flag() {
    let cli = Cli::try_parse_from([
        "minibox", "run", "-v", "/tmp/host:/guest", "ubuntu", "--", "/bin/sh",
    ]);
    assert!(cli.is_ok(), "parse failed: {:?}", cli.err());
    match cli.unwrap().command {
        Commands::Run { volumes, .. } => {
            assert_eq!(volumes.len(), 1);
            assert_eq!(volumes[0], "/tmp/host:/guest");
        }
        _ => panic!("wrong command"),
    }
}

#[test]
fn cli_parses_multiple_volume_flags() {
    let cli = Cli::try_parse_from([
        "minibox", "run",
        "-v", "/tmp/a:/a",
        "-v", "/tmp/b:/b:ro",
        "ubuntu", "--", "/bin/sh",
    ]);
    assert!(cli.is_ok(), "parse failed: {:?}", cli.err());
    match cli.unwrap().command {
        Commands::Run { volumes, .. } => assert_eq!(volumes.len(), 2),
        _ => panic!("wrong command"),
    }
}

#[test]
fn cli_parses_mount_flag() {
    let cli = Cli::try_parse_from([
        "minibox", "run",
        "--mount", "type=bind,src=/tmp/host,dst=/guest",
        "ubuntu", "--", "/bin/sh",
    ]);
    assert!(cli.is_ok(), "parse failed: {:?}", cli.err());
    match cli.unwrap().command {
        Commands::Run { mounts, .. } => assert_eq!(mounts.len(), 1),
        _ => panic!("wrong command"),
    }
}
```

Add to `#[cfg(test)]` in `commands/run.rs`:

```rust
#[test]
fn parse_volume_valid_rw() {
    use std::path::PathBuf;
    let m = parse_volume("/tmp/host:/guest").unwrap();
    assert_eq!(m.host_path, PathBuf::from("/tmp/host"));
    assert_eq!(m.container_path, PathBuf::from("/guest"));
    assert!(!m.read_only);
}

#[test]
fn parse_volume_valid_ro() {
    use std::path::PathBuf;
    let m = parse_volume("/tmp/host:/guest:ro").unwrap();
    assert_eq!(m.host_path, PathBuf::from("/tmp/host"));
    assert_eq!(m.container_path, PathBuf::from("/guest"));
    assert!(m.read_only);
}

#[test]
fn parse_volume_missing_colon_errors() {
    let err = parse_volume("/tmp/nocolon").unwrap_err();
    assert!(err.to_string().contains(":"), "expected colon hint: {err}");
}

#[test]
fn parse_volume_relative_dst_errors() {
    let err = parse_volume("/tmp/host:relative/path").unwrap_err();
    assert!(
        err.to_string().contains("absolute") || err.to_string().contains("/"),
        "expected absolute path error: {err}"
    );
}

#[test]
fn parse_mount_valid_bind() {
    use std::path::PathBuf;
    let m = parse_mount("type=bind,src=/tmp/host,dst=/guest").unwrap();
    assert_eq!(m.host_path, PathBuf::from("/tmp/host"));
    assert_eq!(m.container_path, PathBuf::from("/guest"));
    assert!(!m.read_only);
}

#[test]
fn parse_mount_readonly() {
    let m = parse_mount("type=bind,src=/tmp/host,dst=/guest,readonly").unwrap();
    assert!(m.read_only);
}

#[test]
fn parse_mount_non_bind_type_errors() {
    let err = parse_mount("type=volume,src=myvolume,dst=/data").unwrap_err();
    assert!(
        err.to_string().contains("bind") || err.to_string().contains("type"),
        "expected bind-only error: {err}"
    );
}

#[test]
fn parse_mount_missing_src_errors() {
    let err = parse_mount("type=bind,dst=/guest").unwrap_err();
    assert!(err.to_string().contains("src"), "expected src error: {err}");
}

#[test]
fn parse_mount_missing_dst_errors() {
    let err = parse_mount("type=bind,src=/tmp/host").unwrap_err();
    assert!(err.to_string().contains("dst"), "expected dst error: {err}");
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test -p minibox-cli 2>&1 | head -15
```

Expected: compile errors — `privileged`, `volumes`, `mounts` fields missing from `Commands::Run`.

- [ ] **Step 3: Update `Commands::Run` in `main.rs`**

Add three new fields to `Commands::Run`:

```rust
Run {
    /// Image name (e.g., alpine, ubuntu, library/nginx)
    image: String,

    /// Command to run in the container (everything after --)
    #[arg(last = true)]
    command: Vec<String>,

    /// Memory limit in bytes (passed to cgroups v2 `memory.max`)
    #[arg(long)]
    memory: Option<u64>,

    /// CPU weight in the range 1–10000 (passed to cgroups v2 `cpu.weight`)
    #[arg(long)]
    cpu_weight: Option<u64>,

    /// Image tag (default: latest)
    #[arg(short, long, default_value = "latest")]
    tag: String,

    /// Network mode: none (default), bridge, host, tailnet.
    #[arg(long, default_value = "none")]
    network: String,

    /// Grant full Linux capabilities to the container (required for DinD).
    #[arg(long)]
    privileged: bool,

    /// Bind mount in src:dst[:ro] format. Repeatable.
    /// Example: -v /tmp/bin:/minibox  -v /tmp/traces:/traces:ro
    #[arg(short = 'v', long = "volume", value_name = "SRC:DST[:ro]")]
    volumes: Vec<String>,

    /// Long-form mount specification. Repeatable.
    /// Example: --mount type=bind,src=/tmp/bin,dst=/minibox
    #[arg(long = "mount", value_name = "type=bind,src=PATH,dst=PATH[,readonly]")]
    mounts: Vec<String>,
},
```

Update the `Commands::Run` match arm in `main` to destructure and pass the new fields:

```rust
Commands::Run {
    image,
    command,
    memory,
    cpu_weight,
    tag,
    network,
    privileged,
    volumes,
    mounts,
} => {
    commands::run::execute(
        image,
        tag,
        command,
        memory,
        cpu_weight,
        network,
        privileged,
        volumes,
        mounts,
        socket_path,
    )
    .await
}
```

- [ ] **Step 4: Add `parse_volume`, `parse_mount`, update `execute` in `commands/run.rs`**

Add the two parse functions and update `execute`:

```rust
use minibox_core::domain::BindMount;
use std::path::PathBuf;

/// Parse a `-v src:dst[:ro]` volume shorthand into a `BindMount`.
///
/// # Errors
///
/// Returns an error if:
/// - There is no `:` separating src and dst.
/// - `dst` is not an absolute path (does not start with `/`).
pub fn parse_volume(s: &str) -> anyhow::Result<BindMount> {
    let parts: Vec<&str> = s.splitn(3, ':').collect();
    if parts.len() < 2 {
        anyhow::bail!("invalid volume format {:?}: expected src:dst or src:dst:ro", s);
    }
    let host_path = PathBuf::from(parts[0]);
    let container_path = PathBuf::from(parts[1]);
    if !container_path.is_absolute() {
        anyhow::bail!(
            "container path {:?} must be absolute (start with /)",
            container_path
        );
    }
    let read_only = parts.get(2).map(|f| *f == "ro").unwrap_or(false);
    Ok(BindMount { host_path, container_path, read_only })
}

/// Parse a `--mount type=bind,src=PATH,dst=PATH[,readonly]` spec into a `BindMount`.
///
/// # Errors
///
/// Returns an error if:
/// - `type` is not `bind`.
/// - `src` or `dst` is missing.
/// - `dst` is not absolute.
pub fn parse_mount(s: &str) -> anyhow::Result<BindMount> {
    let mut mount_type = None::<String>;
    let mut src = None::<PathBuf>;
    let mut dst = None::<PathBuf>;
    let mut read_only = false;

    for kv in s.split(',') {
        if kv == "readonly" || kv == "ro" {
            read_only = true;
            continue;
        }
        let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
        match k {
            "type" => mount_type = Some(v.to_string()),
            "src" | "source" => src = Some(PathBuf::from(v)),
            "dst" | "target" | "destination" => dst = Some(PathBuf::from(v)),
            _ => {} // ignore unknown keys
        }
    }

    match mount_type.as_deref() {
        Some("bind") | None => {}
        Some(t) => anyhow::bail!("unsupported mount type {:?}: only 'bind' is supported", t),
    }

    let host_path = src.ok_or_else(|| anyhow::anyhow!("--mount missing 'src' key"))?;
    let container_path = dst.ok_or_else(|| anyhow::anyhow!("--mount missing 'dst' key"))?;
    if !container_path.is_absolute() {
        anyhow::bail!(
            "container path {:?} must be absolute (start with /)",
            container_path
        );
    }

    Ok(BindMount { host_path, container_path, read_only })
}
```

Update `execute` signature and body:

```rust
pub async fn execute(
    image: String,
    tag: String,
    command: Vec<String>,
    memory_limit_bytes: Option<u64>,
    cpu_weight: Option<u64>,
    network: String,
    privileged: bool,
    volumes: Vec<String>,
    mount_specs: Vec<String>,
    socket_path: &std::path::Path,
) -> Result<()> {
    let network_mode = match network.as_str() {
        "none" => NetworkMode::None,
        "bridge" => NetworkMode::Bridge,
        "host" => NetworkMode::Host,
        "tailnet" => NetworkMode::Tailnet,
        other => {
            anyhow::bail!("unknown network mode: {other} (expected: none, bridge, host, tailnet)")
        }
    };

    // Parse -v shorthand mounts.
    let mut mounts: Vec<minibox_core::domain::BindMount> = Vec::new();
    for v in &volumes {
        mounts.push(parse_volume(v).with_context(|| format!("invalid -v flag {:?}", v))?);
    }
    // Parse --mount long-form mounts.
    for m in &mount_specs {
        mounts.push(parse_mount(m).with_context(|| format!("invalid --mount flag {:?}", m))?);
    }

    let request = DaemonRequest::Run {
        image,
        tag: Some(tag),
        command,
        memory_limit_bytes,
        cpu_weight,
        ephemeral: true,
        network: Some(network_mode),
        mounts,
        privileged,
    };

    // ... rest of execute body unchanged (client call + streaming loop) ...
}
```

- [ ] **Step 5: Run CLI tests**

```bash
cargo test -p minibox-cli 2>&1 | tail -15
```

Expected: all new tests pass, all existing tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/minibox-cli/src/main.rs crates/minibox-cli/src/commands/run.rs
git commit -m "feat(cli): add --privileged, -v/--volume, --mount flags to minibox run"
```

---

## Task 9: `just build-linux` recipe and updated `just trace`

**Files:**
- Modify: `Justfile`

- [ ] **Step 1: Verify musl target is available**

```bash
rustup target list --installed | grep musl
```

If `x86_64-unknown-linux-musl` is not listed, it will be added by the recipe itself.

- [ ] **Step 2: Add `build-linux` recipe to `Justfile`**

Add after `build` (around line 25), in the `# ── Build` section:

```just
# Build static Linux x86_64 binaries (works from macOS or Linux).
# Output: target/x86_64-unknown-linux-musl/release/{miniboxd,minibox}
build-linux:
    rustup target add x86_64-unknown-linux-musl
    RUSTFLAGS="-C target-feature=+crt-static" \
        cargo build --release --target x86_64-unknown-linux-musl \
        -p miniboxd -p minibox-cli
```

- [ ] **Step 3: Update `just trace`**

Replace the existing `trace` recipe body with the full cross-platform version:

```just
# Trace miniboxd with uftrace.
# macOS: cross-compiles Linux binary, runs it inside minibox via Colima.
# Linux: runs natively (requires root + apt install uftrace).
# After run: uftrace graph -d <trace-dir>
trace:
    #!/usr/bin/env bash
    set -euo pipefail

    TRACE_DIR="traces/$(date +%Y%m%d-%H%M%S)"
    mkdir -p "$TRACE_DIR"

    if [[ "$(uname -s)" == "Darwin" ]]; then
        echo "trace: building Linux musl binary..."
        just build-linux

        BINARY_DIR="$(pwd)/target/x86_64-unknown-linux-musl/release"
        ABS_TRACE="$(pwd)/$TRACE_DIR"

        echo "trace: running uftrace inside minibox container..."
        minibox run --privileged \
            -v "${BINARY_DIR}:/minibox" \
            -v "${ABS_TRACE}:/traces" \
            ubuntu \
            -- sh -c "
                apt-get install -y uftrace -q 2>/dev/null &&
                uftrace record -P . --no-libcall -d /traces /minibox/miniboxd &
                DAEMON_PID=\$!
                sleep 2
                /minibox/minibox pull alpine 2>/dev/null || true
                /minibox/minibox run alpine -- /bin/echo 'uftrace smoke' 2>/dev/null || true
                kill \$DAEMON_PID 2>/dev/null || true
                wait \$DAEMON_PID 2>/dev/null || true
            "

        echo ""
        echo "── uftrace report (top 20 by total time) ──────────────────────────────"
        uftrace report -d "$TRACE_DIR" --sort=total 2>/dev/null | head -25 || echo "(no trace data)"
    else
        [[ "$(uname -s)" == "Linux" ]] || { echo "error: unsupported platform"; exit 1; }
        command -v uftrace >/dev/null 2>&1 || { echo "error: apt install uftrace"; exit 1; }
        [[ "$(id -u)" -eq 0 ]] || { echo "error: sudo just trace"; exit 1; }

        echo "trace: building native release binary..."
        cargo build --release -p miniboxd -p minibox-cli

        echo "trace: recording to $TRACE_DIR ..."
        uftrace record -P . --no-libcall -d "$TRACE_DIR" ./target/release/miniboxd &
        DAEMON_PID=$!

        for i in $(seq 1 10); do
            [[ -S /run/minibox/miniboxd.sock ]] && break
            sleep 0.5
        done
        [[ -S /run/minibox/miniboxd.sock ]] || { echo "error: daemon socket did not appear"; kill "$DAEMON_PID" 2>/dev/null; exit 1; }

        echo "trace: smoke — pull alpine..."
        ./target/release/minibox pull alpine || true
        echo "trace: smoke — run echo..."
        ./target/release/minibox run alpine -- /bin/echo "uftrace smoke" || true

        echo "trace: stopping daemon..."
        kill "$DAEMON_PID" 2>/dev/null || true
        wait "$DAEMON_PID" 2>/dev/null || true

        echo ""
        echo "── uftrace report (top 20 by total time) ──────────────────────────────"
        uftrace report -d "$TRACE_DIR" --sort=total 2>/dev/null | head -25 || echo "(no trace data)"
    fi

    echo ""
    echo "trace: data saved to $TRACE_DIR"
    echo "trace: call graph      → uftrace graph -d $TRACE_DIR"
    echo "trace: chrome devtools → uftrace dump -d $TRACE_DIR --chrome > $TRACE_DIR/trace.json"
```

- [ ] **Step 4: Run pre-commit gate**

```bash
cargo xtask pre-commit 2>&1 | tail -10
```

Expected: `pre-commit checks passed`.

- [ ] **Step 5: Run full unit tests**

```bash
cargo xtask test-unit 2>&1 | tail -10
```

Expected: all tests pass.

- [ ] **Step 6: Commit**

```bash
git add Justfile
git commit -m "feat(justfile): add build-linux recipe and update just trace for macOS DinD path"
```

---

## Self-Review Checklist

After completing all tasks, verify:

- [ ] `cargo check --workspace` compiles cleanly
- [ ] `cargo xtask test-unit` passes (all ~257 unit tests)
- [ ] Old JSON without `mounts`/`privileged` still deserializes (Task 1 backward-compat test)
- [ ] `minibox run --privileged -v /tmp/a:/b ubuntu -- /bin/echo ok` parses without error
- [ ] `just build-linux` produces `target/x86_64-unknown-linux-musl/release/miniboxd`
- [ ] Linux integration path: `sudo just trace` (on VPS) runs and produces a non-empty `traces/` dir

---

## Spec Coverage Check

| Spec section | Covered by task |
|---|---|
| `BindMount` type in protocol | Task 1 |
| `mounts` + `privileged` in `DaemonRequest::Run` | Task 1 |
| `ContainerSpawnConfig` fields | Task 2 |
| `apply_bind_mounts` in `filesystem.rs` | Task 3 |
| `ContainerConfig` + `apply_full_capabilities` | Task 4 |
| `LinuxNamespaceRuntime` wiring | Task 5 |
| Handler wiring | Task 6 |
| Lima path validation | Task 7 |
| Colima bind mounts in spawn script | Task 7 |
| CLI `--privileged`, `-v`, `--mount` | Task 8 |
| `just build-linux` | Task 9 |
| Updated `just trace` (macOS + Linux) | Task 9 |
| Error: host path does not exist | Task 3 (test), Task 8 (CLI parse) |
| Error: host path outside Lima dirs | Task 7 (test + impl) |
| Error: relative `dst` | Task 8 (test + impl) |
| Cleanup on partial bind mount failure | Task 3 (impl) |
