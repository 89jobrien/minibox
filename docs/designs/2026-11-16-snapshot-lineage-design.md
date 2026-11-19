# Design: Snapshot Lineage and the VZ Checkpoint Backend

**Status**: proposed. Brainstorm approved 2026-11-16 (content-addressed manifest DAG; VZ as the
first real backend). Not yet implemented.

**Closes**: `TODO(feature-idea-31)` (`crates/minibox-domain/src/checkpoint.rs:11`) and
`TODO(feature-idea-29)` (`crates/minibox/src/daemon/handler/snapshot.rs:11`).

## Goal

Give minibox versioned, immutable, tamper-evident VM snapshots with parent lineage, and ship the
first real `VmCheckpoint` implementation (macOS Virtualization.framework) so the port stops being
a stub.

## Approved Approach

**Content-addressed manifest DAG.** A snapshot ID is the SHA-256 of its canonical manifest, which
records the parent ID and the length and digest of every payload. Publication is root-last: the
payloads and manifest are written in a staging directory, which is atomically renamed to the ID.

## Problem

1. `VmCheckpoint` has no implementation. Every wiring site installs `NoopVmCheckpoint`
   (`crates/miniboxd/src/main.rs`, `crates/macbox/src/lib.rs:133,416,626`).
2. `SnapshotInfo` has no snapshot identity, parent, or digest, so snapshots cannot form a
   lineage and cannot be verified.
3. **A path defect is live today.** `handle_save_snapshot` runs `create_dir_all` on a path built
   from the raw request `id` before it calls the checkpoint port, and `handle_restore_snapshot`
   joins the raw `name` into a path. The no-op backend does not prevent this: a root-only request
   with `id = "../../x"` creates directories outside the snapshot root today. This violates the
   path-validation rule in `.claude/rules/rust-patterns.md`.
4. The port is keyed by `container_id`, but a VZ snapshot captures the whole VM, which hosts
   every container.

## Decisions Taken During Brainstorm

| #   | Decision                                                                                                                                               |
| --- | ------------------------------------------------------------------------------------------------------------------------------------------------------ |
| 1   | Scope is foundation (types and path hardening) plus exactly one real backend.                                                                          |
| 2   | The backend is macOS VZ (`saveMachineStateToURL` / `restoreMachineStateFromURL`): whole-VM state including memory.                                     |
| 3   | The port is re-keyed from `container_id` to a `VmId`.                                                                                                  |
| 4   | Each snapshot also takes copy-on-write `clonefile` copies of all three writable virtiofs shares (rootfs, images, containers). Restore swaps them back. |

Correction recorded during design: the images share is **writable** (`crates/macbox/src/vz/vm.rs`,
`build_virtiofs(vm_boot::IMAGES_TAG, ..., false)`), because the guest pulls images itself. The
doc comment on `IMAGES_TAG` in `crates/minibox-core/src/vm_boot.rs` still says "read-only" and is
corrected as part of Phase 3.

## Delivery Phases

The change spans more than three crates because replacing the port signature forces every
consumer to change in one compile-clean step. To keep each unit reviewable, it ships as four
phases. Phases 1 and 2 do not need the VZ spike.

| Phase | Content                                                                                                                                         | Crates                                                                  | Gate                                                  |
| ----- | ----------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------- | ----------------------------------------------------- |
| 0     | Spike: does VZ allow save/restore on this VM configuration?                                                                                     | `macbox` (macOS only)                                                   | Result decides Phase 3. If it fails, stop and report. |
| 1     | Path hardening. Add `VmId` and `SnapshotLabel`; validate in the handler; remove the unvalidated path construction. Port unchanged.              | `minibox-domain`, `minibox`                                             | Ships independently; fixes the live defect.           |
| 2     | Lineage foundation. Identity and manifest types, new port signature, `SnapshotStore`, protocol and CLI changes, mock and conformance migration. | `minibox-domain`, `minibox-core`, `minibox`, `mbx`, `minibox-testsuite` | Mechanical, compile-driven migration in one wave.     |
| 3     | VZ backend. Pinned VM identity, `VzVm` restructuring, `VzCheckpoint`, wiring.                                                                   | `macbox`, `miniboxd` (wiring test)                                      | Requires Phase 0 result and Phase 2.                  |

## Phase 0: Spike

A macOS-only, `#[ignore]` test (run on request, like the QEMU-gated tests) that builds the real
VM configuration and answers:

1. Does `validateSaveRestoreSupportWithError` accept a configuration with three virtiofs shares,
   a virtio-vsock device and a NAT network device? (The binding exists in
   `objc2-virtualization` 0.3.2.)
2. Does a real pause, save, stop, restore, resume round trip work, with the machine identifier
   and MAC address pinned?
3. Do host-side vsock connections survive a restore, or must the agent connection be rebuilt?
4. How large are the three shares, and how long does hashing them take? (Decides whether the
   images share stays in the swap set.)

The VM must boot for the spike to run. It does boot to the agent today. Container execution
inside it is separately blocked on a `pivot_root` `EINVAL`
(`.ctx/godmode/BLOCKED-vz-pivot-root-einval.md`), which does not affect snapshotting the VM
itself but limits what a restored VM can demonstrate.

## Crate Ownership

- **`minibox-domain`**: identity and manifest types, the port, `SnapshotError`. It has no OS
  dependencies and already depends on `sha2`, `hex`, `nutype`, `serde` and `thiserror`.
- **`minibox`**: `SnapshotStore` (portable filesystem logic, compiled and tested on Linux CI),
  handlers, `MockVmCheckpoint`. Already depends on `sha2`, `walkdir`, `nix`, `tempfile`, `miette`.
- **`macbox`**: `VzCheckpoint`, pinned VM identity, `VzVm` restructuring. The only crate with
  Virtualization.framework code.
- **Affected, signature-driven changes only**: `minibox-core` (`protocol.rs`), `mbx`
  (`commands/snapshot.rs`, `main.rs`), `minibox-testsuite`
  (`adapters/vm_checkpoint.rs`), `miniboxd` (`tests/checkpoint_wiring_tests.rs`).

## Public API

### Identity and manifest types (`minibox-domain`, new module `snapshot`)

```rust
/// Identity of a VM that owns snapshots. 1..=64 chars of [a-z0-9_-], starting alphanumeric.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct VmId(String);

impl VmId {
    pub fn parse(raw: &str) -> Result<Self, SnapshotError>;
    pub fn as_str(&self) -> &str;
}

/// Human-readable snapshot label. 1..=64 chars of [A-Za-z0-9._-], not starting with '.'.
/// Stored in the manifest only; never used to build a path.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct SnapshotLabel(String);

impl SnapshotLabel {
    pub fn parse(raw: &str) -> Result<Self, SnapshotError>;
    pub fn as_str(&self) -> &str;
}

/// Content address of a snapshot manifest: `sha256:<64 lowercase hex>`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct SnapshotId(String);

impl SnapshotId {
    pub fn parse(raw: &str) -> Result<Self, SnapshotError>;
    pub fn from_digest(digest: [u8; 32]) -> Self;
    /// The 64-character hex digest, which is also the snapshot directory name.
    pub fn hex(&self) -> &str;
    pub fn as_str(&self) -> &str;
}

/// What a payload is. Determines its location inside the snapshot directory.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PayloadKind {
    /// Machine state file (`state.vzstate`).
    MachineState,
    /// Copy-on-write clone of a virtiofs share (`clones/<tag>`). `tag` is [a-z0-9_-]{1,64}.
    ShareClone { tag: String },
}

impl PayloadKind {
    /// Path relative to the snapshot directory.
    pub fn relative_path(&self) -> std::path::PathBuf;
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PayloadEntry {
    pub kind: PayloadKind,
    /// File length in bytes; for a clone, the total of all file lengths.
    pub len: u64,
    /// SHA-256 hex of the file; for a clone, the tree digest over sorted
    /// (relative path, mode, length, file SHA-256) records.
    pub sha256: String,
}

/// Immutable record whose canonical bytes determine the `SnapshotId`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SnapshotManifest {
    pub schema_version: u32,
    pub vm_id: VmId,
    pub parent: Option<SnapshotId>,
    /// RFC 3339 creation timestamp.
    pub created_at: String,
    /// Adapter that produced the snapshot, e.g. `vz`.
    pub adapter: String,
    pub label: Option<SnapshotLabel>,
    /// SHA-256 hex over the pinned VM configuration (memory, CPU, share tags,
    /// pinned identifiers, boot-asset hashes).
    pub vm_config_digest: String,
    /// Sorted by `PayloadKind::relative_path`.
    pub payloads: Vec<PayloadEntry>,
}

impl SnapshotManifest {
    pub const SCHEMA_VERSION: u32 = 1;
    /// Deterministic serialization: struct field order, sorted payloads, no whitespace.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, SnapshotError>;
    pub fn id(&self) -> Result<SnapshotId, SnapshotError>;
    pub fn size_bytes(&self) -> u64;
}

/// A published snapshot: the manifest and its derived ID.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SnapshotInfo {
    pub id: SnapshotId,
    pub manifest: SnapshotManifest,
}
```

### Error type (`minibox-domain`)

Callers match on these, so this is a `thiserror` enum with `miette::Diagnostic` derived, per
`.claude/rules/rust-patterns.md`. This adds `miette` (already a workspace dependency) to
`minibox-domain`.

```rust
#[derive(Debug, thiserror::Error, miette::Diagnostic)]
pub enum SnapshotError {
    #[error("invalid {kind}: {reason}")]
    InvalidIdentifier { kind: &'static str, reason: String },
    #[error("unknown vm: {0}")]
    UnknownVm(String),
    #[error("snapshot not found: {0}")]
    NotFound(String),
    #[error("snapshot corrupt: {0}")]
    Corrupt(String),
    #[error("parent snapshot missing or corrupt: {0}")]
    ParentInvalid(String),
    #[error("ambiguous snapshot label: {0}")]
    AmbiguousLabel(String),
    #[error("checkpoint: not supported by this adapter")]
    Unsupported,
    #[error("checkpoint backend failure: {0}")]
    Backend(String),
}
```

### Port (`minibox-domain`, `checkpoint.rs`, replacing the current signatures)

```rust
pub trait VmCheckpoint: Send + Sync {
    /// Capture VM state as a new immutable snapshot, optionally as a child of `parent`.
    fn save_snapshot(
        &self,
        vm: &VmId,
        parent: Option<&SnapshotId>,
        label: Option<&SnapshotLabel>,
    ) -> Result<SnapshotInfo, SnapshotError>;

    /// Roll the VM back to `snapshot`. Verifies the snapshot before touching the VM.
    fn restore_snapshot(&self, vm: &VmId, snapshot: &SnapshotId) -> Result<(), SnapshotError>;

    /// All verified snapshots of `vm`. Corrupt entries are an error, not silently skipped.
    fn list_snapshots(&self, vm: &VmId) -> Result<Vec<SnapshotInfo>, SnapshotError>;
}
```

`DynVmCheckpoint` is unchanged. `NoopVmCheckpoint` returns `SnapshotError::Unsupported` from all
three methods and still does not touch the filesystem. The port no longer takes a `&Path`, so no
caller-supplied path can reach an adapter.

### `SnapshotStore` (`minibox`, new `adapters/snapshot_store.rs`)

```rust
pub struct SnapshotStore { /* canonical root, 0700 */ }

pub struct SnapshotDraft {
    pub vm_id: VmId,
    pub parent: Option<SnapshotId>,
    pub label: Option<SnapshotLabel>,
    pub adapter: String,
    pub vm_config_digest: String,
    /// Payloads already written into the staging directory.
    pub payloads: Vec<PayloadKind>,
}

/// An in-progress save. Dropping it without `publish` removes the staging directory.
pub struct StagingDir { /* private */ }

impl StagingDir {
    pub fn path(&self) -> &std::path::Path;
}

/// A snapshot whose manifest, payload lengths/digests and parent chain were verified.
pub struct VerifiedSnapshot { /* private */ }

impl VerifiedSnapshot {
    pub fn info(&self) -> &SnapshotInfo;
    pub fn payload_path(&self, kind: &PayloadKind) -> std::path::PathBuf;
}

impl SnapshotStore {
    /// Create the root (0700), canonicalize it once, and remove stale `.staging` entries.
    pub fn open(root: &std::path::Path) -> miette::Result<Self>;
    pub fn begin(&self, vm: &VmId) -> Result<StagingDir, SnapshotError>;
    /// Hash payloads, write `manifest.json` last, fsync, compute the ID, and atomically
    /// rename staging to `<hex>`. An existing identical snapshot is a successful no-op.
    pub fn publish(
        &self,
        staging: StagingDir,
        draft: SnapshotDraft,
    ) -> Result<SnapshotInfo, SnapshotError>;
    pub fn load(&self, vm: &VmId, id: &SnapshotId) -> Result<VerifiedSnapshot, SnapshotError>;
    pub fn list(&self, vm: &VmId) -> Result<Vec<SnapshotInfo>, SnapshotError>;
}
```

### VZ backend (`macbox`, `vz` feature)

```rust
/// The single VM the VZ adapter manages.
pub const VZ_VM_ID: &str = "minibox-vz";

/// Platform machine identifier and NAT MAC, generated once and persisted in the VM directory.
/// Virtualization.framework requires a restored VM's configuration to match the saved one;
/// today both are random on every build.
pub struct VmIdentity { /* private */ }

impl VmIdentity {
    pub fn load_or_create(vm_dir: &std::path::Path) -> anyhow::Result<Self>;
    pub fn digest_input(&self) -> Vec<u8>;
}

pub struct VzCheckpoint { /* Arc<VzVm>, Arc<SnapshotStore>, VzVmConfig, VmIdentity */ }

impl VzCheckpoint {
    pub fn new(
        vm: std::sync::Arc<VzVm>,
        store: std::sync::Arc<SnapshotStore>,
        config: VzVmConfig,
        identity: VmIdentity,
    ) -> Self;
}

impl VmCheckpoint for VzCheckpoint { /* signatures as the port */ }
```

`VzVm` gains a swappable inner machine so a restore does not invalidate the clones of
`Arc<VzVm>` that `VzRegistry`, `VzFilesystem`, `VzLimiter` and `VzRuntime` hold
(`crates/macbox/src/lib.rs:563-607`):

```rust
impl VzVm {
    /// Build a VM from `config` and `identity` without starting it. Main-queue only.
    pub fn build_unstarted(
        config: &VzVmConfig,
        identity: &VmIdentity,
    ) -> anyhow::Result<Retained<VZVirtualMachine>>;
    pub fn pause(&self) -> anyhow::Result<()>;
    pub fn resume(&self) -> anyhow::Result<()>;
    pub fn save_state(&self, dest: &std::path::Path) -> anyhow::Result<()>;
    /// Replace the inner machine with `fresh`, restore `source` into it, and leave it paused.
    pub fn restore_state(
        &self,
        fresh: Retained<VZVirtualMachine>,
        source: &std::path::Path,
    ) -> anyhow::Result<()>;
}
```

`prepare_on_main_queue` is split into `build_unstarted` plus a start step; its behavior and
callers are unchanged.

## Protocol Changes (`minibox-core`, `protocol.rs`)

The wire field names stay as they are; semantics are re-keyed.

| Message                          | Change                                                                                                                                                                            |
| -------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `SaveSnapshot { id, name }`      | `id` is now a VM ID. `name` is the label. New `#[serde(default)] parent: Option<String>`.                                                                                         |
| `RestoreSnapshot { id, name }`   | `id` is a VM ID. `name` is a snapshot reference: a `SnapshotId`, or a label that must resolve to exactly one snapshot.                                                            |
| `ListSnapshots { id }`           | `id` is a VM ID.                                                                                                                                                                  |
| `SnapshotSaved { info }`         | `info` becomes the new `SnapshotInfo` shape. This is a breaking JSON change, but safe: no backend has ever produced a `SnapshotInfo`, so none exists to be persisted or replayed. |
| `SnapshotRestored { id, name }`  | `name` carries the restored snapshot ID.                                                                                                                                          |
| `SnapshotList { id, snapshots }` | `snapshots` uses the new shape.                                                                                                                                                   |

No response variants are added or removed, so terminal-response logic is unaffected. Protocol
tests (`protocol.rs` tests, `tests/protocol_evolution.rs`, `tests/proptest_roundtrip.rs`,
`tests/property_roundtrip.rs`) and any insta snapshots are updated in the same change.

## Handlers (`minibox`, `daemon/handler/snapshot.rs`)

- Parse request strings into `VmId`, `SnapshotLabel`, `SnapshotId` at the top of each handler.
  A parse failure returns `DaemonResponse::Error` before any filesystem access.
- Phase 1 (port unchanged): keep building paths, but only from validated values, and replace
  `create_dir_all` on raw input with a check that the resolved path stays under the snapshot
  root. Phase 2 deletes this path construction entirely, because the adapter owns storage.
- Phase 2: handlers call the port inside `spawn_blocking`. Label-to-ID resolution lives in the
  handler: list, filter by label, zero matches is `NotFound`, more than one is `AmbiguousLabel`.
- `handle_save_snapshot` gains a `parent: Option<String>` parameter; the dispatch in
  `daemon/server.rs:724-732` is updated to match.

## Data Flow

**Save**

1. Source: `mbx snapshot save <vm> [--label L] [--parent ID]` becomes `SaveSnapshot`.
2. The handler validates identifiers and calls `VmCheckpoint::save_snapshot` in `spawn_blocking`.
3. `VzCheckpoint` calls `SnapshotStore::begin`, then pauses the VM.
4. It writes `state.vzstate` into staging and `clonefile`s the three shares into
   `clones/<tag>` while the VM is paused.
5. It resumes the VM. A guard resumes it on every error path.
6. `SnapshotStore::publish` hashes the payloads, writes the manifest last, fsyncs, computes the
   ID and renames staging to `<snapshot_root>/<vm_id>/<hex>`.
7. Sink: `SnapshotSaved { info }`.

**Restore**

1. The handler resolves the reference to a `SnapshotId`.
2. `VzCheckpoint` calls `SnapshotStore::load`, which verifies the manifest hash against the
   directory name, every payload length and digest, and the parent chain. It also compares the
   manifest `vm_config_digest` with the current one and refuses on mismatch.
3. It stops the VM, renames the live shares aside, and `clonefile`s the snapshot's clones into
   place.
4. It builds a fresh machine with `build_unstarted`, restores the state into it, and resumes.
5. It deletes the renamed-aside shares only after the restore succeeds. Any failure after step 3
   renames them back.
6. Sink: `SnapshotRestored { id, name }`.

## Storage Layout

```text
<snapshot_root>/<vm_id>/
  .staging/<random>/        in-progress saves, never listed, swept on open
  <64-hex>/
    manifest.json
    state.vzstate
    clones/<tag>/
```

`<snapshot_root>` is created `0700` and canonicalized once at startup. Directory names come only
from validated `VmId` and `SnapshotId` values. Snapshot directories are opened with
`O_NOFOLLOW | O_DIRECTORY`. `openat2` from the original TODO is Linux-only and is not used on
macOS; a future Linux backend can adopt it behind the same port. Deleting a snapshot, and
refusing to delete one that has children, is out of scope.

## Hexagonal Boundaries

- **Port**: `VmCheckpoint` in `minibox-domain::checkpoint`.
- **Adapters**: `NoopVmCheckpoint` (`minibox-domain`, the default), `MockVmCheckpoint`
  (`minibox::testing::mocks`), `VzCheckpoint` (`macbox::vz::checkpoint`).
- **Shared infrastructure behind the adapter**: `SnapshotStore` is plain filesystem logic. The
  VZ adapter supplies payloads; the store never calls the framework.

## Wiring

- The VZ composition point is `crates/macbox/src/lib.rs` (around line 592), not
  `crates/miniboxd/src/main.rs`: `miniboxd` delegates the VZ adapter to `macbox::start()`. There,
  `NoopVmCheckpoint` is replaced by `VzCheckpoint`, and the VZ adapter declares
  `Capability::Checkpoint` in the same change, as the trait docs require.
- Every other suite keeps `NoopVmCheckpoint`. `HandlerDependencies` already has the `checkpoint`
  field, so no construction site gains a field.
- `crates/miniboxd/tests/checkpoint_wiring_tests.rs` pins the no-op text in `main.rs`. It is
  extended to cover the `macbox` sites too, with only the VZ site exempt.

## Testing

| Layer              | Tests                                                                                                                                                                                                                                                                                                    | Runs on                |
| ------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------- |
| Identifier types   | Reject empty, `..`, `/`, NUL, over-length, wrong hex length, uppercase hex; accept valid forms; serde round trip                                                                                                                                                                                         | everywhere             |
| `SnapshotManifest` | Canonical bytes are stable across field-equal values; ID changes if any field changes; payload ordering is normalized                                                                                                                                                                                    | everywhere             |
| `SnapshotStore`    | Publish then load round trip; tampered payload byte, truncated payload, edited manifest, renamed directory, missing parent and forged parent all fail closed; crash before rename leaves nothing listed; staging swept on open; duplicate publish is idempotent; symlinked snapshot directory is refused | everywhere (temp dirs) |
| Handlers           | Traversal inputs (`../../x`, absolute paths, embedded NUL) rejected with no filesystem effect, including with `NoopVmCheckpoint` (regression for the live defect)                                                                                                                                        | everywhere             |
| Port conformance   | `minibox-testsuite` `vm_checkpoint` cases migrated to the new signatures; add lineage and verification cases                                                                                                                                                                                             | everywhere             |
| Wiring             | Updated `checkpoint_wiring_tests`                                                                                                                                                                                                                                                                        | everywhere             |
| VZ round trip      | Save, mutate shares, restore, assert state and shares match                                                                                                                                                                                                                                              | macOS, `#[ignore]`     |

## Out of Scope

- Snapshot deletion and garbage collection.
- Other backends (native Linux overlay snapshots, krun, smolvm).
- The #12 rootless-containers work.
- Reconciling `DaemonState` container records after a restore, beyond failing in-flight
  requests. See Risks.
- Per-container snapshots. A snapshot is whole-VM by decision 3.

## Risks

- [ ] **Breaking API changes: yes.** `VmCheckpoint` signatures, `SnapshotInfo` shape, the
      `SnapshotSaved`/`SnapshotList` wire shape, and the `SaveSnapshot` request gaining a field.
      Mitigated because no backend has produced a snapshot yet, and the new field is
      `#[serde(default)]`.
- [ ] **New external dependency: no.** `miette` is added to `minibox-domain` but is already a
      workspace dependency. `clonefile` needs `libc`; whether `libc` exposes `clonefile` on macOS
      must be confirmed in Phase 3, otherwise declare it `extern "C"` with a SAFETY comment.
- [ ] **Feature flag required: yes.** `VzCheckpoint` and the `VzVm` restructuring sit behind the
      existing `vz` feature. `SnapshotStore` and the domain types are unconditional.
- [ ] **Spike risk.** Virtualization.framework may reject save/restore for this configuration.
      Phase 0 decides; Phases 1 and 2 still deliver the defect fix and the lineage foundation.
- [ ] **Hash cost.** Hashing three writable shares on every save may take long if the images
      share is large. Hashing runs after the VM resumes, so it does not extend the pause, but the
      save call is slow. Phase 0 measures it; the images share can be excluded from the swap set.
- [ ] **Images share rollback.** Restoring it also rolls back images pulled after the snapshot.
      If `~/.minibox/images` is shared with other adapters, swapping it could affect them.
      Phase 0 checks this before the share is included.
- [ ] **Stale daemon state.** After a restore the guest no longer matches `DaemonState`
      container records. Out of scope here, but clients must treat a restore as a VM restart.
- [ ] **VZ container execution is currently blocked** on the `pivot_root` `EINVAL`
      (`.ctx/godmode/BLOCKED-vz-pivot-root-einval.md`). It does not block snapshotting the VM, but
      it limits an end-to-end demonstration.
- [ ] **Shared handle.** `VzVm` is shared by four adapters. The swappable inner machine must be
      synchronized so no adapter call observes a half-restored VM.
