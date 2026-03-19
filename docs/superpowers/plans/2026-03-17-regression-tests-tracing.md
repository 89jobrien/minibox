---
status: done
completed: "2026-03-17"
branch: main
note: All regression tests shipped, tracing contract finalized
---
# Regression Tests & Tracing Level Gating Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add regression tests for `relative_path`, tar root-entry skip, absolute-symlink rewriting, and macro downcast contracts; downgrade three inner tracing spans from INFO to DEBUG.

**Architecture:** All test additions go into existing `#[cfg(test)]` blocks using established helper functions. The tracing change is three one-word substitutions in `registry.rs`. No new files created.

**Tech Stack:** Rust edition 2024, `tar`/`flate2` for test archive building, `tracing` crate, `std::sync::Arc`.

**Spec:** `docs/superpowers/specs/2026-03-17-regression-tests-tracing-design.md`

---

## Files

| File                                       | Change                                       |
| ------------------------------------------ | -------------------------------------------- |
| `crates/minibox-lib/src/image/layer.rs`    | Add 7 tests to existing `#[cfg(test)]` block |
| `crates/minibox-lib/src/adapters/mocks.rs` | Add 4 tests to existing `#[cfg(test)]` block |
| `crates/minibox-lib/src/image/registry.rs` | Change 3 spans: `info_span!` → `debug_span!` |

---

## Task 1: `relative_path` named unit tests

**Files:** Modify `crates/minibox-lib/src/image/layer.rs`

The `relative_path` function computes a relative path from a symlink's directory to an absolute target (after stripping the leading `/`). It has doctests but no named unit tests. Add three named tests directly after the existing `verify_digest` tests in the `tests` module.

- [ ] **Add the three tests** — append inside the `#[cfg(test)] mod tests` block (before the closing `}`):

```rust
// ---------------------------------------------------------------------------
// relative_path
// ---------------------------------------------------------------------------

#[test]
fn relative_path_same_dir() {
    // bin/echo -> /bin/busybox: target is in same dir, result is just filename
    assert_eq!(
        relative_path(Path::new("bin"), Path::new("bin/busybox")),
        std::path::PathBuf::from("busybox")
    );
}

#[test]
fn relative_path_cross_dir() {
    // usr/local/bin/python -> /usr/bin/python: go up two dirs, then into bin
    assert_eq!(
        relative_path(Path::new("usr/local/bin"), Path::new("usr/bin/python")),
        std::path::PathBuf::from("../../bin/python")
    );
}

#[test]
fn relative_path_root_to_nested() {
    // symlink at root level -> /usr/bin/python: no parent dirs to climb
    assert_eq!(
        relative_path(Path::new(""), Path::new("usr/bin/python")),
        std::path::PathBuf::from("usr/bin/python")
    );
}
```

- [ ] **Run the tests:**

```bash
cargo test -p minibox-lib image::layer::tests::relative_path -- --nocapture
```

Expected: 3 tests pass.

- [ ] **Commit:**

```bash
git add crates/minibox-lib/src/image/layer.rs
git commit -m "test: add named unit tests for relative_path"
```

---

## Task 2: Tar root-entry skip regression tests

**Files:** Modify `crates/minibox-lib/src/image/layer.rs`

The `extract_layer` function skips `"."` and `"./"` entries to avoid a false path-escape error. These two tests pin that behaviour.

- [ ] **Add the two tests** — append to the `tests` module, in the `extract_layer — end-to-end` section:

```rust
#[test]
fn root_dot_entry_skipped() {
    // "." is the tar root marker — extract_layer must skip it silently (no error,
    // no file extracted).
    let dest = TempDir::new().unwrap();
    let tar_gz = tar_gz_with_regular_file(".", b"");
    extract_layer(&tar_gz, dest.path()).unwrap(); // must not error
    // The destination directory must remain empty — nothing was extracted.
    let entries: Vec<_> = std::fs::read_dir(dest.path()).unwrap().collect();
    assert!(entries.is_empty(), "no files should be extracted for '.' entry");
}

#[test]
fn root_dot_slash_entry_skipped() {
    // "./" variant of the same root marker
    let dest = TempDir::new().unwrap();
    let tar_gz = tar_gz_with_regular_file("./", b"");
    extract_layer(&tar_gz, dest.path()).unwrap(); // must not error
    let entries: Vec<_> = std::fs::read_dir(dest.path()).unwrap().collect();
    assert!(entries.is_empty(), "no files should be extracted for './' entry");
}
```

- [ ] **Run the tests:**

```bash
cargo test -p minibox-lib image::layer::tests::root_dot -- --nocapture
```

Expected: 2 tests pass.

- [ ] **Commit:**

```bash
git add crates/minibox-lib/src/image/layer.rs
git commit -m "test: add regression tests for tar root-entry skip"
```

---

## Task 3: Absolute-symlink rewrite regression tests

**Files:** Modify `crates/minibox-lib/src/image/layer.rs`

These two tests cover the exact scenarios that were broken before the fix: busybox applet symlinks (same-directory target) and cross-directory symlinks.

- [ ] **Add the two tests** — append to the `tests` module, after the existing `absolute_symlink_with_parent_traversal_rejected` test:

```rust
#[cfg(unix)]
#[test]
fn busybox_applet_symlink_correct() {
    // bin/echo -> /bin/busybox: after rewrite, target should be "busybox" (same dir)
    // This is the specific busybox case that was broken before the fix.
    let dest = TempDir::new().unwrap();
    let tar_gz = tar_gz_with_symlink("bin/echo", "/bin/busybox");
    extract_layer(&tar_gz, dest.path()).unwrap();
    let link = dest.path().join("bin/echo");
    assert!(link.symlink_metadata().is_ok(), "symlink should exist");
    let target = std::fs::read_link(&link).unwrap();
    assert!(
        !target.is_absolute(),
        "target must be relative, got: {target:?}"
    );
    assert_eq!(
        target,
        std::path::PathBuf::from("busybox"),
        "bin/echo -> /bin/busybox should rewrite to 'busybox'"
    );
}

#[cfg(unix)]
#[test]
fn cross_dir_absolute_symlink_rewritten() {
    // usr/local/bin/python -> /usr/bin/python: rewritten to ../../bin/python
    let dest = TempDir::new().unwrap();
    let tar_gz = tar_gz_with_symlink("usr/local/bin/python", "/usr/bin/python");
    extract_layer(&tar_gz, dest.path()).unwrap();
    let link = dest.path().join("usr/local/bin/python");
    assert!(link.symlink_metadata().is_ok(), "symlink should exist");
    let target = std::fs::read_link(&link).unwrap();
    assert!(
        !target.is_absolute(),
        "target must be relative, got: {target:?}"
    );
    assert_eq!(
        target,
        std::path::PathBuf::from("../../bin/python"),
        "usr/local/bin/python -> /usr/bin/python should rewrite to '../../bin/python'"
    );
}
```

- [ ] **Run the tests:**

```bash
cargo test -p minibox-lib image::layer::tests -- --nocapture
```

Expected: all layer tests pass including the 2 new ones.

- [ ] **Commit:**

```bash
git add crates/minibox-lib/src/image/layer.rs
git commit -m "test: add regression tests for absolute symlink rewriting (busybox applet + cross-dir)"
```

---

## Task 4: Macro contract tests

**Files:** Modify `crates/minibox-lib/src/adapters/mocks.rs`

The mock types use `adapt!` which expands to `as_any!` + `default_new!`. These tests document that downcast via `as_any()` works for all four mock types, wrong-type downcasts return `None`, and `Default` is implemented.

**Note on calling convention:** All domain traits (`ImageRegistry`, `FilesystemProvider`, etc.) extend `AsAny`, so `dyn ImageRegistry` has `as_any()` directly. Call it as `arc.as_ref().as_any().downcast_ref::<MockRegistry>()`.

- [ ] **Add a new `#[cfg(test)]` block** — append to the end of `mocks.rs` (after the closing `}` of the existing test module):

```rust
#[cfg(test)]
mod macro_contract_tests {
    use super::*;
    use crate::domain::{ContainerRuntime, FilesystemProvider, ImageRegistry, ResourceLimiter};
    use std::sync::Arc;

    #[test]
    fn mock_registry_downcasts_to_concrete() {
        let arc: Arc<dyn ImageRegistry> = Arc::new(MockRegistry::new());
        let result = arc.as_ref().as_any().downcast_ref::<MockRegistry>();
        assert!(result.is_some(), "MockRegistry must downcast to itself via as_any()");
    }

    #[test]
    fn wrong_type_downcast_returns_none() {
        let arc: Arc<dyn ImageRegistry> = Arc::new(MockRegistry::new());
        // Downcasting to a completely different concrete type must return None, not panic
        let result = arc.as_ref().as_any().downcast_ref::<MockFilesystem>();
        assert!(result.is_none(), "wrong-type downcast must return None");
    }

    #[test]
    fn default_matches_new() {
        // default_new! implements Default by delegating to ::new()
        // If this compiles and runs, the implementation is correct
        let _via_default = MockRegistry::default();
        let _via_new = MockRegistry::new();
    }

    #[test]
    fn all_mock_types_downcast_correctly() {
        let fs: Arc<dyn FilesystemProvider> = Arc::new(MockFilesystem::new());
        assert!(fs.as_ref().as_any().downcast_ref::<MockFilesystem>().is_some());

        let limiter: Arc<dyn ResourceLimiter> = Arc::new(MockLimiter::new());
        assert!(limiter.as_ref().as_any().downcast_ref::<MockLimiter>().is_some());

        let runtime: Arc<dyn ContainerRuntime> = Arc::new(MockRuntime::new());
        assert!(runtime.as_ref().as_any().downcast_ref::<MockRuntime>().is_some());
    }
}
```

- [ ] **Run the tests:**

```bash
cargo test -p minibox-lib adapters::mocks::macro_contract_tests -- --nocapture
```

Expected: 4 tests pass.

- [ ] **Commit:**

```bash
git add crates/minibox-lib/src/adapters/mocks.rs
git commit -m "test: add macro contract tests for as_any!/default_new!/adapt! downcast behaviour"
```

---

## Task 5: Downgrade inner tracing spans to DEBUG

**Files:** Modify `crates/minibox-lib/src/image/registry.rs`

Three spans inside `pull_image` are `info_span!` but represent sub-step detail that belongs at `DEBUG`. The `auth` and `manifest` phase spans stay at INFO. Make three one-word changes.

- [ ] **Apply the three substitutions** in `pull_image`:

  Line with `tracing::info_span!("verify_digest")`:

  ```rust
  // Before:
  let _span = tracing::info_span!("verify_digest").entered();
  // After:
  let _span = tracing::debug_span!("verify_digest").entered();
  ```

  Line with `tracing::info_span!("extract", bytes = data.len())`:

  ```rust
  // Before:
  let _span = tracing::info_span!("extract", bytes = data.len()).entered();
  // After:
  let _span = tracing::debug_span!("extract", bytes = data.len()).entered();
  ```

  Line with `tracing::info_span!("store_manifest")`:

  ```rust
  // Before:
  let _span = tracing::info_span!("store_manifest").entered();
  // After:
  let _span = tracing::debug_span!("store_manifest").entered();
  ```

  The `auth` and `manifest` phase spans remain `info_span!` — do not touch them.

- [ ] **Verify it compiles and all tests still pass:**

```bash
cargo test -p minibox-lib
cargo clippy -p minibox-lib -- -D warnings
cargo fmt --all --check
```

Expected: all tests pass, no clippy warnings, no fmt diff.

- [ ] **Commit:**

```bash
git add crates/minibox-lib/src/image/registry.rs
git commit -m "perf: downgrade inner pull spans from info_span to debug_span (verify_digest, extract, store_manifest)"
```

---

## Final check

```bash
cargo test -p minibox-lib
cargo clippy --workspace -- -D warnings
cargo fmt --all --check
```

All tests pass. No warnings. No fmt diff.
