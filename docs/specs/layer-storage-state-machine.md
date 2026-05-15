# Layer Storage State Machine for Parallel Pulls

Issue: #149

This document specifies the path semantics, state transitions, cache-hit logic, concurrency
behavior, size enforcement, and digest verification for the parallel OCI layer pull
implementation in `minibox-core`.

Primary source files:

- `crates/minibox-core/src/image/registry.rs` — parallel pull orchestration
- `crates/minibox-core/src/image/mod.rs` — `ImageStore`, `store_layer`, `store_layer_verified`
- `crates/minibox-core/src/image/layer.rs` — `HashingReader`, `extract_layer`

---

## §1 — Directory Layout

### Path structure

The image store root is supplied by the caller at `ImageStore::new(base_dir)`. All paths below
are relative to `base_dir`.

```
{base_dir}/
  {safe_name}/              # image name with '/' replaced by '_'
    {tag}/
      manifest.json         # OCI manifest (written last, after all layers)
      layers/
        {digest_key}/       # final layer dir — "Complete" state
        {digest_key}.tmp/   # in-progress dir — "Downloading" state
```

`safe_name` is `name.replace('/', "_")`. For example, `library/ubuntu` becomes
`library_ubuntu`. `digest_key` is `digest.replace(':', "_")`, so `sha256:<hex>` becomes
`sha256_<hex>`.

Example absolute paths for `library/ubuntu:22.04`, layer digest
`sha256:abc123...`:

```
/var/lib/minibox/images/library_ubuntu/22.04/manifest.json
/var/lib/minibox/images/library_ubuntu/22.04/layers/sha256_abc123.../   # Complete
/var/lib/minibox/images/library_ubuntu/22.04/layers/sha256_abc123....tmp/  # Downloading
```

Source: `registry.rs:577-582`, `mod.rs:204-205`, `mod.rs:259`.

### Parent directory creation

- `layers/` itself is never explicitly `create_dir_all`'d by `store_layer_verified`. The tmp
  dir is created with `create_dir_all`, which creates the `layers/` parent on first call.
  (`mod.rs:267`; `registry.rs:642`).
- In `pull_image`, `layer_dir` is constructed directly from `store.base_dir` without calling
  `layers_dir()` — the `create_dir_all(&tmp_dir)` call at `registry.rs:642` also creates
  `layers/` and all ancestors as a side effect.
- `ImageStore::new` only creates `base_dir` itself (`mod.rs:41`). All subdirectories under
  it are created on demand.

### Same-filesystem assumption

`std::fs::rename` is used for the atomic tmp→dest promotion (`registry.rs:694`,
`mod.rs:317`). This requires that `{digest_key}.tmp` and `{digest_key}` reside on the same
filesystem, which is guaranteed because both are children of the same `layers/` directory.
Cross-device renames would fail with `EXDEV`; no fallback copy-then-delete is implemented.

---

## §2 — Layer Lifecycle States

### State definitions

| State          | Files on disk                       | Description                                              |
| -------------- | ----------------------------------- | -------------------------------------------------------- |
| `Absent`       | Neither `dest/` nor `dest.tmp/`     | Layer has never been pulled, or was fully cleaned up.    |
| `Downloading`  | `dest.tmp/` exists; `dest/` absent  | Extraction in progress (or process died mid-pull).       |
| `Complete`     | `dest/` exists; `dest.tmp/` absent  | Layer extracted, digest verified, rename committed.      |
| `Failed`       | `dest.tmp/` may exist; `dest/` absent | Extraction or digest check failed; cleanup attempted.  |

### Transition table

```
Absent
  → Downloading  trigger: create_dir_all(tmp_dir)               registry.rs:642
                 invariant: tmp_dir did not exist (stale one removed first at registry.rs:639)

Downloading
  → Failed       trigger: extract_layer error OR digest mismatch
                 invariant: remove_dir_all(tmp_dir) attempted (best-effort, warn on failure)
                            registry.rs:666-678, registry.rs:683-690

Downloading
  → Complete     trigger: digest OK, extract OK, rename(tmp_dir, layer_dir) succeeds
                 invariant: rename is atomic on POSIX — dest either does or does not exist,
                            no partial state observable by readers
                            registry.rs:694-703

Downloading (race)
  → Complete     trigger: rename fails with EEXIST-equivalent AND layer_dir.exists()
                 invariant: concurrent winner already completed the rename; loser discards
                            its tmp_dir and returns Ok(layer_dir)
                            registry.rs:695-698, mod.rs:318-321

Failed
  → Absent       trigger: remove_dir_all(tmp_dir) succeeds during failure cleanup
  → Downloading  trigger: next pull attempt; stale tmp_dir detected and removed at entry
                 invariant: registry.rs:638-641 explicitly removes any pre-existing tmp_dir
                            before starting a fresh download
```

---

## §3 — Cache Hit Semantics

### Is `dest.exists()` a valid cache-hit signal?

Yes, with an important caveat: the check is performed before acquiring the semaphore in
`pull_image` (`registry.rs:587-595`) and before the `HashingReader`/`GzDecoder` pipeline is
set up. It is also the sole check in `store_layer` (`mod.rs:207-209`) and `store_layer_verified`
(`mod.rs:250-257`).

The cache-hit check is `dest.exists()` (not `dest.is_dir()` or any content check). It signals
that the layer reached `Complete` state, because:

1. The final `layer_dir` is only created by `rename(tmp_dir, layer_dir)`.
2. `rename` is atomic: either it completes (layer is valid and fully extracted) or it does not
   (layer remains in `Downloading`).
3. Therefore, if `layer_dir.exists()` is `true`, the layer was fully extracted and its digest
   verified before the rename.

There is no secondary validity check (e.g. checksum re-verification of the cached contents).

### What happens after a crash mid-download?

If the process dies while `dest.tmp/` exists, the next pull invocation finds it as a stale
tmp dir. Both `pull_image` (`registry.rs:638-641`) and `store_layer_verified` (`mod.rs:262-265`)
explicitly check for and remove any pre-existing tmp dir before starting extraction:

```rust
if tmp_dir.exists() {
    std::fs::remove_dir_all(&tmp_dir)
        .with_context(|| format!("remove stale tmp {}", tmp_dir.display()))?;
}
```

The stale tmp dir is removed unconditionally — no attempt is made to resume a partial
extraction. If `remove_dir_all` fails, the pull aborts with an error.

### Is there a validity check beyond existence?

No. Once `layer_dir.exists()` returns `true`, the layer is treated as valid without re-hashing.
The implementation trusts the POSIX rename atomicity guarantee: a directory at `layer_dir` was
only ever created by a successful `rename` from a `HashingReader`-verified tmp dir.

[PROPOSED] Future hardening could store a `.digest` sentinel file inside `layer_dir` at rename
time and re-verify it on cache hit. This would defend against manual tampering or filesystem
corruption but is not currently implemented.

---

## §4 — Concurrent Pull Behavior

### Semaphore

`pull_image` creates one `Arc<Semaphore>` per image pull with `MAX_CONCURRENT_LAYERS = 4`
permits (`registry.rs:553`). Each layer task acquires one permit before starting the HTTP
request. This bounds the number of simultaneously active HTTP connections, not the total number
of spawned tasks.

### Same-digest concurrent pull

Two tasks pulling the same digest within the same `pull_image` call: this cannot happen in
the current implementation because `manifest.layers` is iterated once (`registry.rs:557`) and
each unique `layer_desc` spawns exactly one task. Manifest layers can repeat the same digest
(shared base layers in multi-image pulls from different callers) — this is the relevant race.

When two independent callers call `pull_image` for images sharing a layer digest:

1. Both tasks observe `!layer_dir.exists()` and proceed to download.
2. Both create their own `tmp_dir` (same path: `{digest_key}.tmp`).
3. One task's `remove_dir_all` of the stale tmp dir may race with the other's `create_dir_all`.
   [PROPOSED] This is an unmitigated race — no lock or advisory file prevents two concurrent
   processes from interfering on the tmp dir. The current design assumes single-daemon
   operation (one `pull_image` at a time per store).
4. The first to call `rename(tmp_dir, layer_dir)` wins.
5. The second's `rename` fails; it checks `layer_dir.exists()` (`registry.rs:695-698`):
   - If `true`: discards its own tmp dir (`remove_dir_all`) and returns `Ok(layer_dir)`.
   - If `false`: the rename failed for another reason; cleanup tmp and return `Err`.

This is a check-then-act sequence, not an atomic operation. Between the failing `rename` and
the `layer_dir.exists()` check, a third event (e.g. `delete_image`) could remove `layer_dir`.
In that edge case the second task would fall through to the error arm and return a rename error.
[PROPOSED] This window is narrow and acceptable for single-daemon deployments; multi-daemon
shared-store deployments would need a lock (e.g. fcntl/flock on a `.lock` file).

### Within a single `pull_image` call

Because `manifest.layers` drives spawning and each digest is spawned once, the same tmp path
is never created twice within one `pull_image`. There is no intra-call same-digest race.

---

## §5 — Size Enforcement

### What does `LimitedStream` limit?

`LimitedStream` counts **compressed bytes on the wire** — the raw HTTP response body bytes
before decompression by `GzDecoder`. This is explicitly documented in the struct contract:

> "What is counted: raw bytes on the wire (compressed), not decompressed tar contents."

Source: `registry.rs:69-73`.

The limit constant is `MAX_LAYER_SIZE = 10 GiB` (`registry.rs:54`).

### Content-Length pre-check

Before wrapping the response in `LimitedStream`, `pull_layer_response` checks the
`Content-Length` header:

```rust
if let Some(content_length) = resp.headers().get("content-length")
    && let Ok(size_str) = content_length.to_str()
    && let Ok(size) = size_str.parse::<u64>()
    && size > MAX_LAYER_SIZE
{
    return Err(RegistryError::Other(...).into());
}
```

Source: `registry.rs:462-472`.

This check uses **short-circuit evaluation**:
- If `Content-Length` is absent: no rejection (the check is skipped entirely).
- If `Content-Length` is present but unparseable: no rejection.
- If `Content-Length` is present, parseable, and `> MAX_LAYER_SIZE`: rejected immediately,
  before any bytes are streamed.

### What if Content-Length is missing, incorrect, or mismatched?

- **Missing**: `LimitedStream` is the only guard. Download proceeds and is capped at the
  streaming limit.
- **Incorrect (claims small, sends large)**: `LimitedStream` catches this — it counts actual
  bytes received, not the declared size.
- **Incorrect (claims large, sends small)**: `LimitedStream` never fires; the stream ends at
  `Poll::Ready(None)`. `StreamReader`/`SyncIoBridge` surface the premature EOF during gzip
  decompression or tar reading.

### Is exactly `MAX_LAYER_SIZE` bytes allowed?

Yes. The boundary condition is `consumed > limit` → error (`registry.rs:130`). A layer of
exactly `MAX_LAYER_SIZE` bytes is allowed; `MAX_LAYER_SIZE + 1` bytes triggers the error.

### Error precedence

`LimitedStream` yields the inner stream's error as-is when the inner stream itself returns an
`Err` (`registry.rs:127`). The byte-count check is only applied to `Ok(chunk)` cases. So:

- **Inner error wins** when the underlying stream fails before the limit is exceeded.
- **Limit error wins** when a chunk causes `consumed > limit`; inner errors on subsequent
  polls are never observed because the caller should drop the stream.
- **Content-Length rejection** (`pull_layer_response`) happens before `LimitedStream` is
  created; it takes precedence over everything.

Precedence order (highest to lowest):
1. Content-Length header exceeds `MAX_LAYER_SIZE` → rejected in `pull_layer_response`.
2. Inner stream `io::Error` → forwarded immediately.
3. `consumed > MAX_LAYER_SIZE` after a chunk → `InvalidData` error from `LimitedStream`.

---

## §6 — Digest Verification

### When is the digest checked?

Digest verification happens **after extraction** but **before the atomic rename**. The
`HashingReader` is placed around the compressed stream (between `SyncIoBridge` and
`GzDecoder`), so it accumulates a SHA-256 of the **compressed** blob bytes as they are read
during extraction.

Byte flow (`registry.rs:619-625`):

```
HTTP response
  → LimitedStream           (wire-byte cap)
  → StreamReader            (async → sync boundary)
  → SyncIoBridge            (async → sync boundary)
  → HashingReader           (SHA-256 of compressed bytes)
  → GzDecoder               (gzip decompression)
  → tar::Archive / extract_layer
```

If `extract_layer` returns an error partway through, the remaining compressed bytes are
drained into `std::io::sink()` so the `HashingReader` covers the full blob
(`registry.rs:651-653`). The digest is then finalized and compared:

```rust
let actual_hex = hashing_reader.finalize();
let expected_hex = digest_owned.strip_prefix("sha256:").ok_or_else(...)? ;
let digest_ok = actual_hex == expected_hex;
```

Source: `registry.rs:657-663`.

### What happens on digest mismatch?

On mismatch:

1. `remove_dir_all(tmp_dir)` is attempted (best-effort; a warning is logged if it fails)
   `registry.rs:666-678`.
2. `ImageError::DigestMismatch { digest, expected, actual }` is returned.
3. `layer_dir` is never created — the rename never runs.

On extraction error (when the digest matched):

1. `remove_dir_all(tmp_dir)` is attempted (`registry.rs:683-690`).
2. The extraction error is returned with context.

Digest mismatch is checked before surfacing an extraction error (`registry.rs:663-678` runs
before `registry.rs:681-691`). This is intentional: a corrupted or truncated blob causes both
a gz/tar error and a digest mismatch, and the mismatch is the more actionable root cause.

### Summary of cleanup guarantees

| Outcome                     | tmp_dir after return        | layer_dir after return |
| --------------------------- | --------------------------- | ---------------------- |
| Digest mismatch             | Removed (best-effort)       | Never created          |
| Extraction error            | Removed (best-effort)       | Never created          |
| Success                     | Does not exist (renamed)    | Exists, complete       |
| Rename race (loser path)    | Removed                     | Exists (created by winner) |
| Rename fails, dir missing   | Removed (best-effort)       | Never created          |

---

## Implementation Gaps and Proposed Improvements

| Gap | Location | Proposed resolution |
| --- | -------- | ------------------- |
| No re-verification of cached layer contents | `registry.rs:587`, `mod.rs:250` | Store a `.digest` sentinel file at rename time; check it on cache hit. |
| Concurrent multi-daemon tmp dir races | `registry.rs:638-643` | Use `fcntl`/`flock` on a per-digest `.lock` file before creating tmp dir. |
| Stale tmp dir removal may fail silently | `registry.rs:639-641` | Current behavior: removal failure aborts the pull with an error. This is correct but should be documented in the error message. |
| No EXDEV fallback for cross-device rename | `registry.rs:694` | Document the same-filesystem assumption explicitly in `ImageStore::new` docs; add a runtime assert or early check. |
| `store_layer` (non-verified path) has no tmp dir | `mod.rs:197-222` | `store_layer` writes directly to `dest/` with no atomicity. A partial extraction leaves a corrupt `dest/`. Only `store_layer_verified` and `pull_image` use the tmp/rename pattern. This is a correctness gap for any caller using `store_layer` directly. |
