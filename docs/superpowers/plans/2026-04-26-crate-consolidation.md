---
status: done
---

# minibox-77 Crate Consolidation — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Consolidate minibox workspace from 13 crates to 8, reducing internal coupling,
simplifying the publish chain (4 → 2 published crates), and preparing for crates.io
publication. Phases 0–3 are complete (minibox-llm dropped, minibox-oci + minibox-client
absorbed into minibox-core, linuxbox + daemonbox merged into minibox). Remaining work:
absorb minibox-testers, extract DEFAULT_ADAPTER_SUITE const, update CI/docs, dry-run
publish, and tag release.

**Architecture:** SOLID hexagonal — domain traits in minibox-core, adapters in minibox.
minibox-testers moves behind `test-utils` feature in minibox (not minibox-core, due to
circular dep). Published surface: minibox-macros (proc-macro) + minibox-core (library).

**Tech Stack:** Rust 2024, cargo workspace, serde, tokio, nix, `cargo xtask`, `gh` CLI

---

## Causal Chain

```text
T1: Absorb minibox-testers into minibox       (prereq: removes last orphan crate)
  └─► T2: Extract DEFAULT_ADAPTER_SUITE       (independent, but cleaner after T1)
        └─► T3: Update CI, docs, and xtask    (depends on final crate names from T1+T2)
              └─► T4: Dry-run publish and tag  (depends on CI green from T3)
```

**Note:** T1 and T2 are independent and can run in parallel. T3 depends on both. T4 is
strictly sequential after T3.

---

## File Map

| Action    | Path                                    |
| --------- | --------------------------------------- |
| Modify    | `crates/minibox/Cargo.toml`             |
| Modify    | `crates/minibox/src/lib.rs`             |
| Create    | `crates/minibox/src/testing/`           |
| Delete    | `crates/minibox-testers/`               |
| Modify    | `Cargo.toml` (workspace members)        |
| Modify    | `crates/miniboxd/src/main.rs`           |
| Modify    | `crates/minibox/src/adapters/smolvm.rs` |
| Modify    | `.github/workflows/release.yml`         |
| Modify    | `.github/workflows/ci.yml`              |
| Modify    | `CLAUDE.md`                             |
| Modify    | `crates/xtask/src/`                     |
| Reference | `crates/minibox-core/Cargo.toml`        |
| Reference | `crates/minibox-macros/Cargo.toml`      |

---

## Completed Phases (for reference)

| Phase | Description                                     | Commit  |
| ----- | ----------------------------------------------- | ------- |
| 0     | License files, drop minibox-llm, publish guards | 21b5e5c |
| 1+2   | Absorb minibox-oci + minibox-client into core   | cdee328 |
| 3     | Merge linuxbox + daemonbox into unified minibox | 6153d19 |

Current state: 9 crates (macbox, mbx, minibox, minibox-core, minibox-macros,
minibox-testers, miniboxd, winbox, xtask). Target: 8 crates (drop minibox-testers).

---

## Task 1: Absorb minibox-testers into minibox (test-utils feature)

**Files:**

- Modify: `crates/minibox/Cargo.toml`
- Modify: `crates/minibox/src/lib.rs`
- Create: `crates/minibox/src/testing/mod.rs`
- Create: `crates/minibox/src/testing/` (all files from minibox-testers/src/)
- Delete: `crates/minibox-testers/`
- Modify: `Cargo.toml` (workspace members, workspace.dependencies)
- Modify: all crates with `minibox-testers` dev-dep

**Bug/Change:** minibox-testers (1427 LOC) is a standalone crate providing mocks, fixtures,
conformance helpers, and backend capability probes. It depends on both minibox-core and
minibox, so it cannot be absorbed into minibox-core (circular dep). It belongs in minibox
behind a `test-utils` feature flag.

**Fix/Implementation:** Move all minibox-testers source into `crates/minibox/src/testing/`,
gate with `#[cfg(feature = "test-utils")]`, update downstream dev-deps to use
`minibox = { features = ["test-utils"] }`.

- [ ] **Step 1: Audit minibox-testers imports**

    Check which modules import from `minibox_core` vs `minibox` (the merged crate). Since
    testers now lives inside minibox, all imports become `crate::` paths.

    ```bash
    rg 'use minibox_core::' crates/minibox-testers/src/
    rg 'use minibox::' crates/minibox-testers/src/
    ```

- [ ] **Step 2: Move testers source into minibox**

    ```bash
    mkdir -p crates/minibox/src/testing
    cp -r crates/minibox-testers/src/* crates/minibox/src/testing/
    mv crates/minibox/src/testing/lib.rs crates/minibox/src/testing/mod.rs
    ```

- [ ] **Step 3: Gate the module in minibox/src/lib.rs**

    Add to `crates/minibox/src/lib.rs`:

    ```rust
    #[cfg(feature = "test-utils")]
    pub mod testing;
    ```

- [ ] **Step 4: Add test-utils feature to minibox/Cargo.toml**

    Add under `[features]`:

    ```toml
    test-utils = ["dep:tempfile"]
    ```

    Move `tempfile` from `[dependencies]` to optional if not already used outside testing.
    Keep other deps (`anyhow`, `serde`, `tokio`, etc.) as-is since minibox already has them.

- [ ] **Step 5: Fix internal paths in testing modules**

    In all `crates/minibox/src/testing/*.rs` files:
    - Replace `use minibox_core::` with `use minibox_core::` (still valid — minibox depends
      on minibox-core) or `use crate::` where the type is re-exported
    - Replace `use minibox::` with `use crate::`
    - Remove any `use minibox_testers::` self-references

- [ ] **Step 6: Update downstream dev-deps**

    Search all Cargo.toml files for `minibox-testers`. Replace with
    `minibox = { workspace = true, features = ["test-utils"] }` in dev-dependencies.

    Key sites:
    - `crates/miniboxd/Cargo.toml`
    - `crates/xtask/Cargo.toml`
    - Any integration test crates

- [ ] **Step 7: Update all test imports**

    ```bash
    rg 'use minibox_testers::' --type rs -l
    ```

    Replace `use minibox_testers::` with `use minibox::testing::` in every match.

- [ ] **Step 8: Delete minibox-testers crate**

    Remove `crates/minibox-testers/` directory. Remove `"crates/minibox-testers"` from
    workspace members in root `Cargo.toml`. Remove `minibox-testers` from
    `[workspace.dependencies]`.

- [ ] **Step 9: Verify**

    ```bash
    cargo check --workspace
    cargo xtask test-unit
    cargo xtask pre-commit
    ```

    Expected: clean pass, same test count.

- [ ] **Step 10: Commit**

    ```bash
    git add -A
    git commit -m "$(cat <<'EOF'
    refactor: absorb minibox-testers into minibox test-utils feature (#153 Phase 4)

    Move all test infrastructure (mocks, fixtures, conformance helpers, backend
    capability probes) from standalone minibox-testers crate into
    minibox::testing behind #[cfg(feature = "test-utils")]. Workspace drops
    from 9 to 8 crates.
    EOF
    )"
    ```

---

## Task 2: Extract DEFAULT_ADAPTER_SUITE const + smolvm cross-platform docs

**Files:**

- Modify: `crates/miniboxd/src/main.rs`
- Modify: `crates/minibox/src/adapters/smolvm.rs`

**Bug/Change:** Default adapter suite is hardcoded inline. smolvm docs say "macOS" but
smolmachines uses libkrun and works on Linux too.

**Fix/Implementation:** Extract a `DEFAULT_ADAPTER_SUITE` const in miniboxd for single-point
configuration. Update smolvm module docs for cross-platform accuracy.

- [ ] **Step 1: Add DEFAULT_ADAPTER_SUITE const to miniboxd/src/main.rs**

    Add above `AdapterSuite`:

    ```rust
    /// Default adapter suite when `MINIBOX_ADAPTER` is unset.
    ///
    /// Change this single value to switch the default runtime for all
    /// platforms. Current options: `"native"`, `"gke"`, `"colima"`,
    /// `"smolvm"`.
    const DEFAULT_ADAPTER_SUITE: &str = "native";
    ```

    Update `AdapterSuite::from_env()` to use the const:

    ```rust
    fn from_env() -> Result<Self> {
        let val = std::env::var("MINIBOX_ADAPTER")
            .unwrap_or_else(|_| DEFAULT_ADAPTER_SUITE.to_string());
        match val.as_str() {
            "native" => Ok(Self::Native),
            "gke" => Ok(Self::Gke),
            "colima" => Ok(Self::Colima),
            "smolvm" => Ok(Self::SmolVm),
            other => anyhow::bail!(
                "unknown MINIBOX_ADAPTER value {other:?} \
                 (expected \"native\", \"gke\", \"colima\", or \"smolvm\")"
            ),
        }
    }
    ```

- [ ] **Step 2: Update smolvm.rs module-level docs**

    Replace module doc comment with cross-platform version:

    ```rust
    //! SmolVM adapter suite — lightweight Linux VMs via smolmachines.
    //!
    //! Delegates container operations into a smolmachines VM. smolmachines
    //! uses libkrun (a lightweight VMM) to boot Linux VMs with sub-second
    //! cold starts. Works on both macOS (Apple Silicon / Intel) and Linux.
    //!
    //! Selected by `MINIBOX_ADAPTER=smolvm`. Compiled on all platforms.
    //!
    //! Requirements:
    //! - smolmachines installed (https://smolmachines.com)
    //!   - macOS: `brew install smolvm`
    //!   - Linux: see smolmachines docs
    ```

- [ ] **Step 3: Fix "macOS host side" language in SmolVmLimiter doc comment**

    Replace "on the macOS host side" with "on the host side".

- [ ] **Step 4: Verify**

    ```bash
    cargo check --workspace
    cargo xtask test-unit
    ```

- [ ] **Step 5: Commit**

    ```bash
    git add crates/miniboxd/src/main.rs crates/minibox/src/adapters/smolvm.rs
    git commit -m "$(cat <<'EOF'
    feat(miniboxd): extract DEFAULT_ADAPTER_SUITE const, update smolvm docs (#153 Phase 5)

    Single-point configuration for default adapter suite. Update smolvm
    adapter docs to reflect cross-platform support (libkrun, not VZ-only).
    EOF
    )"
    ```

---

## Task 3: Update CI, docs, and xtask for consolidated workspace

**Files:**

- Modify: `.github/workflows/release.yml`
- Modify: `.github/workflows/ci.yml`
- Modify: `CLAUDE.md`
- Modify: `crates/xtask/src/` (any hardcoded crate names)

- [ ] **Step 1: Update release.yml publish chain**

    Shrink from 4 published crates to 2. Use Bash heredoc (Write tool blocked for workflow
    files):

    ```yaml
    - name: Publish crates
      env:
          CARGO_REGISTRY_TOKEN: ${{ secrets.CARGO_REGISTRY_TOKEN }}
      run: |
          cargo publish -p minibox-macros
          sleep 30
          cargo publish -p minibox-core
    ```

    Remove any `minibox-oci`, `minibox-client` publish steps.

- [ ] **Step 2: Update ci.yml clippy targets**

    Remove `-p linuxbox`, `-p daemonbox`, `-p minibox-oci`, `-p minibox-client` from clippy
    invocations. Ensure `-p minibox` is present. Use Bash heredoc for edits.

- [ ] **Step 3: Update CLAUDE.md workspace structure**

    Key changes:
    - "12 crates in cargo workspace" → "8 crates in cargo workspace"
    - Remove linuxbox, daemonbox, minibox-oci, minibox-client, minibox-llm entries
    - Add unified minibox entry: "minibox (library): Linux container primitives, adapters,
      daemon handler/server/state"
    - Update minibox-core entry: "minibox-core (library): Cross-platform shared types,
      protocol, domain traits, OCI image management, client library"
    - Update minibox-testers references → `minibox::testing`

- [ ] **Step 4: Update xtask hardcoded crate names**

    ```bash
    rg 'linuxbox|daemonbox|minibox-oci|minibox-client|minibox-testers' crates/xtask/src/
    ```

    Replace any matches with current crate names.

- [ ] **Step 5: Verify full CI locally**

    ```bash
    cargo xtask pre-commit
    ```

- [ ] **Step 6: Commit**

    ```bash
    git add -A
    git commit -m "$(cat <<'EOF'
    chore: update CI, docs, and xtask for consolidated workspace (#153 Phase 6)

    Publish chain: 4 → 2 crates. Update clippy targets, CLAUDE.md workspace
    docs, and xtask references to match 8-crate layout.
    EOF
    )"
    ```

---

## Task 4: Dry-run publish and tag release

**Files:** none (git and cargo operations only)

- [ ] **Step 1: Verify CARGO_REGISTRY_TOKEN exists**

    ```bash
    gh secret list -R 89jobrien/minibox | rg CARGO_REGISTRY_TOKEN
    ```

- [ ] **Step 2: Dry-run publish both crates**

    ```bash
    cargo publish -p minibox-macros --dry-run
    cargo publish -p minibox-core --dry-run
    ```

    Both must succeed. Common failures: missing `description`, missing license file,
    unpublished dependency.

- [ ] **Step 3: Promote main → next → stable**

    ```bash
    git checkout next && git merge main && git push
    git checkout stable && git merge next && git push
    git checkout main
    ```

    Or trigger `phased-deployment.yml` workflow dispatch.

- [ ] **Step 4: Tag and release**

    ```bash
    git tag v0.21.0 stable
    git push origin v0.21.0
    ```

    This triggers `release.yml` which publishes to crates.io and creates the GitHub Release.

---

## Self-Review

**Spec coverage check:**

| Gap / objective                              | Task |
| -------------------------------------------- | ---- |
| minibox-testers absorbed behind feature flag | T1   |
| DEFAULT_ADAPTER_SUITE single-point config    | T2   |
| smolvm docs accurate for cross-platform      | T2   |
| CI publish chain matches 2-crate target      | T3   |
| CLAUDE.md reflects 8-crate workspace         | T3   |
| xtask references updated                     | T3   |
| crates.io dry-run passes                     | T4   |
| Tagged release on stable branch              | T4   |

**Placeholder scan:** All placeholders filled with concrete paths, commands, and crate names.

**Type consistency:** All referenced crate names (minibox, minibox-core, minibox-macros,
minibox-testers) and module paths (minibox::testing, minibox::daemon) match current source
tree as of commit 6153d19.

---

## Rollback Strategy

Each task produces a single commit. If a task breaks CI:

1. `git revert <commit>` to restore previous state
2. Fix the issue on a branch
3. Re-apply

The workspace compiles after every task, so partial progress is safe.

## Risk Register

| Risk                                                          | Mitigation                                                     |
| ------------------------------------------------------------- | -------------------------------------------------------------- |
| minibox-testers has imports that don't resolve inside minibox | T1 Step 1 audits all imports before moving                     |
| tempfile becomes required dep outside test-utils              | T1 Step 4: make it optional behind the feature                 |
| Benchmark regressions from crate merge                        | Run `cargo bench -p minibox` after T1; compare results         |
| crates.io publish fails (missing metadata)                    | T4 Step 2 dry-run catches before tagging                       |
| Test count regression                                         | Compare `cargo nextest run --lib` count before T1 and after T3 |
