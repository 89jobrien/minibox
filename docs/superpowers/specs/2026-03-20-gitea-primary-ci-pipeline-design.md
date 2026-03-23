---
status: approved
---

# Gitea-Primary CI Pipeline

**Date:** 2026-03-20

## Overview

Move Gitea to primary remote. GitHub becomes a push mirror. Full integration, e2e, property-based, and benchmark testing runs via Gitea Actions on jobrien-vm. GitHub Actions retains a lightweight macOS gate (fmt + clippy + unit tests only).

---

## Remote Topology

```
Local machine
  └── git push → origin (Gitea, 100.105.75.7:3000/joe/minibox)
                    └── push mirror → github (89jobrien/minibox)
```

**Local git config changes:**
- Rename `origin` → `github`
- Rename `gitea` → `origin`

**Gitea push mirror:** Configured in Gitea repo Settings → Mirror → Push Mirror, targeting `git@github.com:89jobrien/minibox.git`. Fires on every push to Gitea.

The `gh` CLI continues to work — it targets the `github` remote which remains named and accessible.

---

## Workflow Structure

File: `.gitea/workflows/ci.yml`

```
push to Gitea
      │
      ├────────────────────────────────┐
      ▼                                ▼
  [unit]                          [property]
  cargo xtask test-unit           proptest suite
  no root · ~30s                  no root · ~60s
      │                                │
      └──────────┬─────────────────────┘
                 │ both pass
      ┌──────────┴─────────────────────┐
      ▼                                ▼
  [integration]                   [e2e]
  cgroup tests                    daemon+CLI tests
  sudo · ~2min                    sudo · ~3min
      │                                │
      └──────────┬─────────────────────┘
                 │ both pass
                 ▼
             [bench]
             cargo xtask bench
             sudo · ~1min
```

All jobs run on the `act_runner` system service on jobrien-vm. `mise` is at `~/.local/bin/mise` — not in default PATH for systemd services; always use full path.

### Job definitions

**unit**
```yaml
- run: ~/.local/bin/mise exec -- cargo xtask test-unit
```

**property**
```yaml
- run: ~/.local/bin/mise exec -- cargo test -p linuxbox --test proptest_suite
```

**integration** (needs: [unit, property])
```yaml
- run: sudo -E ~/.cargo/bin/cargo test -p miniboxd --test cgroup_tests -- --test-threads=1 --nocapture
- run: sudo -E ~/.cargo/bin/cargo test -p miniboxd --test integration_tests -- --test-threads=1 --ignored --nocapture
- if: always()
  run: sudo -E ~/.local/bin/mise exec -- cargo xtask nuke-test-state
```

**e2e** (needs: [unit, property])

`cargo xtask test-e2e-suite` internally invokes `sudo -E` for the test binary — the xtask itself handles escalation. Run via mise to pick up the correct toolchain:
```yaml
- run: ~/.local/bin/mise exec -- cargo xtask test-e2e-suite
- if: always()
  run: sudo -E ~/.local/bin/mise exec -- cargo xtask nuke-test-state
```

**bench** (needs: [integration, e2e])

bench binary runs as root to avoid permission issues with perf counters:
```yaml
- run: sudo -E ~/.local/bin/mise exec -- cargo xtask bench
```

---

## Property-Based Tests

New file: `crates/linuxbox/tests/proptest_suite.rs`

`proptest` is added as a crate-local dev-dependency in `crates/linuxbox/Cargo.toml` only — not hoisted to workspace `[workspace.dependencies]` since no other crate uses it.

### Targets

| Target | Invariant |
|--------|-----------|
| Protocol roundtrip | `decode(encode(msg)) == msg` for any valid `Request` / `Response` |
| `ImageRef` parsing | Any syntactically valid ref string parses without panic |
| `validate_layer_path` | Never panics on arbitrary input; only returns `Ok` or a clean `ImageError` |
| Tar path rejection | Any path containing `..` components is always rejected |

---

## GitHub Actions

`.github/workflows/ci.yml` is trimmed to the `macos` job only (fmt + clippy + unit tests). The `linux` self-hosted job is removed — that work moves to Gitea. The macOS job remains as a cross-platform compile and lint gate on GitHub-hosted runners.

---

## Implementation Steps

1. Rename git remotes locally: `gitea` → `origin`, `origin` → `github`
2. Configure Gitea push mirror to GitHub in repo Settings → Mirror → Push Mirror
3. Remove `linux` job from `.github/workflows/ci.yml`
4. Add `proptest` to `crates/linuxbox/Cargo.toml` under `[dev-dependencies]`
5. Write `crates/linuxbox/tests/proptest_suite.rs` with four test targets
6. Create `.gitea/workflows/ci.yml` with the five-job pipeline
7. Push to new `origin` (Gitea) and verify workflow triggers
8. Verify GitHub mirror receives the push
