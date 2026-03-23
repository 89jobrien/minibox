# Property-Based Tests: Daemonbox and Container Boundary

**Date:** 2026-03-21
**Status:** Approved

## Goal

Extend the existing proptest suite beyond protocol encode/decode roundtrips and path validation to cover three new boundary areas: daemonbox state consistency, handler input safety, and cgroup config bounds.

## Scope

9 new property tests across 2 files. All tests that touch Linux kernel interfaces are gated `#[cfg(target_os = "linux")]`; all others must pass on macOS via `cargo xtask test-unit`.

## File 1: `crates/daemonbox/tests/proptest_suite.rs` (new)

### Dependencies

Add to `crates/daemonbox/Cargo.toml`:

```toml
[dev-dependencies]
proptest = "1"
tempfile = "3"
```

`tokio` is not listed separately — `daemonbox` already depends on `tokio = { workspace = true }` (workspace has `features = ["full"]`), so `rt` and `macros` are available in the dev context without a redundant dev-dep.

### Fixture Construction

`DaemonState::new(image_store, data_dir)` requires:

1. An `ImageStore` instance — constructed via `ImageStore::new(tmp.path())` which calls `create_dir_all` immediately.
2. A `data_dir` path — `DaemonState` writes `state.json` there on every `add_container` / `remove_container` call via `save_to_disk`.

Each proptest closure must create its own `tempfile::TempDir` (assigned to a named variable so it stays alive for the closure duration) and derive both paths from it. The disk I/O from `save_to_disk` is expected — proptest will run 256 iterations per property, each performing a small number of JSON writes to a temp directory.

### Strategies

- `arb_container_id()` — `[a-z][a-z0-9]{7,31}` (valid UUID-like identifiers)
- `arb_container_record(id: String)` — builds a `ContainerRecord` with:
  - `info: ContainerInfo { id, image, command: String::new(), state: "created".into(), created_at: "2026-01-01T00:00:00Z".into(), pid: None }`
  - `pid: None`
  - `rootfs_path: PathBuf::from("/tmp/fake-rootfs")`
  - `cgroup_path: PathBuf::from("/tmp/fake-cgroup")`
  - `post_exit_hooks: vec![]`

### DaemonState Invariants (4 tests)

All use `tokio::runtime::Runtime::new().unwrap().block_on(...)` to drive async methods synchronously within proptest closures.

1. **`state_add_then_get_finds_record`** — `add_container(r)` then `get_container(&id)` always returns a record with matching id.

2. **`state_remove_after_add_returns_none`** — `add_container(r)` then `remove_container(&id)` then `get_container(&id)` always returns `None`.

3. **`state_list_count_matches_adds`** — after adding N records with distinct IDs (up to 8), `list_containers().len() == N`.

4. **`state_arbitrary_sequence_no_panic`** — an arbitrary sequence of adds (up to 8 distinct IDs) followed by removes (arbitrary subset) never panics, and `list_containers().len()` equals adds minus successful removes.

### Handler Input Safety (3 tests)

Wire `HandlerDependencies` using `adapters::mocks` (already present in minibox-lib). No Linux syscalls are required. Both `handle_stop` and `handle_remove` check `state.get_container(id)` first — for an unknown ID they return `ContainerNotFound` before touching any filesystem or mock dependencies.

5. **`handle_stop_unknown_id_is_error`** — `handle_stop(arbitrary_id, empty_state)` always returns `DaemonResponse::Error`, never panics.

6. **`handle_remove_unknown_id_is_error`** — `handle_remove(arbitrary_id, empty_state, mock_deps)` always returns `DaemonResponse::Error`, never panics.

7. **`handle_list_always_returns_container_list`** — `handle_list(state)` with arbitrary pre-populated state always returns `DaemonResponse::ContainerList`, never any other variant.

## File 2: `crates/minibox-lib/tests/proptest_suite.rs` (extend)

### CgroupConfig Boundary Validation (2 tests, Linux + root only)

Gated with `#[cfg(target_os = "linux")]`. Skipped by `cargo xtask test-unit` on macOS; run under `just test-integration` on the VPS where a real cgroup2 mount and root privileges are available.

**Implementation note:** `CgroupManager::create()` calls `create_dir_all` and `enable_subtree_controllers` (which writes to `cgroup.subtree_control`) **before** the memory and cpu_weight range checks. Tests therefore require a real cgroup2 mount to reach the validation logic — a plain `TempDir` is insufficient. The cgroup root is controlled by the `MINIBOX_CGROUP_ROOT` env var; under `just test-integration` this is set to a valid cgroup2 path with root permissions.

8. **`memory_below_4096_always_rejected`** — `CgroupManager` with `memory_limit_bytes < 4096` returns `Err` from `create()`.

9. **`cpu_weight_out_of_range_rejected`** — `CgroupManager` with `cpu_weight` outside `1..=10_000` returns `Err` from `create()`.

## Test Placement Rationale

- Public API invariants for daemonbox types → `crates/daemonbox/tests/` (integration test style, exercises public API only)
- Extensions to existing minibox-lib property suite → `crates/minibox-lib/tests/proptest_suite.rs`
- No inline `#[cfg(test)]` blocks — handler dispatch requires `HandlerDependencies` wiring that is cleaner in integration test context

## Platform Matrix

| Test group             | macOS   | Linux (no root) | Linux (root, cgroup2) |
| ---------------------- | ------- | --------------- | --------------------- |
| DaemonState invariants | ✓       | ✓               | ✓                     |
| Handler input safety   | ✓       | ✓               | ✓                     |
| CgroupConfig bounds    | skipped | skipped         | ✓                     |

## Success Criteria

- `cargo xtask test-unit` passes on macOS with all 7 non-Linux-gated tests running
- `just test-integration` on the VPS picks up the 2 cgroup bound tests
- No new `unsafe` code; mock adapters used throughout
- Proptest default config (256 cases per property) unless a test needs fewer for speed
