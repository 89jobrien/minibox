# Gitea-Primary CI Pipeline Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Gitea the primary remote, mirror to GitHub, and run a full 5-job CI pipeline (unit → property → integration+e2e → bench) via Gitea Actions on jobrien-vm.

**Architecture:** Rename git remotes so `origin` points to Gitea. Gitea push-mirrors to GitHub automatically. A `.gitea/workflows/ci.yml` workflow drives `act_runner` (already running as a system service on the VPS) through the staged pipeline. Property-based tests live in `crates/minibox-lib/tests/proptest_suite.rs` (public API) and an inline proptest block in `layer.rs` (internal path validation).

**Tech Stack:** Gitea Actions (act_runner), proptest 1.x, existing xtask commands (`test-unit`, `test-e2e-suite`, `bench`, `nuke-test-state`), mise for toolchain activation on the runner.

---

## File Map

| Action | File | Purpose |
|--------|------|---------|
| Modify | `.github/workflows/ci.yml` | Remove `linux` self-hosted job |
| Create | `.gitea/workflows/ci.yml` | Full 5-job VPS pipeline |
| Modify | `crates/minibox-lib/Cargo.toml` | Add `proptest` dev-dependency |
| Create | `crates/minibox-lib/tests/proptest_suite.rs` | Protocol roundtrip proptests |
| Modify | `crates/minibox-lib/src/image/layer.rs` | Add inline proptest block for path validation |

---

## Task 1: Rename Git Remotes

**Files:** none (git config only)

- [ ] **Step 1: Rename remotes**

```bash
git remote rename origin github
git remote rename gitea origin
```

- [ ] **Step 2: Verify**

```bash
git remote -v
```

Expected:
```
github  git@github.com:89jobrien/minibox.git (fetch)
github  git@github.com:89jobrien/minibox.git (push)
origin  http://100.105.75.7:3000/joe/minibox.git (fetch)
origin  http://100.105.75.7:3000/joe/minibox.git (push)
```

---

## Task 2: Remove Linux Job from GitHub Actions

**Files:**
- Modify: `.github/workflows/ci.yml`

- [ ] **Step 1: Remove the `linux` job block from `.github/workflows/ci.yml`**

The file should contain only the `macos` job after this edit. Final state:

```yaml
name: CI

on:
  push:
    branches: [main]
  pull_request:
    branches: [main]

env:
  CARGO_TERM_COLOR: always
  RUST_BACKTRACE: 1

jobs:
  macos:
    name: macOS
    runs-on: macos-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - name: fmt check
        run: cargo fmt --all --check
      - name: clippy
        run: >
          cargo clippy
          -p minibox-lib -p minibox-macros -p minibox-cli
          -p daemonbox -p macbox -p miniboxd
          -- -D warnings
      - name: unit tests
        run: cargo xtask test-unit
```

- [ ] **Step 2: Commit**

```bash
git add .github/workflows/ci.yml
git commit -m "ci: remove linux self-hosted job from GitHub Actions (moves to Gitea)"
```

---

## Task 3: Add Proptest Dependency

**Files:**
- Modify: `crates/minibox-lib/Cargo.toml`

- [ ] **Step 1: Add proptest to dev-dependencies in `crates/minibox-lib/Cargo.toml`**

Find the `[dev-dependencies]` section and add:

```toml
[dev-dependencies]
criterion = { workspace = true }
proptest = "1"
```

- [ ] **Step 2: Verify it resolves**

```bash
cargo check -p minibox-lib
```

Expected: `Finished` with no errors.

---

## Task 4: Write Protocol Roundtrip Proptests

**Files:**
- Create: `crates/minibox-lib/tests/proptest_suite.rs`

These are integration tests (in `tests/`) so they can only access the public API.

- [ ] **Step 1: Create `crates/minibox-lib/tests/proptest_suite.rs`**

```rust
//! Property-based tests for minibox-lib's public API.
//!
//! Invariants tested:
//! - Protocol encode→decode roundtrip is lossless (re-encode produces same bytes)
//! - Arbitrary valid DaemonRequest / DaemonResponse survive the round-trip
//! - image/tag strings of any content survive protocol encode→decode
//!
//! Note: `ImageRef` is not a public type; image ref string safety is covered
//! by the `DaemonRequest::Pull` roundtrip which exercises arbitrary image/tag
//! strings through the full protocol layer.

use minibox_lib::protocol::{
    ContainerInfo, DaemonRequest, DaemonResponse, OutputStreamKind,
    decode_request, decode_response, encode_request, encode_response,
};
use proptest::prelude::*;
use proptest::option;

// ---------------------------------------------------------------------------
// Strategies
// ---------------------------------------------------------------------------

fn arb_request() -> impl Strategy<Value = DaemonRequest> {
    prop_oneof![
        (
            any::<String>(),
            option::of(any::<String>()),
            prop::collection::vec(any::<String>(), 0..8),
            option::of(any::<u64>()),
            option::of(1u64..=10000u64),
            any::<bool>(),
        )
            .prop_map(|(image, tag, command, memory_limit_bytes, cpu_weight, ephemeral)| {
                DaemonRequest::Run {
                    image,
                    tag,
                    command,
                    memory_limit_bytes,
                    cpu_weight,
                    ephemeral,
                }
            }),
        any::<String>().prop_map(|id| DaemonRequest::Stop { id }),
        any::<String>().prop_map(|id| DaemonRequest::Remove { id }),
        Just(DaemonRequest::List),
        // Exercises arbitrary image ref strings (the spec's ImageRef invariant —
        // ImageRef is not public, but Pull carries the same data through the wire).
        (any::<String>(), option::of(any::<String>()))
            .prop_map(|(image, tag)| DaemonRequest::Pull { image, tag }),
    ]
}

fn arb_stream_kind() -> impl Strategy<Value = OutputStreamKind> {
    prop_oneof![Just(OutputStreamKind::Stdout), Just(OutputStreamKind::Stderr)]
}

fn arb_container_info() -> impl Strategy<Value = ContainerInfo> {
    (
        any::<String>(),
        any::<String>(),
        any::<String>(),
        any::<String>(),
        any::<String>(),
        option::of(any::<u32>()),
    )
        .prop_map(|(id, image, command, state, created_at, pid)| ContainerInfo {
            id,
            image,
            command,
            state,
            created_at,
            pid,
        })
}

fn arb_response() -> impl Strategy<Value = DaemonResponse> {
    prop_oneof![
        any::<String>().prop_map(|id| DaemonResponse::ContainerCreated { id }),
        any::<String>().prop_map(|message| DaemonResponse::Success { message }),
        prop::collection::vec(arb_container_info(), 0..8)
            .prop_map(|containers| DaemonResponse::ContainerList { containers }),
        any::<String>().prop_map(|message| DaemonResponse::Error { message }),
        (arb_stream_kind(), any::<String>())
            .prop_map(|(stream, data)| DaemonResponse::ContainerOutput { stream, data }),
        any::<i32>().prop_map(|exit_code| DaemonResponse::ContainerStopped { exit_code }),
    ]
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

proptest! {
    /// Encoding a DaemonRequest and decoding it back, then re-encoding,
    /// must produce identical bytes — guarantees no data is lost in transit.
    #[test]
    fn request_encode_decode_roundtrip(req in arb_request()) {
        let encoded = encode_request(&req).expect("encode must succeed");
        let decoded = decode_request(&encoded).expect("decode must succeed");
        let re_encoded = encode_request(&decoded).expect("re-encode must succeed");
        prop_assert_eq!(encoded, re_encoded);
    }

    /// Same invariant for DaemonResponse (all six variants including ContainerList).
    #[test]
    fn response_encode_decode_roundtrip(resp in arb_response()) {
        let encoded = encode_response(&resp).expect("encode must succeed");
        let decoded = decode_response(&encoded).expect("decode must succeed");
        let re_encoded = encode_response(&decoded).expect("re-encode must succeed");
        prop_assert_eq!(encoded, re_encoded);
    }
}
```

- [ ] **Step 2: Run the tests**

```bash
cargo test -p minibox-lib --test proptest_suite
```

Expected: both proptest targets pass (100 cases each by default).

---

## Task 5: Add Path Validation Proptests to layer.rs

`validate_tar_entry_path` is private, so these live in an inline `#[cfg(test)]` block inside `layer.rs`.

**Files:**
- Modify: `crates/minibox-lib/src/image/layer.rs`

- [ ] **Step 1: Add proptest to the existing `#[cfg(test)]` block at the bottom of `layer.rs`**

Add a new `proptest_tests` submodule inside the existing `#[cfg(test)] mod tests { ... }` block:

```rust
    #[cfg(test)]
    mod proptest_tests {
        use super::*;
        use proptest::prelude::*;
        use std::path::{Path, PathBuf};
        use tempfile::TempDir;

        /// Any path containing a `..` component must always be rejected.
        proptest! {
            #[test]
            fn dotdot_paths_always_rejected(
                prefix in "[a-z]{1,8}",
                suffix in "[a-z]{1,8}",
            ) {
                let dir = TempDir::new().unwrap();
                let dest = dir.path();
                let evil = PathBuf::from(format!("{prefix}/../../{suffix}"));
                let result = validate_tar_entry_path(&evil, dest);
                prop_assert!(result.is_err(), "expected rejection for path {:?}", evil);
            }

            /// Valid relative paths (no `..`, no absolute) must never panic —
            /// they may succeed or return a clean error, never panic.
            #[test]
            fn safe_relative_paths_do_not_panic(
                component in "[a-zA-Z0-9_-]{1,16}",
            ) {
                let dir = TempDir::new().unwrap();
                let dest = dir.path();
                let path = PathBuf::from(&component);
                // Must not panic — result can be Ok or Err
                let _ = validate_tar_entry_path(&path, dest);
            }
        }
    }
```

- [ ] **Step 2: Run the tests**

```bash
cargo test -p minibox-lib image::layer::tests::proptest_tests
```

Expected: both proptest targets pass.

- [ ] **Step 3: Commit tasks 3–5**

```bash
git add crates/minibox-lib/Cargo.toml \
        crates/minibox-lib/tests/proptest_suite.rs \
        crates/minibox-lib/src/image/layer.rs
git commit -m "test(minibox-lib): add property-based tests for protocol roundtrip and path validation"
```

---

## Task 6: Check act_runner Labels

The Gitea workflow's `runs-on` must match the labels registered by `act_runner`. Do this before writing the workflow.

**Files:** none

- [ ] **Step 1: SSH to jobrien-vm and check registered labels**

```bash
sshpass -p "$(op item get jobrien-vm --account=my.1password.com --fields password --reveal)" \
  ssh -o IdentitiesOnly=yes -o IdentityAgent=none -o PreferredAuthentications=password \
  dev@100.105.75.7 \
  "sudo cat /var/lib/gitea/act_runner.yaml | grep -A10 'labels\|runner'"
```

Note the label values — they go in `runs-on` in Task 7.
If empty or not set, `ubuntu-latest` is the act_runner default label.

---

## Task 7: Create Gitea Actions Workflow

**Files:**
- Create: `.gitea/workflows/ci.yml`

Use the `runs-on` label confirmed in Task 6 (default: `ubuntu-latest`).

- [ ] **Step 1: Create `.gitea/workflows/ci.yml`**

```yaml
name: CI

on:
  push:
    branches: [main]
  pull_request:
    branches: [main]

env:
  CARGO_TERM_COLOR: always
  RUST_BACKTRACE: 1

jobs:
  unit:
    name: Unit Tests
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: unit tests
        run: ~/.local/bin/mise exec -- cargo xtask test-unit

  property:
    name: Property Tests
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: property tests
        run: ~/.local/bin/mise exec -- cargo test -p minibox-lib --test proptest_suite

  integration:
    name: Integration Tests
    runs-on: ubuntu-latest
    needs: [unit, property]
    steps:
      - uses: actions/checkout@v4
      - name: cgroup tests
        run: sudo -E ~/.cargo/bin/cargo test -p miniboxd --test cgroup_tests -- --test-threads=1 --nocapture
      - name: integration tests
        run: sudo -E ~/.cargo/bin/cargo test -p miniboxd --test integration_tests -- --test-threads=1 --ignored --nocapture
      - name: cleanup
        if: always()
        run: sudo -E ~/.local/bin/mise exec -- cargo xtask nuke-test-state

  e2e:
    name: E2E Tests
    runs-on: ubuntu-latest
    needs: [unit, property]
    steps:
      - uses: actions/checkout@v4
      - name: daemon + CLI e2e
        # xtask test-e2e-suite internally calls `sudo -E` for the test binary
        run: ~/.local/bin/mise exec -- cargo xtask test-e2e-suite
      - name: cleanup
        if: always()
        run: sudo -E ~/.local/bin/mise exec -- cargo xtask nuke-test-state

  bench:
    name: Benchmarks
    runs-on: ubuntu-latest
    needs: [integration, e2e]
    steps:
      - uses: actions/checkout@v4
      - name: bench
        run: sudo -E ~/.local/bin/mise exec -- cargo xtask bench
```

- [ ] **Step 2: Commit**

```bash
git add .gitea/workflows/ci.yml
git commit -m "ci(gitea): add full VPS pipeline — unit+property → integration+e2e → bench"
```

---

## Task 8: Configure Gitea Push Mirror (Manual)

This step is done in the Gitea web UI — cannot be scripted.

- [ ] **Step 1: Open Gitea repo settings**

Navigate to: `http://100.105.75.7:3000/joe/minibox/settings`

- [ ] **Step 2: Add push mirror**

Settings → **Mirror** → **Push Mirrors** → Add:

| Field | Value |
|-------|-------|
| Git Repository URL | `https://github.com/89jobrien/minibox.git` |
| Force Push | ✓ (enabled) |
| Username | your GitHub username |
| Password/Token | GitHub personal access token with `repo` scope |

Click **Add Push Mirror**.

- [ ] **Step 3: Trigger a manual sync** to verify it works before the next task.

---

## Task 9: Push to Gitea and Verify

- [ ] **Step 1: Push everything to the new origin (Gitea)**

```bash
git push origin main
```

- [ ] **Step 2: Verify Gitea Actions triggered**

Open `http://100.105.75.7:3000/joe/minibox/actions` — the workflow run should appear.

- [ ] **Step 3: Verify GitHub mirror received the push**

```bash
git fetch github
git log github/main --oneline -3
```

Expected: same commits as local `main`.

- [ ] **Step 4: Verify GitHub Actions still passes (macOS gate)**

```bash
gh run list --limit 3
```

Expected: the macOS job triggered and is passing (or will pass shortly).

---

## Task 10: Update `just push` and CLAUDE.md notes

The `just push` recipe currently calls `git push {{args}}` which will push to `origin` (now Gitea). The mirror handles GitHub. No Justfile change needed.

- [ ] **Step 1: Smoke-test the full flow**

```bash
just commit "chore: verify gitea pipeline"
just push
```

Verify: Gitea run triggers, GitHub mirror updates, GitHub Actions macOS job triggers.

- [ ] **Step 2: Final check — no orphaned GitHub runner jobs**

```bash
gh run list --limit 5
```

Expected: only `CI` (macOS job) appears — no `Linux`, `Security Scanning`, or `Integration Tests` jobs.
