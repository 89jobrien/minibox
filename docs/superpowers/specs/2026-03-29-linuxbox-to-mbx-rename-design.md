# Design: Rename `linuxbox` Crate to `mbx`

**Date:** 2026-03-29
**Status:** Approved

## Summary

Rename the `linuxbox` crate to `mbx` — both the directory (`crates/linuxbox/` → `crates/mbx/`) and the crate name. This is a mechanical refactor with no behavioral changes.

## Scope

- 118 occurrences of `linuxbox` across 45 files
- Crate directory: `crates/linuxbox/` → `crates/mbx/`
- Crate name: `linuxbox` → `mbx`
- All dependents updated in-place

## Changes

### 1. Directory rename
```
git mv crates/linuxbox crates/mbx
```

### 2. Crate manifest (`crates/mbx/Cargo.toml`)
```toml
name = "mbx"   # was: linuxbox
```

### 3. Workspace manifest (`Cargo.toml`)
- Update workspace member: `"crates/linuxbox"` → `"crates/mbx"`
- Update workspace dep: `linuxbox = { path = "crates/linuxbox" }` → `mbx = { path = "crates/mbx" }`

### 4. Dependent crate manifests
Five crates declare `linuxbox = { workspace = true }`:
- `crates/miniboxd/Cargo.toml`
- `crates/macbox/Cargo.toml`
- `crates/daemonbox/Cargo.toml`
- `crates/minibox-bench/Cargo.toml`
- `crates/winbox/Cargo.toml` (also in `ignored` list)

All become `mbx = { workspace = true }`.

### 5. Rust source files
All `use linuxbox::` and `linuxbox::` qualified paths become `use mbx::` and `mbx::` respectively. Affects ~45 `.rs` files.

### 6. `recipe.json`
cargo-chef artifact — update `linuxbox` crate name to `mbx` manually (or regenerate with `cargo chef prepare`).

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
