# Regression Tests & Tracing Level Gating

**Date:** 2026-03-17
**Scope:** macOS-runnable unit tests + tracing hotpath fix
**Crates touched:** `minibox-lib` (tests + code change)

---

## Motivation

The council analysis flagged three gaps from recent work:

1. The `"."` / `"./"` tar root-entry skip and the absolute-symlink rewrite fix in `layer.rs` have no regression coverage.
2. The `as_any!`, `default_new!`, and `adapt!` macros (defined in `minibox-macros`, used in `minibox-lib`) have no tests documenting downcast contracts or failure modes.
3. Three fine-grained inner spans in `registry.rs` (`verify_digest`, `extract`, `store_manifest`) fire at `INFO` level, adding span overhead where `DEBUG` is more appropriate.

All work runs on macOS. No Linux-only or root-required tests are included.

---

## Area 1 — `layer.rs` regression tests

**File:** `crates/minibox-lib/src/image/layer.rs`, existing `#[cfg(test)]` block.

### `relative_path` unit tests

The `relative_path` function (used by the absolute-symlink rewrite path) has two doctests already. The named unit tests below are complementary — they give each case an explicit name for regression tracking and verify the edge case of an empty `from_dir`:

| Test | Input `(from_dir, to)` | Expected result |
|------|------------------------|-----------------|
| `relative_path_same_dir` | `("bin", "bin/busybox")` | `"busybox"` |
| `relative_path_cross_dir` | `("usr/local/bin", "usr/bin/python")` | `"../../bin/python"` |
| `relative_path_root_to_nested` | `("", "usr/bin/python")` | `"usr/bin/python"` |

### Tar root-entry skip tests

The `extract_layer` function silently skips `"."` and `"./"` entries. Without a test, a future refactor could accidentally re-enable the false path-escape error for these entries. Add:

| Test | Tar builder call | Expected outcome |
|------|-----------------|-----------------|
| `root_dot_entry_skipped` | `tar_gz_with_regular_file(".", b"")` | `Ok(())`, no file created |
| `root_dot_slash_entry_skipped` | `tar_gz_with_regular_file("./", b"")` | `Ok(())`, no file created |

Both call the existing `tar_gz_with_regular_file` builder with `"."` or `"./"` as the `name` argument. After extraction, assert that no file named `"."` or `"./"` appears in the destination directory.

### Absolute-symlink rewrite tests

The busybox applet case (symlink in same directory as its target) and the cross-directory case are the two scenarios that were broken before the fix. Add:

| Test | Symlink entry | Expected target after extraction |
|------|--------------|----------------------------------|
| `busybox_applet_symlink_correct` | `bin/echo -> /bin/busybox` | `"busybox"` (relative, same dir) |
| `cross_dir_absolute_symlink_rewritten` | `usr/local/bin/python -> /usr/bin/python` | `"../../bin/python"` |

Both are `#[cfg(unix)]` and use `tar_gz_with_symlink`. After calling `extract_layer`, read the symlink with `std::fs::read_link` and assert the target is relative (not absolute).

---

## Area 2 — Macro contract tests

**File:** `crates/minibox-lib/src/adapters/mocks.rs`, new `#[cfg(test)]` block at bottom.

The `as_any!` macro body uses `crate::domain::AsAny`. Because `macro_rules!` path resolution happens at the call site, `crate` resolves to whichever crate expands the macro — in this case `minibox-lib`. `minibox-macros` itself has no `domain` module, so tests must live in `minibox-lib`. The mock adapters (`MockRegistry`, `MockFilesystem`, `MockLimiter`, `MockRuntime`) already use `adapt!`, making `mocks.rs` the natural test site.

**Calling convention:** `Arc<dyn Trait>` does not auto-deref to `AsAny`. Use `arc.as_ref().as_any()` to reach the trait method through the `dyn` reference.

### Tests

| Test | What it proves |
|------|---------------|
| `mock_registry_downcasts_to_concrete` | `arc.as_ref().as_any().downcast_ref::<MockRegistry>()` returns `Some` |
| `wrong_type_downcast_returns_none` | Same arc downcasted to `MockFilesystem` returns `None` (no panic) |
| `default_matches_new` | `MockRegistry::default()` compiles and runs (proves `default_new!` delegates to `::new()`) |
| `all_mock_types_downcast_correctly` | `MockFilesystem`, `MockLimiter`, `MockRuntime` each downcast to their own concrete type |

These tests document the contract: "anything passed through `adapt!` can be recovered as its concrete type via `as_any()`; wrong-type downcasts return `None` rather than panicking."

---

## Area 3 — Tracing level adjustment in `registry.rs`

**File:** `crates/minibox-lib/src/image/registry.rs`, `pull_image` method.

### Problem

Three spans inside `pull_image` are currently `info_span!`:

```rust
let _span = tracing::info_span!("verify_digest").entered();   // inside layer loop (per-layer)
let _span = tracing::info_span!("extract", bytes = ...).entered(); // inside layer loop (per-layer)
// ...
let _span = tracing::info_span!("store_manifest").entered();  // outside layer loop (once per pull)
```

`verify_digest` and `extract` fire once per layer (hot path under concurrent pulls). `store_manifest` fires once per image pull. All three represent sub-step detail already captured by the surrounding `info_span!("layer", ...)` and `#[instrument]` spans; they belong at `DEBUG`.

There are also two manual `info_span!` calls for top-level pull phases that should **stay** at INFO:
```rust
.instrument(tracing::info_span!("auth"))    // top-level auth phase
.instrument(tracing::info_span!("manifest")) // top-level manifest phase
```

### Change

Replace only the three inner spans with `debug_span!`:

```rust
let _span = tracing::debug_span!("verify_digest").entered();
let _span = tracing::debug_span!("extract", bytes = data.len()).entered();
let _span = tracing::debug_span!("store_manifest").entered();
```

**What stays at INFO:**
- `info_span!("auth")` and `info_span!("manifest")` — top-level pull phase spans
- `info_span!("layer", ...)` — per-layer outer span
- All `info!` timing summary lines (auth time, manifest time, per-layer breakdown)
- `#[instrument]` on `pull_image`, `authenticate`, `get_manifest`, `pull_layer`

**What moves to DEBUG:**
- `verify_digest` span (per-layer)
- `extract` span (per-layer)
- `store_manifest` span (once per pull)

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
