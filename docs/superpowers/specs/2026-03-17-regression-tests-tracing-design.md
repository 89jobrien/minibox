# Regression Tests & Tracing Level Gating

**Date:** 2026-03-17
**Scope:** macOS-runnable unit tests + tracing hotpath fix
**Crates touched:** `minibox-lib`, `minibox-macros` (tests only), `registry.rs` (code change)

---

## Motivation

The council analysis flagged three gaps from recent work:

1. The `"."` / `"./"` tar root-entry skip and the absolute-symlink rewrite fix in `layer.rs` have no regression coverage.
2. The `minibox-macros` crate (`as_any!`, `default_new!`, `adapt!`) has no tests documenting downcast contracts or failure modes.
3. Fine-grained inner spans in `registry.rs` (`verify_digest`, `extract`, `store_manifest`) fire at `INFO` level on every layer pull, adding span overhead to the hot path that should only appear at `DEBUG`.

All work runs on macOS. No Linux-only or root-required tests are included.

---

## Area 1 — `layer.rs` regression tests

**File:** `crates/minibox-lib/src/image/layer.rs`, existing `#[cfg(test)]` block.

### `relative_path` unit tests

The `relative_path` function (used by the absolute-symlink rewrite path) has doctests but no named unit tests. Add:

| Test | Input `(from_dir, to)` | Expected result |
|------|------------------------|-----------------|
| `relative_path_same_dir` | `("bin", "bin/busybox")` | `"busybox"` |
| `relative_path_cross_dir` | `("usr/local/bin", "usr/bin/python")` | `"../../bin/python"` |
| `relative_path_root_to_nested` | `("", "usr/bin/python")` | `"usr/bin/python"` |

### Tar root-entry skip tests

The `extract_layer` function silently skips `"."` and `"./"` entries. Without a test, a future refactor could accidentally re-enable the false path-escape error for these entries. Add:

| Test | Tar contains | Expected outcome |
|------|-------------|-----------------|
| `root_dot_entry_skipped` | single `"."` regular-file entry | `Ok(())`, no file created |
| `root_dot_slash_entry_skipped` | single `"./"` regular-file entry | `Ok(())`, no file created |

These use the existing `tar_gz_with_regular_file` builder.

### Absolute-symlink rewrite tests

The busybox applet case (symlink in same directory as target) and the cross-directory case are the two scenarios that were broken before the fix. Add:

| Test | Symlink entry | Expected target after extraction |
|------|--------------|----------------------------------|
| `busybox_applet_symlink_correct` | `bin/echo -> /bin/busybox` | `"busybox"` (relative, same dir) |
| `cross_dir_absolute_symlink_rewritten` | `usr/local/bin/python -> /usr/bin/python` | `"../../bin/python"` |

Both are `#[cfg(unix)]` and use `tar_gz_with_symlink`. Both verify the symlink is relative after extraction.

---

## Area 2 — Macro contract tests

**File:** `crates/minibox-lib/src/adapters/mocks.rs`, new `#[cfg(test)]` block at bottom.

The `as_any!` macro hardcodes `crate::domain::AsAny`, so tests must live inside `minibox-lib`. The mock adapters (`MockRegistry`, `MockFilesystem`, `MockLimiter`, `MockRuntime`) already use `adapt!`, making `mocks.rs` the natural test site.

### Tests

| Test | What it proves |
|------|---------------|
| `mock_registry_downcasts_to_concrete` | `Arc<dyn ImageRegistry>` holding `MockRegistry` returns `Some(&MockRegistry)` from `as_any().downcast_ref()` |
| `wrong_type_downcast_returns_none` | Same `Arc` downcasted to `MockFilesystem` returns `None` |
| `default_matches_new` | `MockRegistry::default()` compiles and runs (proves `default_new!` delegates to `::new()`) |
| `all_mock_types_downcast_correctly` | `MockFilesystem`, `MockLimiter`, `MockRuntime` each downcast to their own concrete type |

These tests document the contract: "anything passed through `adapt!` can be recovered as its concrete type via `as_any()`; wrong-type downcasts return `None` rather than panicking."

---

## Area 3 — Tracing level adjustment in `registry.rs`

**File:** `crates/minibox-lib/src/image/registry.rs`, `pull_image` method.

### Problem

Three spans inside `pull_image` are currently `info_span!`:

```rust
let _span = tracing::info_span!("verify_digest").entered();
let _span = tracing::info_span!("extract", bytes = data.len()).entered();
let _span = tracing::info_span!("store_manifest").entered();
```

These fire on every layer pull when `RUST_LOG=info` (the default in production), adding span creation and subscriber notification overhead to what are already CPU-bound operations. The outer per-layer `info_span!("layer", ...)` already captures the important timing; these inner spans are sub-step detail that belongs at `DEBUG`.

### Change

Replace the three `info_span!` calls with `debug_span!`:

```rust
let _span = tracing::debug_span!("verify_digest").entered();
let _span = tracing::debug_span!("extract", bytes = data.len()).entered();
let _span = tracing::debug_span!("store_manifest").entered();
```

**What stays at INFO:**
- The outer `info_span!("layer", ...)` per-layer span
- All `info!` timing summary lines (auth time, manifest time, per-layer breakdown)
- `#[instrument]` on `pull_image`, `authenticate`, `get_manifest`, `pull_layer`

**What moves to DEBUG:**
- Sub-step spans within a layer: `verify_digest`, `extract`, `store_manifest`

`Instant::now()` calls are not gated — they are cheap vDSO reads and gating them would add conditional complexity for negligible gain.

---

## Non-goals

- No Linux-only or root-required tests (those belong in `just test-integration`)
- No benchmark additions (existing `trait_overhead` bench covers downcast cost)
- No `close_extra_fds` or `pivot_root_to` tests (Linux-only, deferred)
- No `close_range(2)` optimization or streaming pull parallelism (separate effort)

---

## Quality gates

After implementing, run on macOS:

```bash
cargo test -p minibox-lib
cargo fmt --all --check
cargo clippy --workspace -- -D warnings
```

All new tests must pass on macOS without root.
