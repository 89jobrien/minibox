# KrunRuntime Adapter Conformance Specification

**Date:** 2026-04-23
**Status:** Complete — Phases 1-3 done (K-S, K-R, K-F, K-L, K-I, K-H all passing)
**Adapter:** `MINIBOX_ADAPTER=krun`
**Platforms:** macOS ARM64 (HVF), Linux x86_64/ARM64 (KVM)

---

## Purpose

This document is the single source of truth for what the krun adapter suite must satisfy
before it can be considered complete. Every requirement maps 1:1 to a test. No requirement
is implemented without a failing test first (TDD).

The krun suite consists of four adapters implementing the domain ports defined in
`crates/minibox-core/src/domain.rs`:

| Adapter            | Port                                          | Crate         |
| ------------------ | --------------------------------------------- | ------------- |
| `KrunRuntime`      | `ContainerRuntime`                            | `macbox`/`krunbox` |
| `KrunFilesystem`   | `FilesystemProvider` + `RootfsSetup` + `ChildInit` | same     |
| `KrunLimiter`      | `ResourceLimiter`                             | same          |
| `KrunRegistry`     | `ImageRegistry`                               | same          |

The adapter is **not macOS-only**. The same Rust code (with platform-conditional
hypervisor backend) runs on macOS via HVF and Linux via KVM.

---

## Platform Gate

Tests are gated on hypervisor availability, not OS:

```rust
fn krun_available() -> bool {
    // macOS: Hypervisor.framework present (always true on Apple Silicon)
    // Linux: /dev/kvm readable
    #[cfg(target_os = "macos")]
    return true;

    #[cfg(target_os = "linux")]
    return std::path::Path::new("/dev/kvm").exists()
        && std::fs::metadata("/dev/kvm")
            .map(|m| !m.permissions().readonly())
            .unwrap_or(false);
}

macro_rules! skip_if_no_krun {
    () => {
        if std::env::var("MINIBOX_KRUN_TESTS").as_deref() != Ok("1") {
            eprintln!("SKIP: set MINIBOX_KRUN_TESTS=1 to run krun conformance tests");
            return;
        }
        if !krun_available() {
            eprintln!("SKIP: no hypervisor available (macOS HVF or Linux /dev/kvm)");
            return;
        }
    };
}
```

Run gate: `MINIBOX_KRUN_TESTS=1 cargo test -p macbox --test krun_adapter_conformance -- --test-threads=1`

`--test-threads=1` is required: parallel krun invocations share per-process state in
libkrun and collide on socket paths.

---

## Phase 1 — `SmolvmProcess` shim (existing, passing)

These 6 tests already pass and establish the baseline for the subprocess shim path.
They live in `crates/macbox/tests/krun_conformance_tests.rs`.

| ID     | Test name                                  | Requirement                                        |
| ------ | ------------------------------------------ | -------------------------------------------------- |
| K-S-01 | `krun_adapter_missing_binary_returns_error`| Missing smolvm binary → `Err`, never panics        |
| K-S-02 | `krun_process_exits_zero_for_true_command` | `/bin/true` in VM exits 0                          |
| K-S-03 | `krun_stdout_is_captured`                  | stdout readable via `proc.collect_stdout()`        |
| K-S-04 | `krun_nonzero_exit_code_propagated`        | `exit 42` in VM → exit code 42                     |
| K-S-05 | `krun_env_var_passed_to_process`           | Env var passed to spawn visible inside VM          |
| K-S-06 | `krun_hostname_differs_from_host`          | VM hostname differs from host hostname             |

---

## Phase 2 — Domain port conformance (drives TDD)

New file: `crates/macbox/tests/krun_adapter_conformance.rs`
(or `crates/krunbox/tests/krun_adapter_conformance.rs` once crate is extracted).

### 2a. `KrunRuntime` (`ContainerRuntime` port)

| ID     | Test name                                          | Requirement                                                          |
| ------ | -------------------------------------------------- | -------------------------------------------------------------------- |
| K-R-01 | `krun_runtime_create_returns_container_id`         | `create()` returns a non-empty `ContainerHandle` with a valid ID     |
| K-R-02 | `krun_runtime_create_start_produces_output`        | `create()` + `start()` → stdout lines received before stop          |
| K-R-03 | `krun_runtime_stop_terminates_process`             | `stop()` on running container → process exits within 5s             |
| K-R-04 | `krun_runtime_wait_returns_exit_code`              | `wait()` after `/bin/true` → exit code 0                            |
| K-R-05 | `krun_runtime_wait_propagates_nonzero_exit`        | `wait()` after `exit 42` → exit code 42                             |
| K-R-06 | `krun_runtime_destroy_cleans_up`                   | `destroy()` after stop → no orphaned processes or temp files        |
| K-R-07 | `krun_runtime_concurrent_containers_independent`   | Two containers run concurrently with independent stdout streams      |
| K-R-08 | `krun_runtime_missing_image_returns_err`           | `create()` with non-existent image → `Err`, not panic               |
| K-R-09 | `krun_runtime_env_vars_visible_in_container`       | `ContainerSpawnConfig.env` entries visible inside VM                 |
| K-R-10 | `krun_runtime_command_args_forwarded`              | Command + args in `ContainerSpawnConfig` run as specified            |

### 2b. `KrunFilesystem` (`RootfsSetup` + `ChildInit`)

| ID     | Test name                                          | Requirement                                                          |
| ------ | -------------------------------------------------- | -------------------------------------------------------------------- |
| K-F-01 | `krun_filesystem_setup_rootfs_returns_ok`          | `setup_rootfs()` returns `Ok` for a valid image path                 |
| K-F-02 | `krun_filesystem_setup_rootfs_missing_path_err`    | `setup_rootfs()` with nonexistent path → `Err`                       |
| K-F-03 | `krun_filesystem_child_init_is_noop_ok`            | `child_init()` returns `Ok` without side effects (VM handles init)   |

Note: krun manages its own rootfs mounting internally via virtio-fs. `KrunFilesystem`
is intentionally thin — `setup_rootfs` validates that the image path exists and is
readable; `child_init` is a no-op returning `Ok`.

### 2c. `KrunLimiter` (`ResourceLimiter` port)

| ID     | Test name                                          | Requirement                                                          |
| ------ | -------------------------------------------------- | -------------------------------------------------------------------- |
| K-L-01 | `krun_limiter_apply_memory_limit_ok`               | `apply(memory_bytes=256MB)` returns `Ok` without error               |
| K-L-02 | `krun_limiter_apply_cpu_weight_ok`                 | `apply(cpu_weight=512)` returns `Ok`                                 |
| K-L-03 | `krun_limiter_apply_zero_memory_is_noop`           | `apply(memory_bytes=0)` → `Ok`, no panic (treated as unlimited)     |
| K-L-04 | `krun_limiter_cleanup_after_apply_ok`              | `cleanup()` after `apply()` → `Ok`                                   |
| K-L-05 | `krun_limiter_cleanup_without_apply_is_safe`       | `cleanup()` without prior `apply()` → `Ok`, no panic                |

Note: resource limits are passed to libkrun at VM creation time, not via cgroups.
`KrunLimiter` translates `ResourceConfig` into libkrun VM config fields.

### 2d. `KrunRegistry` (`ImageRegistry` port)

| ID     | Test name                                          | Requirement                                                          |
| ------ | -------------------------------------------------- | -------------------------------------------------------------------- |
| K-I-01 | `krun_registry_pull_alpine_succeeds`               | `pull("alpine", "latest")` downloads layers and returns manifest     |
| K-I-02 | `krun_registry_pull_cached_image_is_fast`          | Second `pull()` for same image completes without network fetch       |
| K-I-03 | `krun_registry_pull_nonexistent_image_errors`      | `pull("minibox-nonexistent-xyz", "latest")` → `Err`                  |
| K-I-04 | `krun_registry_image_manifest_has_layers`          | Pulled manifest has at least one layer                               |
| K-I-05 | `krun_registry_pull_respects_size_limit`           | Manifest > 10MB → `Err(ImageError::ManifestTooLarge)` (existing cap) |

Note: `KrunRegistry` reuses `DockerHubRegistry` from `minibox-core` — this suite
validates the adapter wiring, not the registry client itself.

---

## Phase 3 — Integration with `HandlerDependencies`

These tests wire the krun adapter suite into `HandlerDependencies` and validate that
`handle_run` produces correct protocol responses end-to-end.

File: `crates/daemonbox/tests/conformance_tests.rs` (new `krun_suite` module)

| ID     | Test name                                          | Requirement                                                          |
| ------ | -------------------------------------------------- | -------------------------------------------------------------------- |
| K-H-01 | `krun_handle_run_returns_container_created`        | `handle_run(ephemeral=false)` → `DaemonResponse::ContainerCreated`   |
| K-H-02 | `krun_handle_run_ephemeral_streams_output`         | `handle_run(ephemeral=true)` → ≥1 `ContainerOutput` + `ContainerStopped` |
| K-H-03 | `krun_handle_run_error_path_returns_error_response`| `handle_run` with invalid image → `DaemonResponse::Error`            |
| K-H-04 | `krun_handle_ps_lists_running_container`           | After `handle_run`, `handle_ps` includes the container               |
| K-H-05 | `krun_handle_stop_terminates_container`            | `handle_stop` after `handle_run` → container state transitions to Stopped |

---

## Capability Matrix

| Capability                    | `krun` | `native` | `colima` | Notes                           |
| ----------------------------- | ------ | -------- | -------- | ------------------------------- |
| Run container (alpine)        | ✓      | ✓        | ✓        |                                 |
| Capture stdout                | ✓      | ✓        | ✓        |                                 |
| Env var injection             | ✓      | ✓        | ✓        |                                 |
| Memory limit                  | ✓      | ✓        | ✗        | krun: VM config; native: cgroup |
| CPU weight                    | ✓      | ✓        | ✗        | krun: VM vcpu; native: cgroup   |
| Namespace isolation           | via VM | ✓        | via OCI  |                                 |
| Rootfs overlay                | via VM | ✓        | ✗        | krun: virtiofs                  |
| Exec into running container   | ✗      | ✓        | ✗        | Phase 4 — vsock exec            |
| PTY / interactive             | ✗      | ✗        | ✗        | Planned, all adapters           |
| Bind mounts                   | ✗      | ✓        | ✗        | Phase 4 — virtiofs bind mounts  |
| macOS (ARM64)                 | ✓      | ✗        | ✓        | krun: HVF; colima: lima         |
| Linux x86_64                  | ✓      | ✓        | ✗        | krun: KVM                       |
| Linux ARM64                   | ✓      | ✓        | ✗        | krun: KVM                       |

---

## TDD Execution Order

Implement in this order — each phase unlocks the next:

```
Phase 1 (existing) ──► already green
Phase 2a KrunRuntime ──► Phase 2b KrunFilesystem ──► Phase 2c KrunLimiter ──► Phase 2d KrunRegistry
Phase 3 HandlerDependencies integration
```

Within Phase 2a, implement K-R-01 first (create returns ID), then K-R-02 (output), then
K-R-03/04/05 (lifecycle), then K-R-06/07/08/09/10 (cleanup, concurrency, errors).

Each test must be **red before the implementation step that makes it green**. Do not
write implementation code to pass a test that is not yet written.

---

## Acceptance Criteria

The krun adapter is complete when:

1. All K-S, K-R, K-F, K-L, K-I, K-H tests pass with `MINIBOX_KRUN_TESTS=1`
2. `cargo xtask test-krun-conformance` exits 0 on macOS ARM64
3. `cargo xtask test-krun-conformance` exits 0 on a Linux x86_64 host with `/dev/kvm`
4. `MINIBOX_ADAPTER=krun cargo run --bin minibox-bench -- --suite adapter` completes
   without error
5. The capability matrix above is reflected in `docs/FEATURE_MATRIX.md`

---

## Files to Create / Modify

| File                                                    | Action   | Notes                                     |
| ------------------------------------------------------- | -------- | ----------------------------------------- |
| `crates/macbox/tests/krun_adapter_conformance.rs`       | Create   | Phase 2 + 3 tests                         |
| `crates/macbox/src/krun/runtime.rs`                     | Create   | `KrunRuntime` impl                        |
| `crates/macbox/src/krun/filesystem.rs`                  | Create   | `KrunFilesystem` impl                     |
| `crates/macbox/src/krun/limiter.rs`                     | Create   | `KrunLimiter` impl                        |
| `crates/macbox/src/krun/registry.rs`                    | Create   | `KrunRegistry` (thin wrapper)             |
| `crates/macbox/src/krun/mod.rs`                         | Modify   | Re-export new modules                     |
| `crates/macbox/src/lib.rs`                              | Modify   | Wire `start_krun()`, platform-conditional |
| `crates/miniboxd/src/main.rs`                           | Modify   | Add `MINIBOX_ADAPTER=krun` dispatch branch|
| `crates/xtask/src/main.rs`                              | Modify   | Add `test-krun-conformance` subcommand    |
| `docs/FEATURE_MATRIX.md`                                | Modify   | Add krun row to capability table          |
