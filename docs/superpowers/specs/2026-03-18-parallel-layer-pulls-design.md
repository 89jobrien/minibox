# Parallel Layer Pulls Design

**Date:** 2026-03-18
**Status:** Approved
**Scope:** `minibox-lib` — `image/layer.rs`, `image/mod.rs`, `image/registry.rs`, `error.rs`, `Cargo.toml`

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

| File | Change |
|------|--------|
| `layer.rs` | `extract_layer(&[u8], &Path)` → `extract_layer(impl Read, &Path)`. Add `HashingReader<R>`. |
| `image/mod.rs` | `store_layer` removes internal `read_to_end` buffer; passes reader directly to `extract_layer`. |
| `registry.rs` | Replace sequential `for` loop with `JoinSet` + `Arc<Semaphore>`. `pull_layer` returns `reqwest::Response` instead of `Bytes`. Add `MAX_CONCURRENT_LAYERS = 4`. |
| `error.rs` | Add `RegistryError::LayerTask { digest, message }` for `JoinError` wrapping. |
| `Cargo.toml` | Add `tokio-util = { version = "0.7", features = ["io"] }` to workspace and `minibox-lib`. |

### `HashingReader<R: Read>`

A transparent `Read` wrapper that feeds all bytes through a `Sha256` hasher as they pass through. After the reader is consumed, `finalize() -> String` returns the hex-encoded digest of the compressed bytes (matching OCI digest spec — digests are computed over the compressed blob, not the decompressed tar).

```rust
pub struct HashingReader<R> {
    inner: R,
    hasher: Sha256,
}

impl<R: Read> HashingReader<R> {
    pub fn new(inner: R) -> Self { ... }
    pub fn finalize(self) -> String { hex::encode(self.hasher.finalize()) }
}

impl<R: Read> Read for HashingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.hasher.update(&buf[..n]);
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

Existing callers passing `std::io::Cursor::new(bytes)` continue to compile without changes.

### `store_layer` Change

Removes the internal `read_to_end` buffer:

```rust
// Before
let mut buf = Vec::new();
data_reader.read_to_end(&mut buf)?;
extract_layer(&buf, &dest)?;

// After
extract_layer(data_reader, &dest)?;
```

### Per-Layer Pipeline

Each layer runs as an independent `tokio::spawn` task. The shared `Arc<Semaphore>` (capacity = `MAX_CONCURRENT_LAYERS`) gates how many are active:

```
tokio::spawn (per layer)
  │
  ├─ acquire semaphore permit         (async, blocks if 4 already active)
  ├─ skip if layer_dir.exists()       (cache hit — prior pull fully verified)
  ├─ GET /blobs/{digest}              (async HTTP, check status + content-length)
  ├─ capture Handle::current()
  │
  └─ spawn_blocking ──────────────────────────────────────────────────────
       SyncIoBridge::new_with_handle(
           StreamReader::new(response.bytes_stream()),
           handle
       )
         └─ HashingReader              ← hashes compressed bytes in-flight
              └─ [passed to store_layer as impl Read]
                   └─ extract_layer
                        └─ GzDecoder → tar::Archive → entries → dest
       actual = hashing_reader.finalize()
       if actual != expected_hex:
           remove_dir_all(dest)        ← rollback partial extraction
           return Err(DigestMismatch)
     ──────────────────────────────────────────────────────────────────────

  permit drops (RAII)
```

`HashingReader` wraps the compressed byte stream (before `GzDecoder`) so it hashes what the registry sent, matching the OCI manifest digest.

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
        let _permit = sem.acquire_owned().await?;
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

| Case | Error |
|------|-------|
| HTTP error / network drop mid-stream | `RegistryError::BlobFetch` |
| Digest mismatch after extraction | `ImageError::DigestMismatch` |
| `spawn_blocking` panic | `RegistryError::LayerTask` |
| Rollback failure | Warning logged; `DigestMismatch` still returned |

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
