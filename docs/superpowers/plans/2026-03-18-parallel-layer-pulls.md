---
status: done
completed: "2026-03-18"
branch: feat/parallel-layer-pulls
note: Parallel pulls shipped
---

# Parallel Layer Pulls Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the sequential layer download loop in `pull_image` with bounded parallel streaming pipelines — each layer streams directly from HTTP into the tar extractor with no per-layer heap buffer, capped at 4 concurrent downloads.

**Architecture:** Add `HashingReader<R>` (transparent `Read` wrapper computing SHA256 in-flight) and `LimitedStream` (streaming byte-count enforcer) to `minibox`. Change `extract_layer` to accept `impl Read`, make `store_layer` atomic (temp-dir + rename), then replace `pull_image`'s sequential `for` loop with a `JoinSet` + `Arc<Semaphore>` loop where each layer task uses `SyncIoBridge` to bridge the async reqwest stream into the sync extraction pipeline.

**Tech Stack:** Rust 2024, Tokio, reqwest (stream feature), tokio-util (io feature), pin-project-lite, sha2, hex, flate2, tar, futures

**Spec:** `docs/superpowers/specs/2026-03-18-parallel-layer-pulls-design.md`

---

## File Map

| File                                   | Change                                                                                                                     |
| -------------------------------------- | -------------------------------------------------------------------------------------------------------------------------- |
| `Cargo.toml` (workspace)               | Add `tokio-util`, `pin-project-lite`                                                                                       |
| `crates/minibox/Cargo.toml`            | Add `tokio-util`, `pin-project-lite`                                                                                       |
| `crates/minibox/src/error.rs`          | Add `RegistryError::LayerTask` variant                                                                                     |
| `crates/minibox/src/image/layer.rs`    | Add `HashingReader<R>`, change `extract_layer(&[u8])` → `extract_layer(impl Read)`                                         |
| `crates/minibox/src/image/mod.rs`      | Rewrite `store_layer`: remove `read_to_end` buffer, add atomic temp-dir+rename                                             |
| `crates/minibox/src/image/registry.rs` | Add `LimitedStream`, change `pull_layer` → returns `reqwest::Response`, replace sequential loop with `JoinSet`+`Semaphore` |

---

## Task 1: Add dependencies

**Files:**

- Modify: `Cargo.toml`
- Modify: `crates/minibox/Cargo.toml`

- [ ] **Step 1: Add workspace deps**

In `Cargo.toml` under `[workspace.dependencies]`, add:

```toml
tokio-util = { version = "0.7", features = ["io"] }
pin-project-lite = "0.2"
```

- [ ] **Step 2: Add to minibox**

In `crates/minibox/Cargo.toml` under `[dependencies]`, add:

```toml
tokio-util = { workspace = true }
pin-project-lite = { workspace = true }
```

- [ ] **Step 3: Verify it compiles**

```bash
cargo check -p minibox
```

Expected: no errors.

- [ ] **Step 4: fmt + commit**

```bash
cargo fmt --all
git add Cargo.toml Cargo.lock crates/minibox/Cargo.toml
git commit -m "chore: add tokio-util and pin-project-lite deps"
```

---

## Task 2: Add `RegistryError::LayerTask` to `error.rs`

**Files:**

- Modify: `crates/minibox/src/error.rs`

- [ ] **Step 1: Write failing test**

At the bottom of `error.rs`, add a `#[cfg(test)]` block:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layer_task_error_formats() {
        let e = RegistryError::LayerTask {
            digest: "sha256:abc123".into(),
            message: "task panicked".into(),
        };
        let s = e.to_string();
        assert!(s.contains("sha256:abc123"), "got: {s}");
        assert!(s.contains("task panicked"), "got: {s}");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p minibox error::tests::layer_task_error_formats
```

Expected: compile error — `LayerTask` variant does not exist yet.

- [ ] **Step 3: Add the variant**

In `crates/minibox/src/error.rs`, inside `RegistryError`, add after the `Other` variant:

```rust
#[error("layer task failed for {digest}: {message}")]
LayerTask { digest: String, message: String },
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo nextest run -p minibox error::tests
```

Expected: PASS.

- [ ] **Step 5: clippy + fmt + commit**

```bash
cargo fmt --all
cargo clippy -p minibox -- -D warnings
git add crates/minibox/src/error.rs
git commit -m "feat: add RegistryError::LayerTask for JoinError wrapping"
```

---

## Task 3: Add `HashingReader<R>` to `layer.rs`

**Files:**

- Modify: `crates/minibox/src/image/layer.rs`

`HashingReader` is a transparent `std::io::Read` wrapper that feeds all bytes through a `Sha256` hasher. It goes at the top of the file (after imports). It wraps the **compressed** byte stream (before `GzDecoder`) so it hashes what the registry sent, matching the OCI manifest digest spec.

- [ ] **Step 1: Write failing tests**

Add at the bottom of the existing `#[cfg(test)]` block in `layer.rs`:

```rust
// ---------------------------------------------------------------------------
// HashingReader
// ---------------------------------------------------------------------------

#[test]
fn hashing_reader_computes_correct_digest() {
    use sha2::{Digest as _, Sha256};

    let data = b"hello parallel layers";
    let expected_hex = hex::encode(Sha256::digest(data));

    let mut reader = HashingReader::new(std::io::Cursor::new(data));
    let mut out = Vec::new();
    std::io::Read::read_to_end(&mut reader, &mut out).unwrap();

    assert_eq!(reader.finalize(), expected_hex);
}

#[test]
fn hashing_reader_transparent_read() {
    let data = b"some bytes 12345";
    let mut reader = HashingReader::new(std::io::Cursor::new(data));
    let mut out = Vec::new();
    std::io::Read::read_to_end(&mut reader, &mut out).unwrap();
    assert_eq!(out, data);
}

#[test]
fn hashing_reader_bytes_read_counter() {
    let data = b"counting bytes here";
    let mut reader = HashingReader::new(std::io::Cursor::new(data));
    let mut out = Vec::new();
    std::io::Read::read_to_end(&mut reader, &mut out).unwrap();
    assert_eq!(reader.bytes_read(), data.len() as u64);
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test -p minibox image::layer::tests::hashing_reader
```

Expected: compile error — `HashingReader` not defined yet.

- [ ] **Step 3: Implement `HashingReader`**

Add after the existing imports at the top of `layer.rs`:

```rust
use sha2::{Digest as _, Sha256};

/// Transparent [`std::io::Read`] wrapper that computes a SHA-256 digest of all
/// bytes passing through it.
///
/// Wraps the **compressed** byte stream (before [`flate2::read::GzDecoder`]) so
/// the hash matches the OCI manifest digest, which is computed over the
/// compressed blob.
pub struct HashingReader<R> {
    inner: R,
    hasher: Sha256,
    bytes_read: u64,
}

impl<R: std::io::Read> HashingReader<R> {
    pub fn new(inner: R) -> Self {
        Self {
            inner,
            hasher: Sha256::new(),
            bytes_read: 0,
        }
    }

    /// Finalise the hasher and return the hex-encoded SHA-256 digest of all
    /// bytes read so far.
    pub fn finalize(self) -> String {
        hex::encode(self.hasher.finalize())
    }

    /// Total compressed bytes read through this reader.
    pub fn bytes_read(&self) -> u64 {
        self.bytes_read
    }
}

impl<R: std::io::Read> std::io::Read for HashingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.hasher.update(&buf[..n]);
        self.bytes_read += n as u64;
        Ok(n)
    }
}
```

Note: `sha2` is already imported elsewhere in the crate — check if the `use sha2::{Digest as _, Sha256};` import conflicts with any existing imports at the top of `layer.rs`. Remove duplicates if so.

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo nextest run -p minibox image::layer::tests
```

Expected: all PASS (new tests + existing layer tests).

- [ ] **Step 5: clippy + fmt + commit**

```bash
cargo fmt --all
cargo clippy -p minibox -- -D warnings
git add crates/minibox/src/image/layer.rs
git commit -m "feat: add HashingReader to layer.rs — transparent SHA-256 in-flight hashing"
```

---

## Task 4: Change `extract_layer` signature from `&[u8]` to `impl Read`

**Files:**

- Modify: `crates/minibox/src/image/layer.rs`

The existing `#[instrument]` attribute references `tar_gz_data.len()` which will not compile after the signature change. Drop the `bytes` field from the instrument macro.

- [ ] **Step 1: Verify existing tests pass before touching anything**

```bash
cargo nextest run -p minibox image::layer::tests
```

Expected: all PASS.

- [ ] **Step 2: Change the signature and fix `#[instrument]`**

In `layer.rs`, change:

```rust
#[instrument(skip(tar_gz_data, dest), fields(bytes = tar_gz_data.len(), dest = %dest.display()))]
pub fn extract_layer(tar_gz_data: &[u8], dest: &Path) -> anyhow::Result<()> {
    debug!(
        "extracting layer ({} bytes) to {:?}",
        tar_gz_data.len(),
        dest
    );

    let gz = GzDecoder::new(tar_gz_data);
```

To:

```rust
#[instrument(skip(reader, dest), fields(dest = %dest.display()))]
pub fn extract_layer(reader: impl std::io::Read, dest: &Path) -> anyhow::Result<()> {
    debug!("extracting layer to {:?}", dest);

    let gz = GzDecoder::new(reader);
```

Everything else in the function body is unchanged.

- [ ] **Step 3: Verify it compiles and all tests pass**

```bash
cargo nextest run -p minibox image::layer::tests
```

Expected: all PASS. (Existing tests use `extract_layer(&tar_gz, dest.path())` where `&[u8]: Read` — they still compile.)

- [ ] **Step 4: Fix the `store_layer` caller in `mod.rs`**

`store_layer` in `crates/minibox/src/image/mod.rs` currently calls `extract_layer(&buf, &dest)` where `buf: Vec<u8>`. After the signature change `&buf` still implements `Read`, so no change needed here — but verify:

```bash
cargo check -p minibox
```

Expected: no errors.

- [ ] **Step 5: clippy + fmt + commit**

```bash
cargo fmt --all
cargo clippy -p minibox -- -D warnings
git add crates/minibox/src/image/layer.rs
git commit -m "refactor: extract_layer accepts impl Read instead of &[u8]"
```

---

## Task 5: Make `store_layer` atomic (temp-dir + rename)

**Files:**

- Modify: `crates/minibox/src/image/mod.rs`

Replace the `read_to_end` buffer + direct `extract_layer` call with: extract into `{digest_key}.tmp`, rename to final `{digest_key}` on success. This ensures `dest` is always either absent or fully written — never partially extracted. Handles duplicate-digest races idempotently.

- [ ] **Step 1: Write failing tests**

Add a `#[cfg(test)]` block at the bottom of `crates/minibox/src/image/mod.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{Compression, write::GzEncoder};
    use std::io::Write;
    use tar::{Builder, Header};
    use tempfile::TempDir;

    fn make_tar_gz(filename: &str, content: &[u8]) -> Vec<u8> {
        let gz = GzEncoder::new(Vec::new(), Compression::default());
        let mut ar = Builder::new(gz);
        let mut h = Header::new_gnu();
        h.set_path(filename).unwrap();
        h.set_size(content.len() as u64);
        h.set_mode(0o644);
        h.set_cksum();
        ar.append(&h, content).unwrap();
        ar.into_inner().unwrap().finish().unwrap()
    }

    #[test]
    fn store_layer_dest_exists_after_success() {
        let base = TempDir::new().unwrap();
        let store = ImageStore::new(base.path()).unwrap();
        let data = make_tar_gz("hello.txt", b"world");
        let digest = "sha256:aabbccdd";

        let dest = store
            .store_layer("library_alpine", "latest", digest, std::io::Cursor::new(&data))
            .unwrap();

        assert!(dest.exists(), "dest should exist after successful store");
        assert!(dest.join("hello.txt").exists(), "file should be inside dest");

        // tmp dir should be gone
        let tmp = dest.with_extension("tmp");
        assert!(!tmp.exists(), "tmp dir should be cleaned up");
    }

    #[test]
    fn store_layer_no_dest_after_bad_data() {
        let base = TempDir::new().unwrap();
        let store = ImageStore::new(base.path()).unwrap();

        // Invalid tar.gz data
        let err = store
            .store_layer("library_alpine", "latest", "sha256:bad", std::io::Cursor::new(b"not a tar"))
            .unwrap_err();

        assert!(
            err.to_string().to_lowercase().contains("extract") || err.to_string().contains("failed"),
            "expected extraction error, got: {err}"
        );

        // dest must not exist (no partial layer dir)
        let digest_key = "sha256_bad";
        let dest = store.base_dir
            .join("library_alpine")
            .join("latest")
            .join("layers")
            .join(digest_key);
        assert!(!dest.exists(), "dest must not exist after failed extraction");
    }

    #[test]
    fn store_layer_idempotent_if_dest_exists() {
        let base = TempDir::new().unwrap();
        let store = ImageStore::new(base.path()).unwrap();
        let data = make_tar_gz("f.txt", b"content");
        let digest = "sha256:idempotent";

        let dest1 = store
            .store_layer("img", "v1", digest, std::io::Cursor::new(&data))
            .unwrap();
        let dest2 = store
            .store_layer("img", "v1", digest, std::io::Cursor::new(&data))
            .unwrap();

        assert_eq!(dest1, dest2);
        assert!(dest1.exists());
    }
}
```

- [ ] **Step 2: Run tests to see current state**

```bash
cargo nextest run -p minibox image::mod::tests
```

Expected: `store_layer_no_dest_after_bad_data` FAILS (current code leaves the partially-created dir behind). Others may pass.

- [ ] **Step 3: Rewrite `store_layer` with atomic pattern**

In `crates/minibox/src/image/mod.rs`, replace the body of `store_layer` after the `if dest.exists()` early-return:

```rust
pub fn store_layer<R: std::io::Read>(
    &self,
    name: &str,
    tag: &str,
    digest: &str,
    data_reader: R,
) -> anyhow::Result<PathBuf> {
    let digest_key = digest.replace(':', "_");
    let dest = self.layers_dir(name, tag)?.join(&digest_key);

    if dest.exists() {
        debug!("layer {} already extracted at {:?}, skipping", digest, dest);
        return Ok(dest);
    }

    // Extract into a sibling temp directory, then atomically rename.
    // This ensures dest is always either absent or fully written.
    let tmp = dest.with_extension("tmp");

    // Clean up any stale tmp from a prior crash.
    if tmp.exists() {
        std::fs::remove_dir_all(&tmp).map_err(|source| ImageError::StoreWrite {
            path: tmp.display().to_string(),
            source,
        })?;
    }

    std::fs::create_dir_all(&tmp).map_err(|source| ImageError::StoreWrite {
        path: tmp.display().to_string(),
        source,
    })?;

    match extract_layer(data_reader, &tmp)
        .with_context(|| format!("extracting layer {digest} to {tmp:?}"))
    {
        Ok(()) => {
            match std::fs::rename(&tmp, &dest) {
                Ok(()) => {}
                Err(_) if dest.exists() => {
                    // Duplicate-digest race: another task finished first. Clean up our tmp.
                    std::fs::remove_dir_all(&tmp).ok();
                }
                Err(source) => {
                    std::fs::remove_dir_all(&tmp).ok();
                    return Err(ImageError::StoreWrite {
                        path: dest.display().to_string(),
                        source,
                    }
                    .into());
                }
            }
        }
        Err(e) => {
            std::fs::remove_dir_all(&tmp).ok(); // best-effort cleanup
            return Err(e);
        }
    }

    info!("stored layer {} at {:?}", digest, dest);
    Ok(dest)
}
```

Also remove the `mut` from `data_reader` in the parameter (it's no longer needed since we pass it directly to `extract_layer`). Remove the old `use std::io::Read;` if it was only used for `read_to_end`.

- [ ] **Step 4: Run all layer/store tests**

```bash
cargo nextest run -p minibox
```

Expected: all PASS.

- [ ] **Step 5: clippy + fmt + commit**

```bash
cargo fmt --all
cargo clippy -p minibox -- -D warnings
git add crates/minibox/src/image/mod.rs
git commit -m "refactor: store_layer uses atomic temp-dir+rename, removes read_to_end buffer"
```

---

## Task 6: Parallel `pull_image` with `JoinSet` + `Semaphore` + streaming

**Files:**

- Modify: `crates/minibox/src/image/registry.rs`

This is the main task. It:

1. Adds `LimitedStream` (private struct, ~25 lines)
2. Refactors `pull_layer` to return `reqwest::Response` instead of `Bytes`
3. Replaces the sequential `for` loop in `pull_image` with a `JoinSet` + `Arc<Semaphore>` parallel pipeline

Each layer task: acquire permit → check cache → HTTP GET → wrap in `LimitedStream` → `spawn_blocking(SyncIoBridge → HashingReader → store_layer)` → verify digest → log.

- [ ] **Step 1: Add imports at the top of `registry.rs`**

Add to the existing `use` block:

```rust
use crate::error::ImageError;
use crate::image::layer::HashingReader;
use pin_project_lite::pin_project;
use std::io;
use std::sync::Arc;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tokio_util::io::{StreamReader, SyncIoBridge};
use futures::stream::StreamExt as _;
```

Remove `use bytes::Bytes;` if it's only used by the old `pull_layer` return type (check first with `grep`).

- [ ] **Step 2: Add `LimitedStream`**

Add after the constants block (after `MAX_LAYER_SIZE`):

```rust
// ---------------------------------------------------------------------------
// LimitedStream
// ---------------------------------------------------------------------------

pin_project! {
    /// Wraps an async byte stream and returns an error if total bytes exceed `limit`.
    ///
    /// Used to enforce [`MAX_LAYER_SIZE`] during streaming download without
    /// buffering the full blob in memory.
    struct LimitedStream<S> {
        #[pin]
        inner: S,
        remaining: u64,
    }
}

impl<S> LimitedStream<S> {
    fn new(inner: S, limit: u64) -> Self {
        Self { inner, remaining: limit }
    }
}

impl<S: futures::Stream<Item = io::Result<bytes::Bytes>>> futures::Stream for LimitedStream<S> {
    type Item = io::Result<bytes::Bytes>;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        let this = self.project();
        match this.inner.poll_next(cx) {
            std::task::Poll::Ready(Some(Ok(chunk))) => {
                if chunk.len() as u64 > *this.remaining {
                    std::task::Poll::Ready(Some(Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!(
                            "layer exceeded size limit of {} bytes",
                            MAX_LAYER_SIZE
                        ),
                    ))))
                } else {
                    *this.remaining -= chunk.len() as u64;
                    std::task::Poll::Ready(Some(Ok(chunk)))
                }
            }
            std::task::Poll::Ready(Some(Err(e))) => std::task::Poll::Ready(Some(Err(e))),
            std::task::Poll::Ready(None) => std::task::Poll::Ready(None),
            std::task::Poll::Pending => std::task::Poll::Pending,
        }
    }
}
```

- [ ] **Step 3: Refactor `pull_layer` to return `reqwest::Response`**

Replace the entire `pull_layer` method with a version that performs the HTTP GET + status/content-length checks but returns the response instead of buffering:

```rust
/// Start a blob download and return the HTTP response for streaming.
///
/// Performs status and `Content-Length` header checks before returning.
/// The caller is responsible for enforcing the streaming byte limit via
/// [`LimitedStream`].
#[instrument(skip(self, token), fields(digest = &digest[..19]))]
async fn pull_layer(
    &self,
    name: &str,
    digest: &str,
    token: &str,
) -> anyhow::Result<reqwest::Response> {
    let url = format!("{REGISTRY_BASE}/{name}/blobs/{digest}");
    debug!("pulling blob {} from {}", digest, url);

    let resp = self
        .http
        .get(&url)
        .bearer_auth(token)
        .send()
        .await
        .map_err(RegistryError::Network)
        .with_context(|| format!("GET blob {digest}"))?;

    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let msg = resp.text().await.unwrap_or_else(|_| String::new());
        return Err(RegistryError::BlobFetch {
            digest: digest.to_owned(),
            message: format!("HTTP {status}: {msg}"),
        }
        .into());
    }

    // Advisory content-length check (streaming LimitedStream enforces the hard limit).
    if let Some(content_length) = resp.headers().get("content-length")
        && let Ok(size_str) = content_length.to_str()
        && let Ok(size) = size_str.parse::<u64>()
        && size > MAX_LAYER_SIZE
    {
        return Err(RegistryError::Other(format!(
            "layer too large per Content-Length: {size} bytes (max {MAX_LAYER_SIZE})"
        ))
        .into());
    }

    Ok(resp)
}
```

- [ ] **Step 4: Replace the sequential loop in `pull_image`**

In `pull_image`, remove everything between the "3. Download and store each layer." comment and "4. Persist manifest." and replace with:

```rust
// 3. Download and store each layer in parallel (bounded by MAX_CONCURRENT_LAYERS).
let total_layers = manifest.layers.len();
let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_LAYERS));
let mut join_set: JoinSet<anyhow::Result<()>> = JoinSet::new();

for (idx, layer_desc) in manifest.layers.iter().cloned().enumerate() {
    let client = self.clone();
    let store = store.clone();
    let token = token.clone();
    let sem = semaphore.clone();
    let name = name.to_owned();
    let tag = tag.to_owned();

    join_set.spawn(async move {
        let _permit = sem.acquire_owned().await.expect("semaphore closed");

        let digest_key = layer_desc.digest.replace(':', "_");
        let layer_dir = store
            .base_dir
            .join(name.replace('/', "_"))
            .join(&tag)
            .join("layers")
            .join(&digest_key);

        if layer_dir.exists() {
            info!(
                "layer {}/{}: {} (cached)",
                idx + 1,
                total_layers,
                &layer_desc.digest[..19]
            );
            return Ok(());
        }

        let layer_start = std::time::Instant::now();

        // Start the HTTP download (async).
        let response = client
            .pull_layer(&name, &layer_desc.digest, &token)
            .await
            .with_context(|| format!("pull layer {}", layer_desc.digest))?;

        // Bridge async stream → sync Read inside spawn_blocking.
        let limited_stream = LimitedStream::new(
            response
                .bytes_stream()
                .map(|r| r.map_err(|e| io::Error::new(io::ErrorKind::Other, e))),
            MAX_LAYER_SIZE,
        );
        let handle = tokio::runtime::Handle::current();
        let digest = layer_desc.digest.clone();

        tokio::task::spawn_blocking(move || {
            let sync_reader =
                SyncIoBridge::new_with_handle(StreamReader::new(limited_stream), handle);
            let mut hashing_reader = HashingReader::new(sync_reader);

            // Extract (streaming: bytes flow from HTTP → HashingReader → GzDecoder → tar).
            store
                .store_layer(&name, &tag, &digest, &mut hashing_reader)
                .with_context(|| format!("store layer {digest}"))?;

            // Verify digest against what we actually received.
            let bytes = hashing_reader.bytes_read();
            let actual = hashing_reader.finalize();
            let expected_hex = digest.strip_prefix("sha256:").ok_or_else(|| {
                anyhow::anyhow!("unexpected digest format: {digest}")
            })?;

            if actual != expected_hex {
                // store_layer used atomic rename so dest is gone or fully written by
                // another task. Remove it if we wrote it (best-effort).
                std::fs::remove_dir_all(&layer_dir).ok();
                return Err(ImageError::DigestMismatch {
                    digest: digest.clone(),
                    expected: expected_hex.to_owned(),
                    actual,
                }
                .into());
            }

            info!(
                "layer {}/{} ({}) done in {:.2?} — {} bytes",
                idx + 1,
                total_layers,
                &digest[..19],
                layer_start.elapsed(),
                bytes,
            );
            Ok(())
        })
        .await
        .map_err(|join_err| {
            anyhow::Error::from(RegistryError::LayerTask {
                digest: layer_desc.digest.clone(),
                message: join_err.to_string(),
            })
        })??
    });
}

// Collect results; abort remaining tasks on first error.
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
            }
            .into());
        }
    }
}
```

Also add after the constants:

```rust
const MAX_CONCURRENT_LAYERS: usize = 4;
```

- [ ] **Step 5: Verify it compiles**

```bash
cargo check -p minibox
```

If there are unused import warnings (e.g. `Bytes` no longer used directly in registry.rs), remove the relevant `use` lines and re-check. Fix any clippy issues now.

- [ ] **Step 6: Run all minibox tests**

```bash
cargo nextest run -p minibox
```

Expected: all PASS. This exercises `HashingReader`, `extract_layer`, `store_layer`, and all existing layer security tests.

- [ ] **Step 7: clippy + fmt**

```bash
cargo fmt --all
cargo clippy -p minibox -- -D warnings
```

Fix any warnings before committing.

- [ ] **Step 8: Commit**

```bash
git add crates/minibox/src/image/registry.rs
git commit -m "feat: parallel streaming layer pulls — JoinSet + Semaphore + SyncIoBridge"
```

---

## Final Verification

- [ ] **Full workspace check**

```bash
cargo check --workspace
cargo fmt --all --check
cargo clippy -p minibox -- -D warnings
cargo nextest run -p minibox
```

Expected: all pass, no warnings, no fmt diff.

- [ ] **Summarise timing improvement**

The pull log should now show layers downloading concurrently. With `RUST_LOG=info`, verify `layer N/M ... done` messages interleave rather than appearing strictly sequentially.
