---
status: done
---

# Proptest Daemonbox Boundary Tests Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add 9 property-based tests covering daemonbox state invariants, handler input safety, and cgroup config bounds.

**Architecture:** Two new/extended test files: `crates/daemonbox/tests/proptest_suite.rs` (new, 7 tests) and `crates/minibox/tests/proptest_suite.rs` (extended, 2 Linux-gated tests). DaemonState tests use `tokio::runtime::Runtime::block_on` to drive async calls synchronously inside proptest closures. Handler safety tests wire `HandlerDependencies` with `adapters::mocks`. Cgroup tests require a real cgroup2 mount and run only under `just test-integration` on the VPS.

**Tech Stack:** Rust, proptest 1.x, tokio (workspace, `features = ["full"]`), tempfile (workspace), minibox mock adapters.

---

## File Map

| File                                       | Action | Responsibility                                  |
| ------------------------------------------ | ------ | ----------------------------------------------- |
| `crates/daemonbox/Cargo.toml`              | Modify | Add `proptest = "1"` dev-dep                    |
| `crates/daemonbox/tests/proptest_suite.rs` | Create | DaemonState invariants (4) + handler safety (3) |
| `crates/minibox/tests/proptest_suite.rs`   | Modify | Add cgroup bounds tests (2, Linux-gated)        |

---

## Task 1: Add proptest dev-dep and DaemonState invariant tests

**Files:**

- Modify: `crates/daemonbox/Cargo.toml`
- Create: `crates/daemonbox/tests/proptest_suite.rs`

- [ ] **Step 1: Add proptest to daemonbox dev-dependencies**

In `crates/daemonbox/Cargo.toml`, find the `[dev-dependencies]` section (or add it after `[dependencies]`) and add:

```toml
[dev-dependencies]
proptest = "1"
```

`tempfile` is already a workspace dev-dep in daemonbox — no change needed.

- [ ] **Step 2: Create the test file with helpers and the first invariant test**

Create `crates/daemonbox/tests/proptest_suite.rs`:

```rust
//! Property-based tests for daemonbox state invariants and handler input safety.

use std::path::Path;
use std::sync::Arc;

use daemonbox::{
    handler::{handle_list, handle_remove, handle_stop, HandlerDependencies},
    state::{ContainerRecord, DaemonState},
};
use minibox::{
    adapters::mocks::{MockFilesystem, MockLimiter, MockRegistry, MockRuntime},
    image::ImageStore,
    protocol::{ContainerInfo, DaemonResponse},
};
use proptest::prelude::*;

// ── Helpers ──────────────────────────────────────────────────────────────────

fn make_rt() -> tokio::runtime::Runtime {
    tokio::runtime::Runtime::new().expect("tokio runtime")
}

fn make_state(tmp: &Path) -> Arc<DaemonState> {
    let image_store = ImageStore::new(tmp.join("images")).expect("ImageStore::new");
    Arc::new(DaemonState::new(image_store, tmp))
}

fn make_deps(tmp: &Path) -> Arc<HandlerDependencies> {
    Arc::new(HandlerDependencies {
        registry: Arc::new(MockRegistry::new()),
        filesystem: Arc::new(MockFilesystem::new()),
        resource_limiter: Arc::new(MockLimiter::new()),
        runtime: Arc::new(MockRuntime::new()),
        containers_base: tmp.join("containers"),
        run_containers_base: tmp.join("run"),
    })
}

fn make_record(id: &str) -> ContainerRecord {
    ContainerRecord {
        info: ContainerInfo {
            id: id.to_string(),
            image: "test-image".into(),
            command: String::new(),
            state: "created".into(),
            created_at: "2026-01-01T00:00:00Z".into(),
            pid: None,
        },
        pid: None,
        rootfs_path: std::path::PathBuf::from("/tmp/fake-rootfs"),
        cgroup_path: std::path::PathBuf::from("/tmp/fake-cgroup"),
        post_exit_hooks: vec![],
    }
}

// ── Strategies ───────────────────────────────────────────────────────────────

fn arb_container_id() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9]{7,31}"
}

// ── DaemonState invariants ────────────────────────────────────────────────────

proptest! {
    #[test]
    fn state_add_then_get_finds_record(id in arb_container_id()) {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = make_state(tmp.path());
        let rt = make_rt();
        let record = make_record(&id);

        rt.block_on(state.add_container(record));
        let found = rt.block_on(state.get_container(&id));

        prop_assert!(found.is_some(), "get after add returned None for id={id}");
        prop_assert_eq!(found.unwrap().info.id, id);
    }
}
```

- [ ] **Step 3: Run the test to verify it compiles and passes**

```bash
cargo test -p daemonbox state_add_then_get_finds_record -- --nocapture
```

Expected: 1 test passes (proptest runs 256 cases).

- [ ] **Step 4: Add the remaining 3 DaemonState invariant tests**

Append to `crates/daemonbox/tests/proptest_suite.rs`:

```rust
proptest! {
    #[test]
    fn state_remove_after_add_returns_none(id in arb_container_id()) {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = make_state(tmp.path());
        let rt = make_rt();

        rt.block_on(state.add_container(make_record(&id)));
        rt.block_on(state.remove_container(&id));
        let found = rt.block_on(state.get_container(&id));

        prop_assert!(found.is_none(), "get after add+remove returned Some for id={id}");
    }
}

proptest! {
    #[test]
    fn state_list_count_matches_adds(
        ids in proptest::collection::hash_set(arb_container_id(), 1..=8)
    ) {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = make_state(tmp.path());
        let rt = make_rt();

        for id in &ids {
            rt.block_on(state.add_container(make_record(id)));
        }
        let list = rt.block_on(state.list_containers());

        prop_assert_eq!(list.len(), ids.len(), "list count mismatch");
    }
}

proptest! {
    #[test]
    fn state_arbitrary_sequence_no_panic(
        adds in proptest::collection::hash_set(arb_container_id(), 1..=8),
        remove_count in 0_usize..=8_usize,
    ) {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = make_state(tmp.path());
        let rt = make_rt();

        for id in &adds {
            rt.block_on(state.add_container(make_record(id)));
        }

        let ids_vec: Vec<_> = adds.iter().collect();
        let to_remove = &ids_vec[..remove_count.min(ids_vec.len())];
        let mut removed = 0;
        for id in to_remove {
            if rt.block_on(state.remove_container(id)).is_some() {
                removed += 1;
            }
        }

        let list = rt.block_on(state.list_containers());
        prop_assert_eq!(list.len(), adds.len() - removed);
    }
}
```

- [ ] **Step 5: Run all DaemonState tests**

```bash
cargo test -p daemonbox -- --nocapture 2>&1 | tail -20
```

Expected: 4 proptest properties pass.

- [ ] **Step 6: Commit**

```bash
git add crates/daemonbox/Cargo.toml crates/daemonbox/tests/proptest_suite.rs
git commit -m "test(proptest): DaemonState invariants — add/remove/list consistency"
```

---

## Task 2: Handler input safety tests

**Files:**

- Modify: `crates/daemonbox/tests/proptest_suite.rs`

- [ ] **Step 1: Append the 3 handler safety tests**

Append to `crates/daemonbox/tests/proptest_suite.rs`:

```rust
// ── Handler input safety ──────────────────────────────────────────────────────

proptest! {
    #[test]
    fn handle_stop_unknown_id_is_error(id in arb_container_id()) {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = make_state(tmp.path());
        let rt = make_rt();

        let resp = rt.block_on(handle_stop(id.clone(), state));

        prop_assert!(
            matches!(resp, DaemonResponse::Error { .. }),
            "expected Error for unknown id={id}, got {resp:?}"
        );
    }
}

proptest! {
    #[test]
    fn handle_remove_unknown_id_is_error(id in arb_container_id()) {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = make_state(tmp.path());
        let deps = make_deps(tmp.path());
        let rt = make_rt();

        let resp = rt.block_on(handle_remove(id.clone(), state, deps));

        prop_assert!(
            matches!(resp, DaemonResponse::Error { .. }),
            "expected Error for unknown id={id}, got {resp:?}"
        );
    }
}

proptest! {
    #[test]
    fn handle_list_always_returns_container_list(
        ids in proptest::collection::hash_set(arb_container_id(), 0..=5)
    ) {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = make_state(tmp.path());
        let rt = make_rt();

        for id in &ids {
            rt.block_on(state.add_container(make_record(id)));
        }
        let resp = rt.block_on(handle_list(state));

        prop_assert!(
            matches!(resp, DaemonResponse::ContainerList { .. }),
            "expected ContainerList, got {resp:?}"
        );
    }
}
```

- [ ] **Step 2: Run all daemonbox proptest properties**

```bash
cargo test -p daemonbox -- --nocapture 2>&1 | tail -20
```

Expected: 7 proptest properties pass.

- [ ] **Step 3: Verify macOS quality gate still passes**

```bash
cargo xtask test-unit
```

Expected: all unit tests pass including the new daemonbox proptest suite.

- [ ] **Step 4: Commit**

```bash
git add crates/daemonbox/tests/proptest_suite.rs
git commit -m "test(proptest): handler input safety — stop/remove unknown IDs return Error, list always ContainerList"
```

---

## Task 3: CgroupConfig boundary tests (Linux-only)

**Files:**

- Modify: `crates/minibox/tests/proptest_suite.rs`

These tests require a real cgroup2 mount and root. They are gated `#[cfg(target_os = "linux")]` and skipped on macOS. Run with `just test-integration` on the VPS.

- [ ] **Step 1: Add the cgroup bound tests to the existing proptest suite**

Open `crates/minibox/tests/proptest_suite.rs` and append at the end:

```rust
// ── CgroupConfig boundary validation (Linux + root only) ─────────────────────

#[cfg(target_os = "linux")]
mod cgroup_props {
    use minibox::container::cgroups::{CgroupConfig, CgroupManager};
    use proptest::prelude::*;

    // NOTE: On unprivileged Linux (no root / no cgroup2 mount), `create_dir_all` will
    // fail with EACCES before the bounds check runs. The `prop_assert!(is_err())` still
    // passes, but for the wrong reason. These tests only exercise validation logic
    // correctly under `just test-integration` where `MINIBOX_CGROUP_ROOT` points to
    // a writable cgroup2 path with root privileges.
    proptest! {
        #[test]
        fn memory_below_4096_always_rejected(
            mem in 0_u64..4096_u64,
            id in "[a-z]{8,16}",
        ) {
            let config = CgroupConfig {
                memory_limit_bytes: Some(mem),
                ..Default::default()
            };
            let mgr = CgroupManager::new(&id, config);
            prop_assert!(
                mgr.create().is_err(),
                "expected Err for memory={mem}, got Ok"
            );
        }

        #[test]
        fn cpu_weight_out_of_range_rejected(
            weight in prop_oneof![Just(0_u64), (10_001_u64..=u64::MAX)],
            id in "[a-z]{8,16}",
        ) {
            let config = CgroupConfig {
                cpu_weight: Some(weight),
                ..Default::default()
            };
            let mgr = CgroupManager::new(&id, config);
            prop_assert!(
                mgr.create().is_err(),
                "expected Err for cpu_weight={weight}, got Ok"
            );
        }
    }
}
```

- [ ] **Step 2: Verify macOS test-unit still passes (Linux tests skipped)**

```bash
cargo xtask test-unit
```

Expected: existing minibox proptest tests still pass; `cgroup_props` module is compiled away on macOS — no failures.

- [ ] **Step 3: Commit**

```bash
git add crates/minibox/tests/proptest_suite.rs
git commit -m "test(proptest): cgroup config bounds — memory < 4096 and cpu_weight out of range always rejected (Linux-gated)"
```

- [ ] **Step 4: Note for VPS verification**

The cgroup tests run on the VPS under `just test-integration`. They are not verified locally. After merging, confirm with:

```bash
# On VPS (root, cgroup2 mount available):
just test-integration 2>&1 | grep -E "cgroup_props|FAILED|ok"
```

Expected: `memory_below_4096_always_rejected` and `cpu_weight_out_of_range_rejected` appear as `ok`.
