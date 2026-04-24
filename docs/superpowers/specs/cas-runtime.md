# CAS-Backed Container Runtime

**Status:** Draft
**Author:** Joseph O'Brien
**Date:** 2026-04-16
**Issue:** minibox-69

---

## Overview

This spec describes a content-addressed storage (CAS) backend for minibox image management,
replacing the current per-layer extracted-directory model. Each file is stored once under its
SHA-256 digest. Images are represented as verified tree objects covering the full file metadata
(mode, uid, gid, xattrs), making the root tree digest the locally-stable identity of a runnable
image. The design is OCI registry-compatible and is introduced as a zero-flag-day adapter
(`MINIBOX_ADAPTER=cas`) alongside the existing `native` and `colima` paths.

The design draws from composefs (verified overlay mounts), containerd's content store
(blob-addressed storage), and OCI image layout (registry interop). The goal is not to replicate
composefs exactly; minibox targets a simpler self-contained implementation that eliminates the
cross-image redundancy of extracted tar layers while preserving the hexagonal adapter contract.

---

## Goals

- **File-granular deduplication** across images. Two images sharing a file store one blob.
- **Root tree digest as image identity.** A stable, content-addressed label for any locally
  runnable image. Used for caching, GC accounting, and future signing.
- **Verified mounts.** The tree digest covers metadata (mode/uid/gid/xattrs), not just content.
  A container's root cannot be silently modified without breaking the digest.
- **OCI registry compatibility preserved.** Pull/push uses the existing OCI image manifest and
  layer blob format. CAS is a local transform of what the registry delivers.
- **Zero flag-day migration.** `MINIBOX_ADAPTER=cas` is an additive arm. Existing
  `native`/`colima`/`vz` paths are unaffected. No data migration required.

---

## Non-Goals (for this spec)

- Signed tree metadata / attestations (Sigstore, cosign).
- Lazy or remote-backed blobs (fetch-on-demand, FUSE overlays).
- idmapped mounts (UID/GID remapping without privilege).
- Multi-platform image index handling.
- P2P blob distribution.

---

## Object Model

### Four object types

**BlobObject**
Raw file content, keyed by SHA-256 of the bytes. No metadata.

```
/var/lib/minibox/cas/blobs/<hex64>
```

**TreeObject**
A serialized sorted list of `TreeEntry` records. Each entry covers one path component: name,
kind (regular/symlink/dir/hardlink), mode, uid, gid, xattrs, and either a blob digest (for
regular files) or inline value (for symlinks). The tree is recursively hashed — subdirectory
entries carry a tree digest, not a blob digest. This is analogous to a git tree object.

```
/var/lib/minibox/cas/trees/<hex64>
```

TreeObjects are serialized as length-prefixed CBOR or a compact JSON-newline format (TBD in
Phase 2). The format must be stable and deterministic.

**RootRef**
A file whose name is the image reference (`<registry>/<repo>:<tag>@<digest>` normalized) and
whose content is a tree digest hex string. Maps a human-readable image name to the local CAS
identity.

```
/var/lib/minibox/cas/refs/<normalized-ref>
```

**PartialTreeObject** (optional, see Open Questions)
A per-layer intermediate tree, stored transiently during import. Useful for debugging and
potential lazy-fetch extensions. Not required for Phase 1.

### Metadata in hash boundary

Metadata (mode, uid, gid, xattrs) is included in the TreeEntry serialization and therefore
covered by the tree digest. This means two images with identical file content but different
ownership produce different tree digests. This is intentional: verified mounts require the
entire filesystem state to be covered.

mtime is excluded from the hash boundary (see Open Questions). OCI layers do not guarantee
reproducible mtimes, and including mtime would break deduplication for identical content
installed at different times.

### Storage layout

```
/var/lib/minibox/cas/
  blobs/
    <sha256-hex>         # raw file bytes, immutable once written
  trees/
    <sha256-hex>         # serialized TreeObject, immutable once written
  refs/
    <normalized-ref>     # contains a tree digest hex string
```

Blobs and trees are written atomically (write to temp, rename). Both are immutable after
creation; GC is the only deletion mechanism.

---

## Domain Traits (Ports)

These traits belong in `minibox-core/src/domain.rs`. They must not import any Linux-specific or
platform-specific types.

### Types

```rust
/// SHA-256 hex digest of a blob (raw file content).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BlobDigest(pub String);

/// SHA-256 hex digest of a TreeObject.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TreeDigest(pub String);

/// A single entry in a tree object.
#[derive(Debug, Clone)]
pub struct TreeEntry {
    pub name: String,
    pub kind: EntryKind,
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
    pub xattrs: Vec<(String, Vec<u8>)>,
    /// Blob digest for regular files; tree digest for directories; symlink target for symlinks.
    pub content: EntryContent,
}

#[derive(Debug, Clone)]
pub enum EntryKind {
    Regular,
    Directory,
    Symlink,
    Hardlink,
}

#[derive(Debug, Clone)]
pub enum EntryContent {
    Blob(BlobDigest),
    Tree(TreeDigest),
    SymlinkTarget(String),
    HardlinkTarget(String), // path within the tree
}

/// Overlay mount parameters returned by SnapshotterProvider::prepare.
#[derive(Debug, Clone)]
pub struct SnapshotMount {
    pub lower: Vec<PathBuf>,  // read-only lower dirs, bottom-to-top order
    pub upper: PathBuf,       // read-write upper dir
    pub work: PathBuf,        // overlayfs work dir
}
```

### ContentStore

```rust
pub trait ContentStore: Send + Sync {
    /// Return true if a blob with the given digest exists in the store.
    fn has_blob(&self, digest: &BlobDigest) -> Result<bool>;

    /// Write bytes into the store. Returns the digest.
    /// If the blob already exists, is a no-op and returns the existing digest.
    fn put_blob(&self, data: &[u8]) -> Result<BlobDigest>;

    /// Return the path to a blob file for use as an overlayfs lower dir source.
    /// The returned path is valid as long as the ContentStore is alive and no GC occurs.
    fn blob_path(&self, digest: &BlobDigest) -> Result<PathBuf>;

    /// Write a TreeObject. Returns the digest.
    /// If the tree already exists, is a no-op.
    fn put_tree(&self, entries: &[TreeEntry]) -> Result<TreeDigest>;

    /// Read a TreeObject by digest.
    fn get_tree(&self, digest: &TreeDigest) -> Result<Vec<TreeEntry>>;
}
```

### SnapshotterProvider

```rust
pub trait SnapshotterProvider: Send + Sync {
    /// Prepare an overlay snapshot for a container.
    /// Returns mount parameters; caller is responsible for mounting.
    fn prepare(
        &self,
        tree: &TreeDigest,
        container_id: &str,
    ) -> Result<SnapshotMount>;

    /// Remove all snapshot state for a container.
    fn remove(&self, container_id: &str) -> Result<()>;
}
```

### LayerImporter

```rust
pub trait LayerImporter: Send + Sync {
    /// Import OCI image layers (as tar byte streams, bottom-to-top order) into the ContentStore,
    /// apply whiteout semantics, and return the root TreeDigest of the merged filesystem.
    fn import_layers(
        &self,
        store: &dyn ContentStore,
        layers: &mut [Box<dyn std::io::Read>],
    ) -> Result<TreeDigest>;
}
```

---

## Import Algorithm

### Layer application (whiteout-correct)

OCI layers are applied bottom-to-top. Each layer is a tar archive. The algorithm builds a
mutable in-memory working tree, then materializes it into the ContentStore.

```
working_tree = empty

for layer in layers (bottom to top):
    for entry in tar_entries(layer):
        path = validate_layer_path(entry.path)?

        if filename == ".wh..wh..opq":
            # opaque whiteout: replace containing directory with empty
            parent_dir = path.parent()
            working_tree.replace_dir(parent_dir, empty)

        elif filename starts with ".wh.":
            # explicit whiteout: remove named entry
            target = path.parent().join(filename[4..])
            working_tree.remove(target)

        else:
            # upsert: last writer wins
            working_tree.upsert(path, entry)

root_digest = materialize(working_tree, store)
```

### Materialization

`materialize(working_tree, store) -> TreeDigest`:

1. Walk the working tree depth-first.
2. For each regular file: `store.put_blob(file_bytes)` → BlobDigest.
3. For each symlink: EntryContent::SymlinkTarget(target).
4. For each directory: recursively materialize children, then `store.put_tree(child_entries)` →
   TreeDigest.
5. Compute the root TreeDigest from the top-level entries.

### RootRef write

After import, write `store.refs/<normalized-image-ref>` containing the root TreeDigest. This
makes the image locally runnable.

---

## Snapshotter Strategy

### Two adapters under MINIBOX_ADAPTER

`native` (existing) — unchanged. Uses current overlayfs-from-extracted-dirs behavior. No CAS
involvement.

`cas` (new) — on `prepare(tree, container_id)`:

1. Walk the TreeObject recursively, collect all BlobDigests reachable from the tree.
2. Each blob is already a file at `cas/blobs/<hex>`. These become the overlayfs lower dirs.
3. Create container-specific `upper/` and `work/` directories.
4. Return `SnapshotMount { lower: blob_paths, upper, work }`.

The caller in `handler.rs` performs the actual mount syscall, as today.

### FilesystemProvider relationship

The existing `FilesystemProvider` trait (overlay mount, pivot_root) stays in place.
`CasFilesystem` is a new implementation that satisfies `FilesystemProvider` and internally
delegates to a `SnapshotterProvider` for mount parameter construction.

---

## Migration Path

### Phase 1: Linux, root, FsContentStore

- `FsContentStore` + `OciLayerImporter` + `OverlayCasSnapshotter`
- Activated by `MINIBOX_ADAPTER=cas`
- Requires root (overlayfs mount)
- Stores blobs/trees at `/var/lib/minibox/cas/` (respects `MINIBOX_DATA_DIR`)

### Phase 2: macOS via Colima

- `ColimaSnapshotter` — materializes blob dirs into Colima VM via nerdctl volume or direct path
- Shares the same `ContentStore` and `LayerImporter` implementations
- `MINIBOX_ADAPTER=cas` on macOS selects ColimaSnapshotter instead of OverlayCasSnapshotter

### Phase 3: Signed tree metadata (out of scope)

- TreeDigest becomes signable (Sigstore bundle, cosign)
- RootRef includes signature alongside digest

---

## Crate Placement

| Component                                                                             | Location                      |
| ------------------------------------------------------------------------------------- | ----------------------------- |
| `BlobDigest`, `TreeDigest`, `TreeEntry`, `EntryKind`, `EntryContent`, `SnapshotMount` | `minibox-core/src/domain.rs`  |
| `ContentStore`, `SnapshotterProvider`, `LayerImporter` traits                         | `minibox-core/src/domain.rs`  |
| `FsContentStore`                                                                      | `minibox/src/cas/store.rs`    |
| `OciLayerImporter`                                                                    | `minibox/src/cas/importer.rs` |
| `OverlayCasSnapshotter`                                                               | `minibox/src/adapters/cas.rs` |
| `ColimaSnapshotter`                                                                   | `macbox/src/adapters/cas.rs`  |
| Wiring / adapter selection                                                            | `miniboxd/src/main.rs`        |

The `minibox/src/cas/` module is Linux-only (`#[cfg(target_os = "linux")]`). `minibox-core` must
remain platform-neutral.

---

## Open Questions

1. **PartialTreeObjects per layer.** Should intermediate per-layer tree objects be persisted to
   `cas/trees/` for debugging and potential lazy-access support, or computed and discarded?
   Pro: enables future lazy fetch and per-layer diff tooling. Con: increases store size and GC
   complexity. Decision deferred to Phase 2.

2. **GC and active mounts.** A running container holds a reference to a set of blob paths via
   its `SnapshotMount`. GC must not collect blobs with active mounts. Options: (a) lease files
   written per-mount, removed on cleanup; (b) reference counting in a sidecar file; (c) scan
   active containers before GC. Lease files are the simplest and most crash-recoverable.

3. **Metadata canonicalization: mtime.** OCI layers do not guarantee reproducible mtimes.
   Including mtime in the hash boundary would break cross-image deduplication for identical
   content. Current decision: exclude mtime. This means `check-drift.sh` cannot detect mtime
   changes — acceptable given the use case.

4. **TreeObject serialization format.** CBOR (compact, schema-free) or JSON-newline (debuggable,
   grep-friendly)? JSON-newline is preferred for the initial implementation; CBOR can be adopted
   if tree hashing becomes a measurable bottleneck.
