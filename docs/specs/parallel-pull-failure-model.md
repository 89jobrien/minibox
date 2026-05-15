# Parallel Pull Failure Model

**Status**: Specified
**Issue**: #148
**Related**: #149 (layer storage state machine), #150 (LimitedStream contract), #151 (digest in task failures)

---

## Overview

`RegistryClient::pull_image` downloads all layers of an OCI image concurrently, up to
`MAX_CONCURRENT_LAYERS` (4) at once, using a `tokio::task::JoinSet`. This document defines
what happens when one or more of those concurrent tasks fail: how errors are classified, how
they surface to the caller, what filesystem state remains, and what retry semantics apply.

All code references are to `crates/minibox-core/src/image/registry.rs` unless noted.

---

## §1  Failure Scenarios

### 1.1 Single layer fails, others succeed

One task in the `JoinSet` returns `Err(_)`. The drain loop (`while let Some(join_result) =
join_set.join_next().await`, line 729) processes tasks in completion order. When the drain
loop encounters the first `Err`, it propagates it immediately via `?`. The remaining tasks
that are still running or queued are **not explicitly cancelled**; the `JoinSet` is dropped
when `pull_image` returns, which causes Tokio to abort all in-flight tasks.

The manifest is never written because the `store_manifest` call (line 742) is only reached
after the drain loop exits cleanly.

### 1.2 Multiple layers fail

The drain loop surfaces the **first** error it dequeues. Tokio's `JoinSet::join_next` returns
completed tasks in completion order, not spawn order, so the "first" error in calendar time
may not be the lowest-index layer. All subsequent in-flight tasks are dropped when the
`JoinSet` is dropped.

No coalescence of multiple errors occurs. Only one `anyhow::Error` chain is visible to the
caller.

### 1.3 All layers fail

Identical to §1.2. The first completed failing task's error is returned; the rest are
silently dropped.

### 1.4 Partial success (some layers complete before others fail)

Layers that completed successfully before the first error is drained have had their `tmp_dir`
atomically renamed to `layer_dir` (line 694). Those directories persist on disk in a valid
state. See §3 for state detail.

---

## §2  Error Propagation

### 2.1 Task return type

Every spawned task returns `(String, anyhow::Result<()>)` — a `(digest, result)` pair.
The digest is captured at spawn time (line 566: `let task_digest = layer_desc.digest.clone()`)
so that even a panicking task carries an actionable identifier.

### 2.2 Drain loop

```text
registry.rs line 729–735

while let Some(join_result) = join_set.join_next().await {
    let (digest, result) = join_result.map_err(|e| RegistryError::LayerTask {
        digest: "(outer task panicked or was cancelled)".to_owned(),
        source: e,
    })?;
    result.with_context(|| format!("layer digest {digest}"))?;
}
```

Two layers of error wrapping apply:

1. `JoinError` (task panic / cancellation) → `RegistryError::LayerTask { digest, source }`.
   The digest is the string `"(outer task panicked or was cancelled)"` in this case because
   the panic occurred before the `(digest, result)` pair could be returned. For the inner
   `spawn_blocking` panic path (line 708), the digest captured at spawn time is used.
2. Application error from within the task → wrapped with `"layer digest {digest}"` context.

The caller receives a single `anyhow::Error` with a chain that includes the layer digest and
the root cause (network error, digest mismatch, extraction failure, or size-limit error).

### 2.3 No structured multi-error type

There is no `Vec<Error>` or structured multi-error return. The caller cannot inspect which
other layers failed; only the first error is visible. This is an intentional simplification:
a failed pull must be retried in full, so granular per-layer error reporting is not needed
at this level.

### 2.4 In-flight task fate

When `pull_image` returns `Err`, the `JoinSet` is dropped. Tokio aborts all tasks that were
still running. Any `spawn_blocking` task that was mid-extraction is detached from the
`JoinSet` and will complete on a blocking thread pool thread, but its result is not observed.
The `tmp_dir` from an aborted extraction remains on disk (see §3.3).

---

## §3  State After Failure

### 3.1 Layers that completed before the first error

These have been atomically renamed from `<digest>.tmp/` to `<digest>/` (line 694). They are
in a valid, complete state and will be treated as cached on any subsequent pull attempt
(line 587: `if layer_dir.exists() { return Ok(()); }`). No cleanup is performed and none
is required.

### 3.2 The failing layer itself

All error paths in the `spawn_blocking` closure clean up `tmp_dir` before returning `Err`:

- **Digest mismatch** (line 665): `remove_dir_all(&tmp_dir)` is attempted; failure is
  logged at `warn!` level but the cleanup error is not propagated.
- **Extract error** (line 682): same pattern — `remove_dir_all(&tmp_dir)` attempted; failure
  logged at `warn!`.
- **Rename race** (line 694): if the destination already exists, `tmp_dir` is removed and
  `Ok(())` is returned. If rename fails for any other reason, `tmp_dir` is removed and the
  rename error is returned.

In the common case the `tmp_dir` is absent after a layer failure. In the warn-logged failure
case it may remain on disk as a partial directory.

### 3.3 Layers aborted by JoinSet drop

A `spawn_blocking` task mid-extraction when the `JoinSet` is dropped is not cancelled —
`spawn_blocking` tasks run to completion on the blocking thread pool. The extraction will
finish (or fail), but since the `JoinSet` no longer collects its result, no rename or cleanup
is triggered by the normal success/error paths in the closure. The `tmp_dir` for that layer
is left on disk.

`cargo xtask nuke-test-state` and its equivalent daemon-side cleanup are responsible for
removing stale `*.tmp` directories.

### 3.4 Manifest

The manifest is never stored on a failed pull (line 737: `store_manifest` is after the drain
loop, inside the success path). An image with some layer directories present but no manifest
file is considered incomplete and is not returned by `ImageStore::has_image`.

### 3.5 Summary table

| Layer                        | Filesystem state after failure            |
| ---------------------------- | ----------------------------------------- |
| Succeeded before first error | `layer_dir/` present — valid, cached      |
| Failed (cleanup succeeded)   | `tmp_dir` absent; `layer_dir` absent      |
| Failed (cleanup failed)      | `tmp_dir` present — partial, stale        |
| Aborted by JoinSet drop      | `tmp_dir` may be present — partial, stale |
| Not yet started              | Neither present                           |

---

## §4  Retry Semantics

### 4.1 No built-in retry

`pull_image` does not retry internally. A single failure causes the entire pull to abort and
return `Err`. There is no per-layer retry limit or backoff.

### 4.2 Caller responsibility

Retry is the caller's responsibility. The daemon handler or CLI command that calls
`pull_image` must decide whether to retry, how many times, and with what backoff. There is
no state machine tracking retry count inside `pull_image` itself.

### 4.3 Re-pull is safe and idempotent

Because completed layers are checked with `layer_dir.exists()` at the start of each task
(line 587), re-running `pull_image` for the same image will skip already-downloaded layers.
Only the failed and unstarted layers will be re-attempted. This makes retrying a failed pull
cheap when the failure was transient.

Stale `*.tmp` directories left by a previous aborted run are detected and removed at the
start of each task's `spawn_blocking` closure (line 638:
`if tmp_dir.exists() { remove_dir_all(&tmp_dir)? }`).

### 4.4 No retry limit enforcement point

There is no global or per-layer retry counter in the codebase. If callers implement retry
loops, they must enforce their own limits to avoid infinite retry on permanent errors (e.g.,
missing blob, permanent auth failure).

---

## §5  Interaction with LimitedStream

### 5.1 Where LimitedStream is inserted

Each layer task wraps the HTTP response body in `LimitedStream::new(stream, MAX_LAYER_SIZE)`
(lines 606–609, `MAX_LAYER_SIZE` = 10 GiB). The limited stream is then bridged sync via
`SyncIoBridge` and fed to `HashingReader` → `GzDecoder` → `tar::Archive`.

### 5.2 When the size limit fires

`LimitedStream` yields `Err(io::Error)` when the byte count exceeds `MAX_LAYER_SIZE`
(see `docs/specs/limited-stream-contract.md` §2). That error propagates up through
`SyncIoBridge` as an `io::Error`, then through `extract_layer` or the `io::copy` drain call.

Because the limit fires mid-stream, `HashingReader` has not seen the full compressed bytes.
The digest verification that follows (line 657) will compute an incomplete hash and, in
almost all cases, report a mismatch. The mismatch cleanup path (line 665) removes `tmp_dir`.

The error returned to the drain loop will carry context like:
`"layer too large" / "digest mismatch"` depending on which error surface wins. The layer is
left in state: `tmp_dir` absent, `layer_dir` absent — identical to a normal failure.

### 5.3 Content-Length pre-check

Before the stream is even created, `pull_layer_response` checks the `Content-Length` header
(line 464) and rejects the response immediately with `RegistryError::Other("layer too
large: ...")` if it already exceeds `MAX_LAYER_SIZE`. In this case no streaming occurs, no
`tmp_dir` is created, and `LimitedStream` is never instantiated.

The `LimitedStream` cap is a defense-in-depth second line: it catches cases where
`Content-Length` is absent, incorrect, or the server misbehaves after the header was
accepted.

### 5.4 Error classification

A size-limit error is treated as a permanent failure for that layer. It is not retried
differently from network or digest errors; the same drain-loop path applies (§2.2).

---

## §6  Test Plan

Tests in `crates/minibox-core/src/image/registry.rs` under `#[cfg(test)] mod tests`.

### Existing tests

| Test name                                  | Covers                                           |
| ------------------------------------------ | ------------------------------------------------ |
| `pull_image_downloads_and_stores_all_layers` ✓ | Happy path: all layers succeed, manifest stored |
| `pull_image_errors_when_auth_fails` ✓       | Auth failure before any layer task is spawned    |
| `pull_image_errors_when_blob_fetch_fails` ✓ | Single-layer HTTP 500 failure (§1.1)             |
| `pull_image_errors_on_digest_mismatch` ✓    | Digest mismatch on a single layer (§1.1, §3.2)  |
| `layer_task_join_error_contains_digest` ✓   | Panic in layer task maps to digest (§2.1, #151) |
| `pull_layer_errors_on_404` ✓               | HTTP 404 on blob fetch                           |

### Tests needed

| Test name                                          | Scenario (section)                                |
| -------------------------------------------------- | ------------------------------------------------- |
| `pull_image_second_layer_fails_first_cached`       | First layer cached; second fails — verify first  |
|                                                    | layer dir persists on disk (§1.4, §3.1)           |
| `pull_image_all_layers_fail`                       | All blobs return 500 — error is returned, no      |
|                                                    | manifest stored, layer dirs absent (§1.3, §3.4)   |
| `pull_image_skips_cached_layers_on_repull`         | Run pull_image twice; second run skips cached     |
|                                                    | layers (§4.3)                                     |
| `pull_image_stale_tmp_dir_removed_on_repull`       | Leave a `*.tmp` dir from a previous run; verify   |
|                                                    | it is removed before extraction starts (§4.3)     |
| `pull_image_size_limit_error_no_layer_dir`         | Serve a response with body > MAX_LAYER_SIZE via   |
|                                                    | chunked transfer (no Content-Length); verify      |
|                                                    | LimitedStream fires, tmp_dir absent (§5.2)        |
| `pull_image_content_length_too_large_rejected`     | Serve Content-Length > MAX_LAYER_SIZE; verify     |
|                                                    | rejection before streaming (§5.3)                 |
| `pull_image_manifest_not_stored_on_layer_failure`  | Layer fails; verify `store.has_image()` returns   |
|                                                    | false and no manifest file exists (§3.4)          |

---

## Implementation Notes

- The drain loop processes tasks in **completion order**, not spawn order. Tests that check
  which error is returned must not assume the lowest-index layer's error wins.
- The `semaphore` is not a retry gate — it is a concurrency limit. Acquiring the permit
  (`sem.acquire_owned().await`) can only fail if the semaphore is explicitly closed, which
  does not occur in normal operation. The `.expect("semaphore closed")` at line 573 is a
  programming-error guard, not a user-visible failure path.
- No layer state is persisted to disk beyond the presence or absence of `layer_dir/` and
  `layer.tmp/`. There is no `Failed` marker file or database entry. A layer is either
  `Complete` (directory exists) or `NotStarted`/`Failed` (directory absent). The distinction
  between "never started" and "failed" is not preserved across process restarts.
