# Property-Based Tests: Daemonbox and Container Boundary

**Date:** 2026-03-21
**Status:** Approved

## Goal

Extend the existing proptest suite beyond protocol encode/decode roundtrips and path validation to cover three new boundary areas: daemonbox state consistency, handler input safety, and container config validation.

## Scope

9 new property tests across 2 files. All tests that touch Linux kernel interfaces are gated `#[cfg(target_os = "linux")]`; all others must pass on macOS via `cargo xtask test-unit`.

## File 1: `crates/daemonbox/tests/proptest_suite.rs` (new)

### Dependencies

Add to `crates/daemonbox/Cargo.toml`:

```toml
[dev-dependencies]
proptest = "1"
tokio = { version = "1", features = ["rt", "macros"] }
```

### Strategies

- `arb_container_id()` — `[a-z][a-z0-9]{7,31}` (valid UUID-like identifiers)
- `arb_container_record()` — arbitrary `ContainerRecord` with generated id, image name, empty command vec, `Created` state, temp paths for rootfs and cgroup

### DaemonState Invariants (4 tests)

All use `tokio::runtime::Runtime::new().unwrap().block_on(...)` to drive async methods without requiring `#[tokio::test]` inside proptest closures.

1. **`state_add_then_get_finds_record`** — `add_container(r)` then `get_container(&id)` always returns a record with the same id.

2. **`state_remove_after_add_returns_none`** — `add_container(r)` then `remove_container(&id)` then `get_container(&id)` always returns `None`.

3. **`state_list_count_matches_adds`** — after adding N records with distinct IDs, `list_containers().len() == N`.

4. **`state_arbitrary_sequence_no_panic`** — an arbitrary sequence of adds (up to 8 distinct IDs) followed by removes (arbitrary subset) followed by a list never panics, and `list_containers().len()` equals the number of IDs added minus the number successfully removed.

### Handler Input Safety (3 tests)

Wire `HandlerDependencies` using `adapters::mocks` (already present in minibox-lib). No Linux syscalls are required.

5. **`handle_stop_unknown_id_is_error`** — `handle_stop(arbitrary_id, empty_state)` always returns `DaemonResponse::Error`, never panics.

6. **`handle_remove_unknown_id_is_error`** — `handle_remove(arbitrary_id, empty_state, mock_deps)` always returns `DaemonResponse::Error`, never panics.

7. **`handle_list_always_returns_container_list`** — `handle_list(state)` with arbitrary pre-populated state always returns `DaemonResponse::ContainerList`, never any other variant.

## File 2: `crates/minibox-lib/tests/proptest_suite.rs` (extend)

### ImageRef Parsing Invariants (2 tests)

8. **`image_ref_arbitrary_string_never_panics`** — `ImageRef::parse(s)` for any arbitrary `String` never panics; it only returns `Ok` or `Err`.

9. **`image_ref_valid_refs_always_parse`** — refs constructed from valid components (`[a-z][a-z0-9_.-]{0,63}` name, optional `[a-zA-Z0-9._-]{1,128}` tag) always parse successfully.

### CgroupConfig Boundary Validation (2 tests, Linux-only)

Gated with `#[cfg(target_os = "linux")]`. These are skipped by `cargo xtask test-unit` on macOS and run under `just test-integration` on the VPS.

Uses a temp directory for the cgroup path since `CgroupManager::create()` would fail without real cgroup2 mount; tests are expected to return `Err` on memory/cpu range violations before touching the filesystem.

10. **`memory_below_4096_always_rejected`** — `CgroupManager` with `memory_limit_bytes < 4096` returns `Err` from `create()`.

11. **`cpu_weight_out_of_range_rejected`** — `CgroupManager` with `cpu_weight` outside `1..=10_000` returns `Err` from `create()`.

## Test Placement Rationale

- Public API invariants for daemonbox types → `crates/daemonbox/tests/` (integration test style, exercises public API only)
- Extensions to existing minibox-lib property suite → `crates/minibox-lib/tests/proptest_suite.rs`
- No inline `#[cfg(test)]` blocks — handler dispatch requires `HandlerDependencies` wiring that is cleaner in integration test context

## Platform Matrix

| Test group | macOS | Linux (no root) | Linux (root) |
|---|---|---|---|
| DaemonState invariants | ✓ | ✓ | ✓ |
| Handler input safety | ✓ | ✓ | ✓ |
| ImageRef parsing | ✓ | ✓ | ✓ |
| CgroupConfig bounds | skipped | skipped | ✓ |

## Success Criteria

- `cargo xtask test-unit` passes on macOS with all non-Linux-gated tests running
- `just test-integration` on the VPS picks up the cgroup bound tests
- No new `unsafe` code; mock adapters used throughout
- Proptest default config (256 cases per property) unless a test needs fewer for speed
