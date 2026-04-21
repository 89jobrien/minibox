# Parallel Layer Pulls Design

**Date:** 2026-03-18
**Status:** Approved
**Scope:** `minibox` — `image/layer.rs`, `image/mod.rs`, `image/registry.rs`, `error.rs`, `Cargo.toml`

## Overview

Replace the sequential layer download loop in `pull_image` with bounded parallel pipelines that stream each layer directly from HTTP into the tar extractor — no per-layer heap buffer. Matches Docker/containerd behaviour: up to 4 concurrent pipelines, each pipelined (download overlaps with extraction of other layers).

## Background

Current `pull_image` flow (sequential):

```
for each layer:
    pull_layer()      → Bytes (full blob buffered in memory)
    verify_digest()   → SHA256 of Bytes
    store_layer()     → read_to_end buffer → extract_layer(&[u8])
```

Layer 2 does not start until layer 1 is fully extracted. A 5-layer image with 200 MB layers downloads and extracts ~1 GB sequentially.

## Design

### Component Changes

#### File: Change

- `layer.rs`: `extract_layer(&[u8], &Path)` → `extract_layer(impl Read, &Path)`. Add `HashingReader<R>`.
- `image/mod.rs`: `store_layer` removes internal `read_to_end` buffer; passes reader directly to `extract_layer`.
- `registry.rs`: Replace sequential `for` loop with `JoinSet` + `Arc<Semaphore>`. `pull_layer` returns `reqwest::Response` instead of `Bytes`. Add `MAX_CONCURRENT_LAYERS = 4`. Add `LimitedStream`. The existing `info!("pulled blob {} ({} bytes)", ...)` log line moves into the `spawn_blocking` result, logging compressed bytes received (available from `HashingReader`'s byte count) after extraction completes.
- `error.rs`: Add `RegistryError::LayerTask { digest, message }` for `JoinError` wrapping.
- `Cargo.toml`: Add `tokio-util = { version = "0.7", features = ["io"] }` and `pin-project-lite = "0.2"` to workspace and `minibox`.

### `HashingReader<R: Read>`

A transparent `Read` wrapper that feeds all bytes through a `Sha256` hasher as they pass through. After the reader is consumed, `finalize() -> String` returns the hex-encoded digest of the compressed bytes (matching OCI digest spec — digests are computed over the compressed blob, not the decompressed tar).

```rust
pub struct HashingReader<R> {
    inner: R,
    hasher: Sha256,
    bytes_read: u64,
}

impl<R: Read> HashingReader<R> {
    pub fn new(inner: R) -> Self { ... }
    /// Returns hex digest of all bytes read so far (compressed bytes, matching OCI spec).
    pub fn finalize(self) -> String { hex::encode(self.hasher.finalize()) }
    pub fn bytes_read(&self) -> u64 { self.bytes_read }
}

impl<R: Read> Read for HashingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.hasher.update(&buf[..n]);
        self.bytes_read += n as u64;
        Ok(n)
    }
}
```

### `extract_layer` Signature Change

```rust
// Before
pub fn extract_layer(tar_gz_data: &[u8], dest: &Path) -> anyhow::Result<()>

// After
pub fn extract_layer(reader: impl Read, dest: &Path) -> anyhow::Result<()>
```

The body replaces `GzDecoder::new(tar_gz_data)` with `GzDecoder::new(reader)`. All internal logic (path validation, device rejection, symlink rewrite, setuid strip) is unchanged.

The existing `#[instrument]` attribute references `tar_gz_data.len()` in the `fields` macro — this must be removed or replaced since `impl Read` has no `.len()`. The `bytes` instrument field is dropped; callers may pass size via a wrapper span if needed.

Existing callers passing `std::io::Cursor::new(bytes)` continue to compile without changes.

### `store_layer` Change

Removes the internal `read_to_end` buffer and switches to an atomic write pattern (extract to temp dir, rename on success):

```rust
// Before
let mut buf = Vec::new();
data_reader.read_to_end(&mut buf)?;
extract_layer(&buf, &dest)?;

// After
let tmp = layers_dir.join(format!("{digest_key}.tmp"));
fs::create_dir_all(&tmp)?;
match extract_layer(data_reader, &tmp) {
    Ok(()) => {
        match fs::rename(&tmp, &dest) {
            Ok(()) => {}
            Err(_) if dest.exists() => { fs::remove_dir_all(&tmp).ok(); } // duplicate-digest race
            Err(e) => return Err(e.into()),
        }
    }
    Err(e) => {
        fs::remove_dir_all(&tmp).ok(); // best-effort cleanup
        return Err(e);
    }
}
```

### Per-Layer Pipeline

Each layer runs as an independent `tokio::spawn` task. The shared `Arc<Semaphore>` (capacity = `MAX_CONCURRENT_LAYERS`) gates how many are active:

```
tokio::spawn (per layer)
  │
  ├─ acquire semaphore permit         (async; .expect() — semaphore is never closed)
  ├─ skip if layer_dir.exists()       (cache hit — prior pull fully verified)
  ├─ GET /blobs/{digest}              (async HTTP, check status + content-length limit)
  ├─ wrap stream in LimitedStream     (enforces MAX_LAYER_SIZE byte limit while streaming)
  ├─ capture Handle::current()
  ├─ move response into spawn_blocking closure
  │
  └─ spawn_blocking (response moved in) ─────────────────────────────────
       SyncIoBridge::new_with_handle(
           StreamReader::new(limited_stream),   ← AsyncRead wrapping LimitedStream
           handle
       )
         └─ HashingReader (inside spawn_blocking)  ← hashes compressed bytes in-flight
              └─ [passed to store_layer as &mut HashingReader]
                   └─ extract_layer(reader, dest)
                        └─ GzDecoder(reader) → tar::Archive → entries → dest
       actual = hashing_reader.finalize()
       if actual != expected_hex:
           remove_dir_all(dest)        ← rollback partial extraction
           return Err(DigestMismatch)
     ──────────────────────────────────────────────────────────────────────

  permit drops (RAII)
```

`HashingReader` is constructed inside the `spawn_blocking` closure — it wraps a sync `Read` (`SyncIoBridge`) and must live on the blocking thread. It wraps the compressed byte stream (before `GzDecoder`) so it hashes what the registry sent, matching the OCI manifest digest.

**Semaphore acquisition:** `sem.acquire_owned().await.expect("semaphore closed")` — `AcquireError` only fires if `Semaphore::close()` is called, which never happens in this design. Using `.expect()` is appropriate and avoids an unnecessary error type conversion.

**Atomic layer writes:** `store_layer` extracts into a sibling temp directory (`{digest_key}.tmp`) and renames it to the final `{digest_key}` directory only after `extract_layer` completes successfully. This ensures the final directory is either absent or fully written — never partially extracted. If a rename finds the destination already exists (duplicate-digest race), it removes its own temp dir and returns `Ok` (idempotent). `dest.exists()` in the skip check is therefore always a reliable signal that the layer is complete.

**Size limiting while streaming:** The existing `Content-Length` header check is insufficient alone (header may be absent or incorrect). A custom `LimitedStream` adapter wraps the `reqwest` bytes stream before `StreamReader`. It is a `Stream<Item=io::Result<Bytes>>` that maintains a running byte count and returns `Err(io::ErrorKind::InvalidData)` if the total exceeds `MAX_LAYER_SIZE`. This replaces the chunk-by-chunk accumulation check in the current `pull_layer`. `LimitedStream` is defined in `registry.rs` (private, ~20 lines implementing `futures::Stream` via `pin_project`).

### `pull_image` Loop Replacement

```rust
let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_LAYERS));
let mut join_set = JoinSet::new();

for layer_desc in manifest.layers.iter().cloned() {
    let client = self.clone();
    let store = store.clone();
    let token = token.clone();
    let sem = semaphore.clone();
    let name = name.to_owned();
    let tag = tag.to_owned();

    join_set.spawn(async move {
        let _permit = sem.acquire_owned().await.expect("semaphore closed");
        // ... per-layer pipeline
    });
}

while let Some(result) = join_set.join_next().await {
    match result {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            join_set.abort_all();
            return Err(e);
        }
        Err(join_err) => {
            join_set.abort_all();
            return Err(RegistryError::LayerTask {
                digest: "(unknown)".into(),
                message: join_err.to_string(),
            }.into());
        }
    }
}
```

The manifest is only written after all tasks complete successfully.

### New Error Variant

In `error.rs`, added to `RegistryError`:

```rust
#[error("layer task failed for {digest}: {message}")]
LayerTask { digest: String, message: String },
```

Wraps `tokio::task::JoinError` (panic or cancellation) into a typed registry error.

### Rollback on Digest Mismatch

Digest verification moves from before extraction to after:

1. `extract_layer` runs while bytes stream in
2. `hashing_reader.finalize()` computes digest of compressed bytes
3. If mismatch: `remove_dir_all(layer_dir)` (best-effort, logged as warning on failure), then return `ImageError::DigestMismatch`
4. If `remove_dir_all` itself fails: log warning, still return the original `DigestMismatch` error

### New Dependency

`tokio-util` with `io` feature provides:

- `tokio_util::io::StreamReader` — wraps `Stream<Item=Bytes>` into `AsyncRead`
- `tokio_util::io::SyncIoBridge` — bridges `AsyncRead` into `std::io::Read` for use inside `spawn_blocking`

## Error Handling Summary

| Case                                 | Error                                           |
| ------------------------------------ | ----------------------------------------------- |
| HTTP error / network drop mid-stream | `RegistryError::BlobFetch`                      |
| Digest mismatch after extraction     | `ImageError::DigestMismatch`                    |
| `spawn_blocking` panic               | `RegistryError::LayerTask`                      |
| Rollback failure                     | Warning logged; `DigestMismatch` still returned |

On any task error, `pull_image` calls `join_set.abort_all()` and drains remaining tasks before propagating. Manifest is never written on partial failure.

## Testing

### Existing Tests (unchanged)

- All `layer.rs` unit tests pass — `extract_layer` signature change is backward-compatible via `std::io::Cursor`
- Existing e2e tests exercise `pull_image` end-to-end on Linux+root and cover the parallel path

### New Unit Tests

**In `layer.rs`:**

- `hashing_reader_computes_correct_digest` — feed known bytes, verify `finalize()` matches expected sha256
- `hashing_reader_transparent_read` — verify bytes pass through `HashingReader` unchanged

**In `registry.rs`:**

- `pull_image_parallel_cache_skip` — construct an `ImageStore` with pre-existing layer dirs; assert skipped layers produce no HTTP requests

## Non-Goals

- Configurable concurrency limit (hardcoded `MAX_CONCURRENT_LAYERS = 4` for now)
- Cross-image layer deduplication (sharing in-progress downloads between two simultaneous `pull_image` calls)
- Fully async extraction with `tokio-tar` / `async-compression` (not needed; bottleneck is network)
