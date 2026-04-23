# Parallel Layer Pulls Design

`feat/parallel-layer-pulls` implements streaming parallel OCI layer downloads in
`crates/minibox-lib/` (now `crates/linuxbox/`). The branch was not merged because all 7
commits target the old crate path — porting to `linuxbox` is required before landing.

This document captures the design so the work can be ported forward.

---

## What Was Implemented

### New Dependencies (commit `63fcc48`)

Added to `minibox-lib/Cargo.toml` (port to `linuxbox/Cargo.toml`):

```toml
tokio-util = { version = "0.7", features = ["io"] }
pin-project-lite = "0.2"
```

### `RegistryError::LayerTask` (commit `b7b7ac7`)

New error variant wrapping `tokio::task::JoinError` from parallel layer tasks:

```rust
#[error("layer download task failed")]
LayerTask(#[from] tokio::task::JoinError),
```

### `HashingReader<R>` (commit `adb5f8c`, in `layer.rs`)

Transparent `Read` wrapper that computes SHA-256 of all bytes passing through it.
Wraps the **compressed** stream (before `GzDecoder`) so the hash matches the OCI
manifest digest, which covers the compressed blob.

```rust
pub struct HashingReader<R> {
    inner: R,
    hasher: Sha256,
    bytes_read: u64,
}

impl<R: std::io::Read> HashingReader<R> {
    pub fn new(inner: R) -> Self { ... }

    /// Returns hex-encoded SHA-256 of all bytes read.
    pub fn finalize(self) -> String { hex::encode(self.hasher.finalize()) }

    /// Total compressed bytes read.
    pub fn bytes_read(&self) -> u64 { self.bytes_read }
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

### `extract_layer` signature change (commit `4922212`)

Old: `fn extract_layer(data: &[u8], dest: &Path) -> Result<()>` — buffered entire layer in RAM.

New: `fn extract_layer(reader: &mut impl Read, dest: &Path) -> Result<()>` — streams from any
`Read`, no intermediate buffer.

### `store_layer` atomic temp-dir+rename (commit `2052c1f`)

Old: `read_to_end` buffered blob → wrote to final path.

New:
1. Creates `{dest}.tmp/` directory
2. Streams into tmp via `extract_layer`
3. On success, `fs::rename(tmp, dest)` — atomic commit
4. On error, `remove_dir_all(tmp)` — clean partial state
5. Race handling: if dest exists after rename fails, another task won — silently discard tmp

### Parallel streaming layer pulls (commit `2dd9465`)

The core change in `RegistryClient::pull_image`:

```rust
const MAX_CONCURRENT_LAYERS: usize = 4;

let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_LAYERS));
let mut join_set: JoinSet<anyhow::Result<()>> = JoinSet::new();

for (idx, layer_desc) in manifest.layers.iter().cloned().enumerate() {
    let sem = semaphore.clone();
    // ...clone client, store, token, name, tag...

    join_set.spawn(async move {
        let _permit = sem.acquire_owned().await.expect("semaphore closed");

        // Early-exit if layer is cached.
        let layer_dir = store.layer_path(&name, &tag, &digest)?;
        if layer_dir.exists() {
            return Ok(());
        }

        // HTTP GET the blob (async).
        let response = client.pull_layer(&name, &digest, &token).await?;

        // LimitedStream enforces MAX_LAYER_SIZE (10 GB) on the async stream.
        let limited_stream = LimitedStream::new(
            response.bytes_stream().map(|r| r.map_err(io::Error::other)),
            MAX_LAYER_SIZE,
        );
        let handle = tokio::runtime::Handle::current();

        // Bridge async → sync: extraction (tar + gzip) is synchronous.
        tokio::task::spawn_blocking(move || {
            let sync_reader = SyncIoBridge::new_with_handle(
                StreamReader::new(limited_stream),
                handle,
            );
            // Byte flow: HTTP → LimitedStream → SyncIoBridge → HashingReader → GzDecoder → tar
            let mut hashing_reader = HashingReader::new(sync_reader);

            // ... extract to tmp dir, verify digest, atomic rename (see above) ...
        }).await??;

        Ok(())
    });
}

// Drain join_set — all tasks must succeed.
while let Some(result) = join_set.join_next().await {
    result.map_err(RegistryError::LayerTask)??;
}
```

### `LimitedStream<S>` (commit `2dd9465`, in `registry.rs`)

`pin_project!` wrapper around an async byte stream. Returns `io::ErrorKind::InvalidData`
if cumulative bytes exceed the limit. Used on the async side (before `SyncIoBridge`) so
the limit is enforced without blocking.

### Digest verify before atomic rename (commit `bc7a764`)

After extraction succeeds, verify the compressed-stream digest against the manifest:

```rust
let actual = hashing_reader.finalize();
let expected_hex = digest.strip_prefix("sha256:")?;
if actual != expected_hex {
    fs::remove_dir_all(&tmp).ok();
    return Err(ImageError::DigestMismatch { digest, expected: expected_hex, actual });
}
// Only now: fs::rename(tmp, dest)
```

---

## Byte Pipeline

```
HTTP response bytes
    → LimitedStream<S>         (async; enforces MAX_LAYER_SIZE)
    → StreamReader             (tokio-util; wraps stream as AsyncRead)
    → SyncIoBridge             (tokio-util; bridges AsyncRead → sync Read, using Handle for wakeups)
    → HashingReader<R>         (sync; SHA-256 over compressed bytes)
    → GzDecoder                (flate2; decompresses)
    → tar::Archive             (extracts into tmp dir)
```

---

## Files to Port

All changes live in `crates/minibox-lib/`. Port to these paths in `crates/linuxbox/`:

| Branch file                              | Port to                                        |
|------------------------------------------|------------------------------------------------|
| `minibox-lib/Cargo.toml`                 | `linuxbox/Cargo.toml` (add deps)               |
| `minibox-lib/src/error.rs`               | `linuxbox/src/error.rs` (`RegistryError::LayerTask`) |
| `minibox-lib/src/image/layer.rs`         | `linuxbox/src/image/layer.rs` (`HashingReader`) |
| `minibox-lib/src/image/mod.rs`           | `linuxbox/src/image/mod.rs`                    |
| `minibox-lib/src/image/registry.rs`      | `linuxbox/src/image/registry.rs` (parallel pull + `LimitedStream`) |

The `linuxbox` versions of these files have evolved significantly since the branch diverged —
the port is an integration, not a diff-apply.

### Key integration notes

- `linuxbox/src/image/registry.rs` is 1141 lines vs branch's 563-line `minibox-lib` version.
  The `pull_image` function needs to be replaced wholesale with the parallel version.
- `layer_path()` in `ImageStore` must return a `Result` (branch already does this via
  `store.layer_path(...)?`) — verify the current `linuxbox` `ImageStore` API.
- `extract_layer` signature change (`&[u8]` → `impl Read`) may have callers in test code that
  need updating.
- The `linuxbox` error module may already have `DigestMismatch` — check before adding.

---

## Related

- `crates/linuxbox/src/image/registry.rs` — current pull implementation (sequential)
- `crates/linuxbox/src/image/layer.rs` — current extraction (no `HashingReader` yet)
- `feat/parallel-layer-pulls` branch — preserved, not deleted; contains full implementation
