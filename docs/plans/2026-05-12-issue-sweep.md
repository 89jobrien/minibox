# Plan: Issue Sweep (2026-05-12)

## Goal

Close already-resolved issues and implement the remaining open fixes across
security, correctness, and cleanup categories.

## Triage: Already Resolved

These issues describe fixes that are already present in the current codebase.
Close with a comment citing the evidence.

| Issue | Title | Evidence |
|-------|-------|----------|
| #310 | bound daemon request frames | `bounded_read_line` at `server.rs:177` uses `fill_buf()` + size check before buffering |
| #311 | redact sensitive daemon request logs | `server.rs:294` logs only `request.type_tag()`; `server.rs:304` logs byte length only |
| #330 | exclude container_id from digest | `DigestProjection` at `execution_manifest.rs:185` already excludes `container_id` |
| #331 | populate layer_digests with content digests | `handler.rs:882-884` extracts digest from `file_name` via `replacen('_', ":", 1)` |
| #332 | update ContainerRecord with manifest info | `state.rs:568` has `set_manifest_info`; called at `handler.rs:993` and `handler.rs:1094` |
| #333 | deserialize manifest as typed struct | `handler.rs:3228` deserializes as `ExecutionManifest`, re-serializes to `Value` |
| #334 | replace expect() with documented invariant | `execution_manifest.rs:146-149` has SAFETY comment explaining infallibility |
| #335 item 2 | add PartialEq to ExecutionPolicy | `execution_policy.rs:22` already derives `PartialEq` |

## Architecture

Remaining actionable issues fall into two independent tracks:

- **Track A** (security): #319 — aggregate image pull size limit in
  `minibox-core/src/image/registry.rs` and `minibox/src/adapters/ghcr.rs`
- **Track B** (cleanup): #335 item 1 — remove stale `#[allow(dead_code)]`
  from `PreparedRun`; #322 — unused import in `conformance_snapshot.rs`;
  #323 — dead code in `miniboxd/tests/protocol_e2e_tests.rs`

Tracks A and B are fully independent (different crates, no shared files).

## Tech Stack

- Rust 2024, serde, reqwest, tokio, anyhow
- No new dependencies

## Tasks

### Task 0: Close resolved issues

**Run**: `gh issue close 310 311 330 331 332 333 334 -c "Already implemented in current codebase."`

Close #335 separately after Task 2 removes the `allow(dead_code)`.

1. Run the command above.
2. For #335, close after Task 2 lands: `gh issue close 335 -c "Both items resolved."`

---

### Task 1: Enforce total image pull size limit (#319)

**Crate**: `minibox-core`
**File(s)**: `crates/minibox-core/src/image/registry.rs`
**Run**: `cargo nextest run -p minibox-core -- registry`

#### 1a. Add constant and pre-pull manifest check

Add a `MAX_TOTAL_IMAGE_SIZE` constant and a pre-download check that sums
declared layer sizes from the manifest.

```rust
// After existing constants (line ~54):
/// SECURITY: Aggregate ceiling across all layers in a single image pull.
/// An image whose manifest declares more than 50 GiB of compressed layers
/// is rejected before any blob download begins.
const MAX_TOTAL_IMAGE_SIZE: u64 = 50 * 1024 * 1024 * 1024; // 50 GiB
```

In `pull_image`, after the manifest is fetched and before the layer download
loop (after line 555), add:

```rust
// SECURITY: Reject manifests whose declared aggregate layer size exceeds
// the total image budget. Descriptor sizes may be absent or understated,
// so this is a first-pass guard — the streaming counter below is the
// authoritative enforcement.
let declared_total: u64 = manifest.layers.iter().map(|l| l.size).sum();
if declared_total > MAX_TOTAL_IMAGE_SIZE {
    anyhow::bail!(
        "image exceeds total size limit: {declared_total} bytes declared \
         across {} layers (max {MAX_TOTAL_IMAGE_SIZE})",
        manifest.layers.len()
    );
}
```

#### 1b. Add streaming aggregate counter

Pass a shared `Arc<AtomicU64>` into each layer task to track actual bytes
downloaded. Check after each layer completes.

In `pull_image`, before the `for` loop:

```rust
use std::sync::atomic::{AtomicU64, Ordering};

let total_downloaded = Arc::new(AtomicU64::new(0));
```

Inside the `join_set.spawn` async block, after the `spawn_blocking` call
returns successfully, add:

```rust
// Account for actual bytes downloaded (compressed, on-wire).
let layer_bytes = hashing_reader.bytes_read();
let running_total = total_downloaded.fetch_add(
    layer_bytes, Ordering::Relaxed
) + layer_bytes;
if running_total > MAX_TOTAL_IMAGE_SIZE {
    anyhow::bail!(
        "image pull aborted: aggregate download {running_total} bytes \
         exceeds limit {MAX_TOTAL_IMAGE_SIZE}"
    );
}
```

This requires `hashing_reader.bytes_read()` — check if `HashingReader`
exposes byte count. If not, track via the `LimitedStream::consumed()`
value instead (pass it out of `spawn_blocking` as a return value).

#### 1c. Write failing test

```rust
#[tokio::test]
async fn pull_image_rejects_manifest_exceeding_total_size_limit() {
    // Set up mock server with a manifest declaring layers that sum
    // to > MAX_TOTAL_IMAGE_SIZE. Verify pull_image returns an error
    // containing "total size limit".
}
```

Run: `cargo nextest run -p minibox-core -- total_size_limit`
Expected: FAIL before implementation, PASS after.

#### 1d. Apply same limit to GHCR adapter

**File**: `crates/minibox/src/adapters/ghcr.rs`

Add `MAX_TOTAL_IMAGE_SIZE` constant (same value). Add the same pre-pull
manifest size sum check in `pull_image` (around line 328).

#### 1e. Update security docs

**File**: `docs/SECURITY_INVARIANTS.mbx.md`

Add entry documenting `MAX_TOTAL_IMAGE_SIZE`, the enforcing functions
(`RegistryClient::pull_image`, `GhcrRegistry::pull_image`), and the
dual enforcement model (manifest declaration + streaming aggregate).

#### 1f. Verify

```
cargo nextest run -p minibox-core -- registry
cargo nextest run -p minibox -- ghcr
cargo clippy --workspace -- -D warnings
```

Commit: `fix(registry): enforce total image pull size limit (fixes #319)`

---

### Task 2: Remove stale #[allow(dead_code)] from PreparedRun (#335)

**Crate**: `minibox`
**File(s)**: `crates/minibox/src/daemon/handler.rs`
**Run**: `cargo clippy -p minibox -- -D warnings`

1. Remove `#[allow(dead_code)]` from `PreparedRun` (line 644).
2. Run `cargo check -p minibox` to verify no dead_code warnings appear.
   If any fields are genuinely unused, remove them rather than
   re-adding the allow.
3. Verify: `cargo clippy -p minibox -- -D warnings` — zero warnings.
4. Commit: `chore(handler): remove stale allow(dead_code) from PreparedRun (fixes #335)`

---

### Task 3: Remove unused import in conformance_snapshot (#322)

**Crate**: `minibox` (test)
**File(s)**: `crates/minibox/tests/conformance_snapshot.rs`
**Run**: `cargo check --tests -p minibox 2>&1 | grep conformance_snapshot`

1. Remove the unused `use tempfile::TempDir;` import (line 10).
2. Check if `daemon_adapter_suite_tests.rs` still has unused `MockLimiter`
   / `MockNetwork` imports. If so, remove them too.
3. Verify: `cargo check --tests -p minibox` — zero warnings for these
   files.
4. Commit: `chore(tests): remove unused imports in conformance and adapter suite tests (fixes #322)`

---

### Task 4: Remove dead code in protocol_e2e_tests (#323)

**Crate**: `miniboxd` (test)
**File(s)**: `crates/miniboxd/tests/protocol_e2e_tests.rs`
**Run**: `cargo check --tests -p miniboxd 2>&1 | grep protocol_e2e`

1. Read the file and identify all dead items flagged by `cargo check`:
   - unused import `Path`
   - struct `DaemonFixture` (never constructed)
   - function `extract_container_id` (never used)
   - struct `ExecResult` (never constructed)
   - struct `SandboxClient` (never constructed)
   - associated items on the above structs
2. Remove all dead items. These are scaffolding from a previous e2e
   harness that was never completed.
3. Verify: `cargo check --tests -p miniboxd` — zero warnings.
4. Commit: `chore(e2e): remove dead code in protocol_e2e_tests (fixes #323)`

---

## Execution Order

Tasks 1-4 are independent. Recommended parallel grouping:

| Slot | Tasks | Rationale |
|------|-------|-----------|
| A | Task 0 + Task 1 | Security fix, close resolved issues |
| B | Task 2 + Task 3 + Task 4 | Cleanup, no overlapping files |

## Post-Completion

After all tasks land on main:

```
cargo check --tests --workspace  # zero warnings
cargo clippy --workspace -- -D warnings  # zero warnings
```

Verify all 11 issues are closed:
`gh issue list --state open --limit 20 --json number,title`
