# Parallel Layer Pull — Failure Model

**Applies to:** `crates/minibox-core/src/image/registry.rs`
**Last updated:** 2026-06-15
**Status:** Describes current behaviour as of this commit.

---

## Overview

`RegistryClient::pull_image` downloads OCI image layers concurrently using a
`tokio::task::JoinSet`. Up to `MAX_CONCURRENT_LAYERS` (4) layers run at the same time,
controlled by a `tokio::sync::Semaphore`. Each layer task returns `(digest, anyhow::Result<()>)`
so that the drain site can attach the layer digest to any error message.

This document catalogues every failure scenario, the current behaviour for each, and the
Rust error types involved.

---

## Error Types

### `RegistryError` (`crates/minibox-core/src/error.rs`)

The primary error type for network and registry failures:

| Variant | When raised |
| --- | --- |
| `RegistryError::Network(reqwest::Error)` | Any `reqwest` transport failure |
| `RegistryError::AuthFailed { image, message }` | HTTP non-2xx from auth endpoint |
| `RegistryError::ManifestFetch { name, tag, message }` | HTTP non-2xx from manifest endpoint |
| `RegistryError::BlobFetch { digest, message }` | HTTP non-2xx from blob endpoint |
| `RegistryError::NoPlatformManifest { platform }` | Multi-arch list has no matching entry |
| `RegistryError::ManifestNestingTooDeep` | Manifest list depth exceeds 2 levels |
| `RegistryError::LayerTask { digest, source: JoinError }` | Tokio task panicked or was cancelled |
| `RegistryError::Other(String)` | Size-limit violations and other ad-hoc errors |

### `ImageError` (`crates/minibox-core/src/error.rs`)

Raised inside `extract_and_verify_layer` (sync, runs in `spawn_blocking`):

| Variant | When raised |
| --- | --- |
| `ImageError::DigestMismatch { digest, expected, actual }` | SHA-256 of downloaded blob != manifest digest |
| `ImageError::LayerExtract(String)` | `extract_layer` returns an error |
| `ImageError::DeviceNodeRejected { entry }` | Tar entry is a block/char device |
| `ImageError::SymlinkTraversalRejected { entry, target }` | Symlink target escapes container root |
| `ImageError::StoreWrite { path, source }` | I/O error writing to the layer store |
| `ImageError::Io(std::io::Error)` | Other I/O errors during extraction |

---

## Failure Scenarios

### 1. Single layer returns HTTP 4xx (e.g. 404 Not Found)

**Where:** `pull_layer_response` in `RegistryClient`.

**Current behaviour:** `pull_layer_response` inspects `resp.status().is_success()`. A
non-2xx status causes an immediate return of `RegistryError::BlobFetch { digest, message }`.
The `message` field contains the HTTP status code and the response body text.

**Propagation:** The task returns `(digest, Err(RegistryError::BlobFetch { .. }))`. At the
drain loop (`join_set.join_next()`), the first error is unwrapped by
`.with_context(|| format!("layer digest {digest}"))` and returned from `pull_image`,
aborting the entire pull. Tasks still in-flight are not explicitly cancelled; `JoinSet` drops
them when `join_set` is dropped (via `JoinSet`'s drop implementation).

**Retry:** None. No retry logic exists at any level. A transient 404 is fatal.

**Cleanup:** Any partial `*.tmp` directory from a layer that started extracting before the
failure is left on disk. On the next pull attempt, `extract_and_verify_layer` removes stale
`.tmp` directories via `std::fs::remove_dir_all` before re-extraction.

---

### 2. Transient network error (connection reset, timeout, DNS failure)

**Where:** `pull_layer_response` → `self.http.get(url).send().await`.

**Current behaviour:** `reqwest` returns a `reqwest::Error`. This is wrapped as
`RegistryError::Network(e)` by `.map_err(RegistryError::Network)`. The task returns
`(digest, Err(RegistryError::Network(..)))`.

**Propagation:** Same fail-fast path as the 404 case: first error out of `join_set.join_next()`
propagates from `pull_image`. Other in-flight tasks are not cancelled.

**Retry:** None.

**Cleanup:** If the error occurs during the HTTP GET phase (before `spawn_blocking` is entered),
no `.tmp` directory exists yet. If the error occurs mid-stream during the `SyncIoBridge`/
`StreamReader` read inside `spawn_blocking`, `extract_and_verify_layer` surfaces it as
`ImageError::Io` or an unexpected EOF. The tmp dir is removed by `cleanup_tmp_dir` inside
`extract_and_verify_layer` on any extraction error path.

---

### 3. Partial download / premature stream EOF

**Where:** Inside `spawn_blocking` → `SyncIoBridge` → `StreamReader::new(limited)`.

**Current behaviour:** `LimitedStream` returns `Poll::Ready(None)` when the inner stream
ends, regardless of whether the expected bytes were consumed. `StreamReader` surfaces a
premature termination as `io::ErrorKind::UnexpectedEof` during `GzDecoder` read. This
propagates up through `HashingReader` and becomes an `io::Error` returned from
`extract_layer`, converted to `ImageError::LayerExtract`.

Regardless of whether extraction failed or succeeded, `extract_and_verify_layer` drains the
remaining bytes through `HashingReader` into `std::io::sink()` (line 271) before computing
the digest. For a truncated download the digest will not match, so `ImageError::DigestMismatch`
is returned even if the tar extraction itself did not error.

**Propagation:** Error returned from `spawn_blocking` closure → `tokio` join → wrapped by the
`map_err(|e| RegistryError::LayerTask { .. })?` → drain loop propagates from `pull_image`.

**Retry:** None.

**Cleanup:** `cleanup_tmp_dir` is called on `DigestMismatch`. On `LayerExtract` the tmp dir
is also cleaned up by `cleanup_tmp_dir` at the `if let Err(e) = extract_result` branch. See
`extract_and_verify_layer` lines 282–295.

---

### 4. Checksum mismatch

**Where:** `extract_and_verify_layer`, after `HashingReader::finalize()`.

**Current behaviour:** `HashingReader` wraps the raw (compressed) byte stream before
`GzDecoder` so the SHA-256 covers the compressed blob, matching the OCI manifest digest
format. After extraction completes (or fails), `actual_hex` is compared to the `sha256:`
prefix stripped from `digest`. If they differ, `ImageError::DigestMismatch { digest,
expected, actual }` is returned.

Note: digest verification happens **after** extraction into the tmp dir, not before. A
corrupted layer fully extracts into `*.tmp` and is then discarded on mismatch.

**Propagation:** Same fail-fast path. The task returns the error; the drain loop surfaces it.

**Retry:** None. A persistent checksum mismatch (e.g. corrupted CDN cache) cannot be
recovered automatically.

**Cleanup:** `cleanup_tmp_dir` is called at the `DigestMismatch` branch
(`extract_and_verify_layer` line 282–288). The final `layer_dir` is never created.

---

### 5. Registry rate limiting (HTTP 429)

**Where:** `pull_layer_response`, `authenticate`, or `get_manifest`.

**Current behaviour:** Any non-2xx status is treated as a fatal error. HTTP 429 produces
`RegistryError::BlobFetch { digest, message: "HTTP 429: ..." }` (for blob requests) or
`RegistryError::ManifestFetch` / `RegistryError::AuthFailed` for other endpoints. No
`Retry-After` header is inspected and no backoff is applied.

**Propagation:** Fail-fast. First 429 aborts the pull.

**Retry:** Not implemented. Rate-limited pulls must be retried by the caller (the daemon
handler or the CLI).

**Cleanup:** Partial `.tmp` directories from layers that started before the 429 are cleaned
on the next attempt.

---

### 6. Individual layer blob exceeds `MAX_LAYER_SIZE` (10 GiB)

**Where:** Two checkpoints in the blob download path:

1. `pull_layer_response` inspects the `Content-Length` header before streaming. If the
   declared size exceeds `MAX_LAYER_SIZE`, `RegistryError::Other("layer too large: ...")` is
   returned immediately.
2. `LimitedStream::poll_next` counts bytes as they arrive. When `consumed > MAX_LAYER_SIZE`
   it returns `io::Error::new(io::ErrorKind::InvalidData, "layer stream exceeded size limit
   ...")`. This surfaces through `StreamReader` and `SyncIoBridge` as an `io::Error`, which
   becomes `ImageError::LayerExtract` or `ImageError::Io`.

**Propagation:** Fail-fast as above. `pull_image` aborts.

**Cleanup:** `cleanup_tmp_dir` runs on extraction errors inside `extract_and_verify_layer`.

---

### 7. Aggregate image exceeds `MAX_TOTAL_IMAGE_SIZE` (50 GiB)

**Where:** Two checkpoints:

1. Pre-pull (before any download): `pull_image` sums `layer.size` from the manifest.
   If `declared_total > MAX_TOTAL_IMAGE_SIZE`, `pull_image` returns an error before any
   task is spawned.
2. Per-task post-download: each task adds the actual compressed bytes to
   `downloaded_total` (an `Arc<AtomicU64>`). If `prev + actual_bytes > MAX_TOTAL_IMAGE_SIZE`
   the task returns an error. Other in-flight tasks are not stopped; the first error at
   drain aborts.

**Propagation:** Fail-fast at drain. Tasks that already completed have committed their layer
dirs to disk.

**Retry:** Not meaningful — the image is genuinely oversized.

**Cleanup:** Layers already extracted and atomically renamed to their final `layer_dir` are
NOT cleaned up. They remain in the image store as valid, reusable cached layers. The manifest
is never persisted (step 4 in `pull_image` is only reached after the drain loop succeeds), so
the partial image is invisible to `ImageStore::load_manifest` until a successful pull retries
remaining layers.

---

### 8. Tokio task panic or cancellation (`RegistryError::LayerTask`)

**Where:** `join_set.join_next()` returns `Err(JoinError)` when the inner task panicked or
was cancelled.

**Current behaviour:** The drain loop wraps the `JoinError` in `RegistryError::LayerTask {
digest, source }`. The `digest` field contains the layer digest captured at spawn time so the
error message is actionable. A fallback string `"(outer task panicked or was cancelled)"` is
used if `join_next` itself returns a `JoinError` at the outer level (the task managing the
`(digest, result)` tuple).

**Propagation:** Same fail-fast path.

**Cleanup:** Same as transient network errors — partial `.tmp` dirs are cleaned on retry.

---

## Concurrency and Fail-Fast Semantics

`pull_image` uses **fail-fast** semantics: the drain loop calls
`result.with_context(..)?` which returns from `pull_image` on the first layer error.

```
while let Some(join_result) = join_set.join_next().await {
    let (digest, result) = join_result.map_err(|e| RegistryError::LayerTask { .. })?;
    result.with_context(|| format!("layer digest {digest}"))?;  // <-- fail-fast
}
```

When `pull_image` returns, the `JoinSet` is dropped. `JoinSet::drop` aborts all tasks that
have not yet finished. There is no "collect all errors" mode.

**Consequence:** if layers 1–3 succeed and layer 4 fails, the caller sees a single error for
layer 4. Layers 1–3 are on disk and will be reused (cache-hit) on the next pull attempt.

---

## Cleanup Details

| Situation | `*.tmp` dir | Final `layer_dir` | Manifest |
| --- | --- | --- | --- |
| Extraction error | Removed by `cleanup_tmp_dir` | Not created | Not written |
| Digest mismatch | Removed by `cleanup_tmp_dir` | Not created | Not written |
| Rename succeeds | Renamed to `layer_dir` | Present | Written after all layers |
| Rename fails, dir already exists | `*.tmp` removed | Pre-existing dir kept | Written after all layers |
| Layer already cached (dir exists) | Not created | Not touched | Written after all layers |

The `cleanup_tmp_dir` function (`registry.rs` line 311) is best-effort: failure to remove a
stale `.tmp` directory is logged at `warn!` level but does not propagate.

---

## Missing / Not Yet Implemented

- **No retry logic.** All failure scenarios are terminal. There is no exponential backoff,
  jitter, or per-layer retry limit at any level of the stack. The caller (daemon handler)
  does not retry either.
- **No partial-pull resume.** Layers successfully downloaded and extracted in a failed pull
  are cached and reused on retry (because `layer_dir.exists()` short-circuits re-download),
  but this is an incidental property of the cache — not an explicit resume protocol.
- **No per-layer timeout.** The `reqwest` client has no per-request timeout configured.
  A stalled HTTP connection will block the layer task indefinitely.
- **No rate-limit back-off.** HTTP 429 responses carry no retry delay.
- **No aggregate error collection.** The first layer error aborts the pull. There is no
  mechanism to report all failed layers in a single error value.
- **No cleanup of committed layers on abort.** Layers that finished extracting before an
  abort are left in the image store. This is generally correct (they are valid and reusable),
  but there is no GC trigger on failed pulls.

---

## Call Graph (simplified)

```
pull_image
  authenticate -> RegistryError::AuthFailed / Network
  get_manifest  -> RegistryError::ManifestFetch / NoPlatformManifest / Other
  [size pre-check] -> anyhow bail
  JoinSet::spawn (x N layers, semaphore cap = 4)
    pull_layer_response -> RegistryError::BlobFetch / Network / Other
    LimitedStream        -> io::Error (size exceeded)
    spawn_blocking
      extract_and_verify_layer
        extract_layer   -> ImageError::LayerExtract / DeviceNodeRejected / ...
        HashingReader   -> ImageError::DigestMismatch
        rename tmp->dir -> io::Error
  drain loop (join_next)
    JoinError           -> RegistryError::LayerTask
    layer Err           -> propagate from pull_image (fail-fast)
  store_manifest        -> ImageError::StoreWrite
```
