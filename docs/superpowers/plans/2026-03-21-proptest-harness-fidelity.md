# Proptest Harness Fidelity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix two measurement fidelity problems: (1) proptest cases each spin up their own Tokio runtime via `make_rt()` — 256+ allocations per property per run — replace with a single shared static runtime; (2) adapter microbenchmark closures call `rt.block_on()` on every timing iteration, measuring scheduler overhead rather than adapter work — replace with direct sync invocations on mock adapters.

**Architecture:** Two independent changes in separate files. The proptest fix uses `std::sync::OnceLock<Runtime>` (stable since Rust 1.70, no new dependency) as a module-level static. The adapter bench fix lifts the async calls to pre-resolved sync equivalents by adding `_sync` test helpers to mock adapters, or (simpler) just removing `block_on` for the two async mocks that have trivially synchronous implementations.

**Tech Stack:** Rust, `proptest`, `tokio`, `std::sync::OnceLock`

---

## File Map

| File | Change |
|---|---|
| `crates/daemonbox/tests/proptest_suite.rs` | Replace `make_rt()` per-case with a shared static `OnceLock<Runtime>` |
| `crates/minibox-bench/src/main.rs` | Fix `bench_adapter_suite` — move `rt` creation outside `nano_test` closures; add sync helpers for mock calls |
| `crates/minibox-lib/src/adapters/mocks.rs` | Add sync test-helper methods `has_image_sync` and `spawn_process_sync` (or expose existing sync internals) |

---

### Task 1: Replace `make_rt()` per-case with a static runtime in proptest_suite

**Background:** Every `proptest!` block calls `make_rt()` which does `Runtime::new()` — a full Tokio runtime instantiation including thread spawning. With 256 cases per property × 7 properties = ~1800 runtimes per test run. A single `OnceLock<Runtime>` initialised once per test binary is sufficient.

**Files:**
- Modify: `crates/daemonbox/tests/proptest_suite.rs:14-16`

- [ ] **Step 1: Write a failing test**

Add at the top of `proptest_suite.rs`:

```rust
#[test]
fn runtime_is_shared_not_per_call() {
    // This test documents the contract: RT.get_or_init should return the same
    // Runtime instance on repeated calls (pointer equality).
    let rt1 = runtime();
    let rt2 = runtime();
    assert!(std::ptr::eq(rt1, rt2), "runtime() must return the same instance");
}
```

- [ ] **Step 2: Run to verify it fails**

```bash
cargo test -p daemonbox --test proptest_suite runtime_is_shared 2>&1 | head -10
```

Expected: FAIL — `runtime()` function does not exist.

- [ ] **Step 3: Replace `make_rt()` with a static accessor**

At the top of `proptest_suite.rs`, replace:

```rust
fn make_rt() -> tokio::runtime::Runtime {
    tokio::runtime::Runtime::new().expect("tokio runtime")
}
```

With:

```rust
use std::sync::OnceLock;

static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

fn runtime() -> &'static tokio::runtime::Runtime {
    RT.get_or_init(|| tokio::runtime::Runtime::new().expect("tokio runtime"))
}
```

- [ ] **Step 4: Update all call sites**

Replace every `let rt = make_rt();` and `rt.block_on(...)` in the file.

There are 7 `proptest!` blocks. In each one, change:

```rust
// Before:
let rt = make_rt();
rt.block_on(state.add_container(record));

// After:
runtime().block_on(state.add_container(record));
```

Specifically, the changes needed per block:

**`state_add_then_get_finds_record`** — remove `let rt = make_rt();`, replace `rt.block_on` with `runtime().block_on` (3 calls)

**`state_remove_after_add_returns_none`** — same (3 calls)

**`state_list_count_matches_adds`** — same (2 calls in loop + 1 list call)

**`state_add_remove_sequence_list_count_is_consistent`** — same (loop + remove + list)

**`handle_stop_unknown_id_is_error`** — same (1 call)

**`handle_remove_unknown_id_is_error`** — same (1 call)

**`handle_list_always_returns_container_list`** — same (loop + list)

- [ ] **Step 5: Run tests to verify they all pass**

```bash
cargo test -p daemonbox --test proptest_suite
```

Expected: all 8 tests pass (7 proptest properties + 1 new shared-runtime test).

- [ ] **Step 6: Verify proptest still runs correct case counts**

```bash
cargo test --release -p daemonbox --test proptest_suite -- --nocapture 2>&1 | grep "proptest"
```

Expected: no failures, cases still exercised.

- [ ] **Step 7: Commit**

```bash
git add crates/daemonbox/tests/proptest_suite.rs
git commit -m "perf(proptest): replace per-case make_rt() with shared OnceLock<Runtime>"
```

---

### Task 2: Check mock adapter internals

**Background:** In `bench_adapter_suite`, two tests call `rt.block_on(async { mock.method().await })`. We need to know if the mock futures do any real async work or just return immediately.

**Files:**
- Read: `crates/minibox-lib/src/adapters/mocks.rs`

- [ ] **Step 1: Inspect mock implementations**

```bash
grep -A 10 "fn has_image\|fn spawn_process" crates/minibox-lib/src/adapters/mocks.rs
```

Expected: both return a simple `async { ... }` with no `.await` inside — they are trivially synchronous futures wrapped in async.

- [ ] **Step 2: Confirm the overhead source**

If `has_image` returns `async { true }` and `spawn_process` returns `async { Ok(SpawnResult { ... }) }`, then `rt.block_on(fut)` costs ~50–200 ns of Tokio scheduler overhead per iteration, completely dominating the ~1–5 ns of mock logic. Confirmed — the fix is to bypass `block_on` for these tests.

---

### Task 3: Add sync test helpers to mock adapters

**Files:**
- Modify: `crates/minibox-lib/src/adapters/mocks.rs`

- [ ] **Step 1: Write failing test**

Add to `mocks.rs` (or its test block):

```rust
#[test]
fn mock_registry_has_image_sync_available() {
    let reg = MockRegistry::new().with_cached_image("alpine", "latest");
    assert!(reg.has_image_sync("alpine", "latest"));
    assert!(!reg.has_image_sync("alpine", "missing"));
}
```

- [ ] **Step 2: Run to verify it fails**

```bash
cargo test -p minibox-lib mock_registry_has_image_sync
```

Expected: FAIL — `has_image_sync` does not exist.

- [ ] **Step 3: Add `has_image_sync` to `MockRegistry`**

`MockRegistryState.cached_images` is `Vec<(String, String)>` (name, tag). Use the tuple form:

In `mocks.rs`, add to the `MockRegistry` impl block (after `pull_count`):

```rust
/// Sync test helper — bypasses async machinery for benchmarks.
pub fn has_image_sync(&self, image: &str, tag: &str) -> bool {
    self.state
        .lock()
        .unwrap()
        .cached_images
        .contains(&(image.to_string(), tag.to_string()))
}
```

- [ ] **Step 4: Add `spawn_process_sync` to `MockRuntime`**

`SpawnResult` has fields `pid: u32` and `output_reader: Option<OwnedFd>` — must include both.

```rust
/// Sync test helper — bypasses async machinery for benchmarks.
pub fn spawn_process_sync(&self, _cfg: &ContainerSpawnConfig) -> Result<SpawnResult> {
    let mut state = self.state.lock().unwrap();
    state.spawn_count += 1;
    if !state.spawn_should_succeed {
        anyhow::bail!("mock spawn failure");
    }
    let pid = state.next_pid;
    state.next_pid += 1;
    Ok(SpawnResult { pid, output_reader: None })
}
```

(Import `SpawnResult` is already in scope at the top of `mocks.rs`.)

- [ ] **Step 5: Run tests**

```bash
cargo test -p minibox-lib mock_registry_has_image_sync
```

Expected: pass.

- [ ] **Step 6: Commit**

```bash
git add crates/minibox-lib/src/adapters/mocks.rs
git commit -m "feat(mocks): add has_image_sync and spawn_process_sync test helpers"
```

---

### Task 4: Fix `bench_adapter_suite` — remove `block_on` from timed closures

**Files:**
- Modify: `crates/minibox-bench/src/main.rs` — `bench_adapter_suite()` (line ~730)

- [ ] **Step 1: Write a failing test**

The contract to enforce: async adapter tests should use sync helpers so measured time reflects adapter logic. Add a doc-comment test:

```rust
// No unit test needed here — the change is behavioral. Verify via:
// cargo xtask bench --suite adapter
// and confirm reported ns values drop to < 10ns (from ~100ns with block_on).
```

- [ ] **Step 2: Replace `rt.block_on` calls with sync equivalents**

In `bench_adapter_suite`, replace the two async tests:

The sync helpers can only be called on concrete types, not `Arc<dyn Trait>`. The fix: use sync helpers for the `_direct` cases (eliminating scheduler noise from the baseline), and keep `rt.block_on` only for the `_trait_object` cases (which must go through the trait method). This means the `_direct` vs `_trait_object` comparison now shows: sync concrete ~1-10ns vs async trait-object dispatch + scheduler ~50-200ns — an honest order-of-magnitude picture.

```rust
// Before:
nano_test("registry_direct_has_image", iters, || {
    rt.block_on(async {
        black_box(registry_concrete.has_image("alpine", "latest")).await;
    });
}),
nano_test("registry_trait_object_has_image", iters, || {
    rt.block_on(async {
        black_box(registry_trait.has_image("alpine", "latest")).await;
    });
}),

// After:
nano_test("registry_direct_has_image", iters, || {
    black_box(registry_concrete.has_image_sync("alpine", "latest"));
}),
nano_test("registry_trait_object_has_image", iters, || {
    // Must use block_on — has_image_sync is not a trait method.
    // Measures vtable dispatch + async scheduler overhead combined.
    rt.block_on(async {
        black_box(registry_trait.has_image("alpine", "latest")).await;
    });
}),
```

And for `runtime_direct_spawn` / `runtime_trait_object_spawn`:

```rust
// Before:
nano_test("runtime_direct_spawn", iters, || {
    rt.block_on(async {
        black_box(runtime_concrete.spawn_process(&spawn_cfg).await).ok();
    });
}),
nano_test("runtime_trait_object_spawn", iters, || {
    rt.block_on(async {
        black_box(runtime_trait.spawn_process(&spawn_cfg).await).ok();
    });
}),

// After:
nano_test("runtime_direct_spawn", iters, || {
    black_box(runtime_concrete.spawn_process_sync(&spawn_cfg)).ok();
}),
nano_test("runtime_trait_object_spawn", iters, || {
    // Must use block_on — spawn_process_sync is not a trait method.
    rt.block_on(async {
        black_box(runtime_trait.spawn_process(&spawn_cfg).await).ok();
    });
}),
```

- [ ] **Step 3: Verify `rt` is still in scope**

`rt` is still needed for the `_trait_object` tests that must use `block_on`. Do NOT remove it. Run `cargo clippy -p minibox-bench` and confirm no unused variable warning for `rt`.

- [ ] **Step 4: Build and run adapter suite**

```bash
cargo build --release -p minibox-bench
./target/release/minibox-bench --suite adapter --iters 200 --out-dir /tmp/bench-adapter-test 2>/dev/null
cat /tmp/bench-adapter-test/*.txt
```

Expected: `registry_direct_has_image` and `registry_trait_object_has_image` now report < 10ns avg (vs ~100ns+ before). The trait-object overhead columns (`_direct` vs `_trait_object`) should now show a real and small difference.

- [ ] **Step 5: Run all bench tests**

```bash
cargo test -p minibox-bench
```

Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add crates/minibox-bench/src/main.rs
git commit -m "perf(bench): remove block_on from adapter microbench tight loops — measure adapter cost not scheduler overhead"
```

---

### Task 5: Final verification

- [ ] **Step 1: Run full proptest suite and note wall time**

```bash
time cargo test --release -p daemonbox --test proptest_suite
```

Note the time. Compare to before the change (check git log for previous run time if available). Expect a significant reduction (seconds → sub-second on fast hardware).

- [ ] **Step 2: Run adapter bench and confirm ns values are plausible**

```bash
./target/release/minibox-bench --suite adapter --iters 500 --out-dir /tmp/bench-final 2>/dev/null
cat /tmp/bench-final/*.txt
```

Expected: all adapter tests < 50ns avg. `arc_clone` and `downcast_to_concrete` should be ~1–5ns. Mock method calls should be ~5–20ns.

- [ ] **Step 3: Run `cargo xtask test-property` to confirm full gate still passes**

```bash
cargo xtask test-property
```

Expected: clean pass.
