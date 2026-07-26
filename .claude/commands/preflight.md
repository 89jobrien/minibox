---
name: preflight
description: >
  Use at the start of any minibox dev session, before running tests, or before a commit.
  Validates shell, toolchain, 1Password, and repo state. Mirrors the SessionStart hook output.
disable-model-invocation: false
---

# preflight — Environment Validation

Validates the minibox development environment. Run at session start or before any task that
requires a clean, fully-configured environment.

## Step 1 — Run preflight script

```bash
nu scripts/preflight.nu
```

Expected output (all checks must pass):

```
=== minibox preflight ===
[ok]   shell
[ok]   cargo on PATH
[ok]   just on PATH
[ok]   rustup on PATH
[ok]   Rust toolchain active
[ok]   CARGO_TARGET_DIR set
[ok]   xtask available
[ok]   op on PATH
[ok]   1Password authed
[ok]   git repo clean

preflight passed 10/10
```

## Step 2 — Interpret failures

| Failing check | Fix |
|---|---|
| `cargo on PATH` | `mise exec -- cargo --version` or install rustup |
| `CARGO_TARGET_DIR set` | Set in `.envrc`: `export CARGO_TARGET_DIR=~/.minibox/cache/target/` |
| `xtask available` | `cargo build -p xtask` |
| `1Password authed` | Open 1Password app and unlock it |
| `git repo clean` | Commit or stash uncommitted changes |

## Step 3 — VPS targets

If the task targets the VPS (bench-vps, test-e2e, Linux build), run the VPS health skill
after local preflight passes:

```
/vps-health
```

## Key Rules

- **Fix all `[fail]` before proceeding** — a partial environment produces misleading results
- **`CARGO_TARGET_DIR not set` is a WARN in CI, FAIL locally** — shared cache is required
  for worktrees to avoid redundant rebuilds
- **1Password unlock is non-blocking** — SSH signing still works once unlocked; no config needed
