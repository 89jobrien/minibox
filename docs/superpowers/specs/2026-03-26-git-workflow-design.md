# Minibox Git Workflow Design

**Date:** 2026-03-26
**Status:** approved
**Scope:** Git branching model, CI pipeline, auto-promotion, cleanup automation

---

## Overview

Three-tier stability pipeline for minibox: `main` (develop) → `next` (validated) → `stable` (release). Every commit on every branch must compile. Designed to support future Maestro integration (mbx as Cargo dependency) and self-hosted CI via minibox containers.

## Branch Model

```
feature/*  ──┐
hotfix/*   ──┤
chore/*    ──┴──► main (develop) ──auto──► next (validated) ──manual──► stable (release)
                                                                            │
                                                                        v* tag → GitHub Release
```

### Branch Purposes

| Branch      | Role                                                                | Who commits                | Deletion policy                      |
| ----------- | ------------------------------------------------------------------- | -------------------------- | ------------------------------------ |
| `main`      | Active R&D. All feature work merges here.                           | Direct push or PR merge    | Never deleted                        |
| `next`      | Validated accumulator. Auto-promoted from `main` when CI green.     | GitHub Actions only        | Never deleted                        |
| `stable`    | Maestro-consumable. API-stable mbx crate. Tagged releases cut here. | Manual promote from `next` | Never deleted                        |
| `feature/*` | Short-lived topic branches                                          | Developer                  | Deleted after merge                  |
| `hotfix/*`  | Emergency fixes targeting `stable`                                  | Developer                  | Deleted after merge; backmerged down |
| `chore/*`   | Non-functional changes (docs, CI, deps)                             | Developer                  | Deleted after merge                  |

### Invariant

Every commit on every branch must compile. No exceptions.

## CI Gates

### Local Hooks (developer machine)

| Hook       | Command                                                       | Equivalent tier |
| ---------- | ------------------------------------------------------------- | --------------- |
| pre-commit | `cargo xtask pre-commit` (fmt-check + clippy + release build) | `main` gate     |
| pre-push   | `cargo xtask prepush` (nextest + llvm-cov coverage)           | `next` gate     |

### Remote CI (GitHub Actions)

CI mirrors the local hooks as server-side enforcement. Force-push or `--no-verify` cannot bypass remote gates.

| Gate                                           | Local hook | `main` CI | `next` CI | `stable` CI |
| ---------------------------------------------- | ---------- | --------- | --------- | ----------- |
| fmt + clippy + check                           | pre-commit | x         | x         | x           |
| nextest + coverage                             | pre-push   |           | x         | x           |
| snapshot check (`cargo insta test --check`)    | pre-push   |           | x         | x           |
| audit + deny + machete                         |            |           | x         | x           |
| bench run + regression report (warn, not gate) |            |           | x         | x           |
| e2e suite (Phase 2)                            |            |           |           | x           |
| geiger (unsafe audit)                          |            |           |           | x           |
| Manual review                                  |            |           |           | x           |

### Snapshot Testing

Protocol wire format and CLI output are snapshot-tested via `insta`. Snapshots catch accidental breaking changes to the JSON protocol and CLI output format before they reach `stable` (and Maestro).

- `cargo insta test --check` fails if any `.snap` file drifts
- Review new snapshots locally with `cargo insta review`
- Snapshots live alongside their test files in `*.snap` files

### Benchmarking

`minibox-bench` runs codec and adapter micro-benchmarks. Results are saved to `bench/results/bench.jsonl` (append-only history) and `bench/results/latest.json` (current snapshot).

- Runs on `next` and `stable` pushes
- Compares against the `stable` baseline in `latest.json`
- **Does not gate** — regressions are reported as warnings, not failures (hardware variance on self-hosted runner makes hard gating unreliable)
- `cargo xtask bench` is the canonical command

### CI Phases (self-hosted runner on jobrien-vm)

**Phase 1 (immediate):** Non-compile gates only. Fast, low-resource, no `target/` needed.

- `cargo audit`
- `cargo deny`
- `cargo machete`
- `cargo geiger`

**Phase 2 (future):** Compile + test inside minibox containers on the runner.

- `cargo xtask test-unit`
- `cargo xtask test-e2e-suite`
- Shared `CARGO_TARGET_DIR` mount for incremental builds
- minibox runs its own CI pipeline (dogfooding)

## Workflow Files

### ci.yml (Quality)

Triggers: push to `main`, `next`, `stable`, `feature/*`, `hotfix/*`, `chore/*` + PRs to `main`, `next`.

Jobs:

- **compile-check** (all branches): `cargo check --workspace && cargo fmt --all --check && cargo clippy ...`
- **test-unit** (`next`, `stable` only): `cargo xtask test-unit` on self-hosted runner
- **audit** (`next`, `stable`): `cargo audit`
- **deny** (`next`, `stable`): `cargo deny`
- **machete** (`next`, `stable`): `cargo machete`
- **geiger** (`stable` only): `cargo geiger`

### phased-deployment.yml

Triggers: push to `main`, `next`, `stable`.

Jobs:

- **auto-promote-main**: On green `main` CI, fast-forward merge `main` into `next`. GitHub Actions bot only.
- **manual-promote-next**: `workflow_dispatch` to promote `next` → `stable`. Requires manual trigger.
- **hotfix-backmerge**: When a `[hotfix]` commit lands on `stable`, auto-backmerge `stable → next → main`.

### release.yml

Trigger: `v*` tag on `stable`.

Jobs:

- Cross-compile musl binaries (x86_64 + aarch64)
- Create GitHub Release with artifacts

### nightly.yml

Trigger: cron (daily 02:00 UTC).

Jobs:

- `cargo geiger` unsafe audit (informational, does not block)

## Remotes

| Remote   | URL                                           | Role                      |
| -------- | --------------------------------------------- | ------------------------- |
| `origin` | `git@github.com:89jobrien/minibox.git`        | Primary (swap from Gitea) |
| `gitea`  | `ssh://git@100.105.75.7:2222/joe/minibox.git` | Mirror (when VM is back)  |

**Action required:** Swap `origin` to point to GitHub. Rename current `origin` to `gitea`.

## Cleanup Automation

### Merged branches

- GitHub repo setting: auto-delete head branch on PR merge
- Local: `git fetch --prune` removes stale remote-tracking refs

### Stale worktrees

- `.worktrees/` dirs from completed feature branches
- post-merge hook or periodic: `git worktree prune`

### Build artifacts on runner

- Post-job step: `cargo xtask clean-artifacts`
- Nightly cron: full `target/` wipe on runner (cold builds are rare — nightly at worst)
- Pre-e2e step (Phase 2): `cargo xtask nuke-test-state` (kill orphans, unmount overlays, clean cgroups/tmp)

### Shared target directory

- Runner: `CARGO_TARGET_DIR=/var/lib/minibox/cache/target/`
- Local dev: `CARGO_TARGET_DIR=~/.mbx/cache/target/`
- Worktrees share the same `target/` — no per-worktree duplication
- Phase 2: minibox containers mount the shared dir as a volume

## Auto-Promotion Logic

### main → next (automatic)

On every push to `main`, after all `main`-tier CI gates pass:

1. GitHub Actions checks out `next`
2. Attempts `git merge --ff-only origin/main`
3. Pushes `next`
4. If ff-only fails (diverged), opens an issue or alerts — does NOT force-push

### next → stable (manual)

Via `workflow_dispatch`:

1. Developer triggers promotion
2. CI runs full `stable`-tier gates on `next` HEAD
3. If green, `git merge --ff-only origin/next` into `stable`
4. Push `stable`

### Hotfix backmerge (automatic)

When a commit with `[hotfix]` lands on `stable`:

1. Merge `stable` into `next`
2. Merge `next` into `main`
3. If merge conflicts, open an issue instead of failing silently

## Maestro Integration (future)

Maestro will consume minibox as a **Cargo library** (`mbx` crate). The `stable` branch represents API stability:

- Semver on `stable` tags: breaking changes to `mbx` public API require a minor version bump (pre-1.0) or major bump (post-1.0)
- Maestro's `maestro-cli` Cargo.toml would pin to a minibox release tag or git rev on `stable`
- No binary distribution needed for Maestro integration — direct crate dependency

## Dogfooding: minibox as CI runner (Phase 2)

The long-term goal is minibox running its own CI pipeline:

- Pull a Rust toolchain image (`alpine` + `rustup` or a custom `ghcr.io` image)
- Mount shared `CARGO_TARGET_DIR` as a volume
- Run `cargo xtask` commands inside the container
- Report results back to GitHub via status checks or webhook

CI workflow files remain **thin wrappers** — all real logic lives in `cargo xtask` commands that work identically whether GitHub Actions or minibox invokes them.

## Migration Checklist

1. Swap remotes: rename `origin` → `gitea`, rename `github` → `origin`
2. Create `next` and `stable` branches from current `main` HEAD
3. Enable "auto-delete head branches" in GitHub repo settings
4. Write `phased-deployment.yml` workflow
5. Update `ci.yml` with branch-conditional job gates
6. Add `CARGO_TARGET_DIR` to `.envrc` / mise config for local dev
7. Add post-job cleanup step to CI for self-hosted runner
8. Update CLAUDE.md and HANDOFF.md to document new branching model
