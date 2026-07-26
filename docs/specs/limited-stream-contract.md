# LimitedStream Contract

**Issue**: #150
**Status**: Implemented — spec documents existing behaviour at
`crates/minibox-core/src/image/registry.rs`

---

## 1. Purpose

`LimitedStream<S>` is an async `Stream` adapter that enforces a hard byte ceiling on a
wrapped byte stream. Its primary use is to cap per-layer compressed blob size during OCI
image pulls, acting as a second line of defence after any `Content-Length` pre-check
performed by the caller.

The type lives in `crates/minibox-core/src/image/registry.rs` (module-private, used
only within that file). It is re-exported for tests via `super::super::LimitedStream`.

---

## 2. Struct Definition

```rust
// registry.rs:91-96
pub struct LimitedStream<S> {
    #[pin]
    inner: S,       // wrapped byte stream
    limit: u64,     // maximum bytes allowed (inclusive)
    consumed: u64,  // running total of bytes seen so far
}
```

| Field      | Type  | Purpose                                                  |
| ---------- | ----- | -------------------------------------------------------- |
| `inner`    | `S`   | The upstream stream, pinned so it can be polled in place |
| `limit`    | `u64` | Maximum byte count allowed (inclusive boundary)          |
| `consumed` | `u64` | Accumulates raw byte count across all chunks yielded     |

`consumed` is updated **before** the limit check, so it always reflects the total bytes
seen from the inner stream regardless of whether an error is returned.

### Constructor

```rust
pub fn new(inner: S, limit: u64) -> Self
```

Initialises `consumed` to zero.

### Inspector

```rust
pub fn consumed(&self) -> u64
```

Returns the running total of bytes consumed. Useful for diagnostics and for callers
that need to compare consumed bytes against a manifest-declared size after the stream
ends.

---

## 3. Size Enforcement Semantics

### 3.1 What bytes are counted

`LimitedStream` counts **raw (compressed) bytes on the wire** — the HTTP response body
bytes before any decompression. For gzip-encoded OCI layers this means the compressed
layer size, not the decompressed tar contents.

### 3.2 When the check fires

The limit is checked **per chunk**, immediately after the chunk length is added to
`consumed`. There is no deferred end-of-stream check. Formally:

```
consumed += chunk.len()
if consumed > limit  →  error
```

A chunk that pushes `consumed` past `limit` is rejected in the same poll call that
delivered that chunk. No bytes from the offending chunk are forwarded to the caller.

### 3.3 Boundary semantics (inclusive limit)

| `consumed` after chunk | Result              |
| ---------------------- | ------------------- |
| `< limit`              | `Ok(chunk)` yielded |
| `== limit`             | `Ok(chunk)` yielded |
| `> limit`              | `Err(InvalidData)`  |

Exactly `limit` bytes is **allowed**. `limit + 1` bytes triggers the error, even if
those extra bytes arrive in the same chunk as the byte at position `limit`.

This means a single chunk that straddles the boundary — containing both the last
permitted byte and at least one excess byte — is rejected entirely. The permitted bytes
within that chunk are **not** forwarded; the error is returned instead.

### 3.4 Global limit in production

The production caller (`RegistryClient::pull_image`) passes `MAX_LAYER_SIZE`:

```rust
// registry.rs:51-54
// MAX_LAYER_SIZE: individual compressed layer blobs; 10 GB allows large images while
// ...
const MAX_LAYER_SIZE: u64 = 10 * 1024 * 1024 * 1024; // 10 GiB
```

A `limit` of zero means no bytes are permitted. The first non-empty chunk triggers the
error immediately.

---

## 4. Error Type

When the limit is exceeded, `poll_next` returns:

```rust
Poll::Ready(Some(Err(io::Error::new(
    io::ErrorKind::InvalidData,
    format!(
        "layer stream exceeded size limit: {} bytes (max {})",
        consumed, limit
    ),
))))
```

- **Error kind**: `io::ErrorKind::InvalidData`
- **Message**: human-readable, includes the actual `consumed` count and the configured
  `limit`
- **Rationale for `InvalidData`**: the registry sent more data than the manifest
  declared or the policy allows; this is a data-validity violation, not a network
  transport error

The caller (the `StreamReader`/`SyncIoBridge` chain inside `spawn_blocking`) surfaces
this as an `io::Error` to `GzDecoder`, which propagates it upward as an `anyhow::Error`
from `extract_layer`.

### Error precedence

An `Err` yielded by the inner stream is forwarded **as-is** (via `E: Into<io::Error>`)
without a limit check. Inner stream errors take precedence over limit violations that
might have occurred simultaneously.

---

## 5. Interaction with `HashingReader`

The byte flow for a layer pull is (registry.rs:622-625):

```
HTTP response body
  └─ LimitedStream           (size cap, compressed bytes)
       └─ StreamReader        (bytes::Bytes → AsyncRead)
            └─ SyncIoBridge   (AsyncRead → sync Read, bridged into spawn_blocking)
                 └─ HashingReader   (SHA-256 over compressed bytes)
                      └─ GzDecoder
                           └─ tar::Archive → filesystem
```

`LimitedStream` wraps the **raw HTTP byte stream**. `HashingReader` wraps the
**sync reader** produced by `SyncIoBridge::new_with_handle(StreamReader::new(limited))`.

This ordering has two consequences:

1. **`HashingReader` hashes compressed bytes after the limit gate.** If `LimitedStream`
   rejects a chunk, `HashingReader` never sees those bytes, so its digest covers only
   the bytes that passed the size check.

2. **Drain-after-error is necessary for correct digest verification.** When
   `extract_layer` fails (e.g. bad gzip header), the caller drains remaining bytes
   through `HashingReader` so the digest covers the full compressed stream:

   ```rust
   // registry.rs:651-653
   if extract_result.is_err() {
       let _ = std::io::copy(&mut hashing_reader, &mut std::io::sink());
   }
   ```

   If `LimitedStream` already returned an error, `SyncIoBridge` will propagate an
   `io::Error` from the drain — this is expected and the return value is discarded.

---

## 6. Premature EOF

If the inner stream ends before the limit is reached, `LimitedStream` returns
`Poll::Ready(None)` — a normal end-of-stream signal. It does **not** check whether the
total bytes consumed equals any expected size; that check belongs to the caller
(`HashingReader` digest verification and any manifest-declared size comparison).

The `StreamReader` and `SyncIoBridge` layers surface a premature EOF as an unexpected-
EOF `io::Error` during `GzDecoder` decompression.

---

## 7. Edge Cases

| Scenario                                     | Expected behaviour                                        |
| -------------------------------------------- | --------------------------------------------------------- |
| All chunks well under limit                  | All chunks forwarded as `Ok`; stream ends normally        |
| Single chunk exactly equal to limit          | Chunk forwarded as `Ok`; next poll returns `None`         |
| Single chunk of `limit + 1` bytes            | `Err(InvalidData)` on the first poll                      |
| Two chunks summing to exactly limit          | Both forwarded as `Ok`; stream exhausted                  |
| Chunk straddles boundary (some bytes ok)     | Entire chunk rejected with `Err(InvalidData)`             |
| `limit == 0`, any non-empty chunk            | `Err(InvalidData)` on first poll                          |
| Inner stream yields `Err`                    | Error forwarded as-is; limit state is not consulted       |
| Premature EOF (stream ends before limit)     | `None` returned; upper layers surface unexpected-EOF      |
| `consumed()` called mid-stream               | Returns bytes seen so far, including any rejected chunk   |

---

## 8. Pre-Existing Content-Length Pre-Check

`LimitedStream` is a second-level cap. The first-level check occurs in
`pull_layer_response` (registry.rs:~466-488), which compares the `Content-Length`
response header against `MAX_LAYER_SIZE` and rejects the response before wrapping it in
`LimitedStream`. The two checks are independent; `LimitedStream` does not inspect
headers.

---

## 9. Test Plan

The tests below are required to fully exercise the contract. Tests in the `/// ---
Boundary tests for #150 ---` block in `registry.rs` (lines 1742-1805) cover items
1-6; items 7-10 are to be implemented as part of issue #152.

| # | Test name (proposed)                        | What it verifies                                          |
| - | ------------------------------------------- | --------------------------------------------------------- |
| 1 | `passes_chunks_under_limit`                 | All chunks forwarded when well under limit (exists)       |
| 2 | `errors_when_limit_exceeded`                | `InvalidData` error when limit exceeded (exists)          |
| 3 | `tracks_consumed_bytes`                     | `consumed()` increments correctly across chunks (exists)  |
| 4 | `exactly_limit_bytes_allowed`               | `consumed == limit` → `Ok` (exists, #150 boundary)        |
| 5 | `one_over_limit_errors`                     | `consumed == limit + 1` → `Err(InvalidData)` (exists)     |
| 6 | `inner_error_forwarded_before_limit_check`  | Inner `Err` forwarded as-is (exists)                      |
| 7 | `boundary_split_across_chunks`              | Two chunks summing to exactly limit both pass (exists)    |
| 8 | `zero_limit_rejects_first_byte`             | `limit == 0`, any non-empty chunk → immediate error (#152)|
| 9 | `chunk_straddles_boundary_rejected_whole`   | Chunk spanning boundary is rejected entirely (#152)       |
|10 | `consumed_reflects_rejected_chunk`          | `consumed()` includes bytes from the rejected chunk (#152)|

All `LimitedStream` tests are cross-platform (no Linux-only gating needed) because the
type has no OS dependencies.
