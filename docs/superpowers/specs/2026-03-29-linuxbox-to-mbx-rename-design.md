# Design: Rename `minibox` Crate to `minibox`

**Date:** 2026-03-29
**Status:** Approved

## Summary

Rename the `minibox` crate to `minibox` — both the directory (`crates/minibox/` → `crates/minibox/`) and the crate name. This is a mechanical refactor with no behavioral changes.

## Scope

- 118 occurrences of `minibox` across 45 files
- Crate directory: `crates/minibox/` → `crates/minibox/`
- Crate name: `minibox` → `minibox`
- All dependents updated in-place

## Changes

### 1. Directory rename

```
git mv crates/minibox crates/minibox
```

### 2. Crate manifest (`crates/minibox/Cargo.toml`)

```toml
name = "minibox"   # was: minibox
```

### 3. Workspace manifest (`Cargo.toml`)

- Update workspace member: `"crates/minibox"` → `"crates/minibox"`
- Update workspace dep: `minibox = { path = "crates/minibox" }` → `minibox = { path = "crates/minibox" }`

### 4. Dependent crate manifests

Five crates declare `minibox = { workspace = true }`:

- `crates/miniboxd/Cargo.toml`
- `crates/macbox/Cargo.toml`
- `crates/daemonbox/Cargo.toml`
- `crates/minibox-bench/Cargo.toml`
- `crates/winbox/Cargo.toml` (also in `ignored` list)

All become `minibox = { workspace = true }`.

### 5. Rust source files

All `use minibox::` and `minibox::` qualified paths become `use minibox::` and `minibox::` respectively. Affects ~45 `.rs` files.

### 6. `recipe.json`

cargo-chef artifact — update `minibox` crate name to `minibox` manually (or regenerate with `cargo chef prepare`).

### 7. Documentation

Update all references in `CLAUDE.md` and any docs under `docs/`.

## Verification Gate

```bash
cargo check --workspace
cargo clippy --workspace -- -D warnings
cargo xtask test-unit
```

## Non-Goals

- No behavioral changes
- No API changes
- No feature additions
