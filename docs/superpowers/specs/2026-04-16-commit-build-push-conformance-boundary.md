# Commit / Build / Push Conformance Boundary

**Date:** 2026-04-16
**Status:** Active
**Relates to:** minibox-35, minibox-40–42, minibox-49–51

## Overview

This document defines the conformance boundary for the three high-level container lifecycle
operations that minibox exposes beyond basic `run`/`stop`: **commit**, **build**, and **push**.
It specifies which domain ports govern each operation, which backend adapters currently implement
them, the skip/fail semantics the conformance suite uses, and what remains unimplemented for the
macOS Colima path.

---

## Domain Ports

Each operation is represented as a trait in `minibox-core::domain`:

| Operation | Port trait          | Config type     | Result type   |
|-----------|---------------------|-----------------|---------------|
| Commit    | `ContainerCommitter`| `CommitConfig`  | `ImageMetadata` |
| Build     | `ImageBuilder`      | `BuildConfig` + `BuildContext` | `ImageMetadata` |
| Push      | `ImagePusher`       | `RegistryCredentials` | `PushResult` |

All three traits require `AsAny + Send + Sync` and are declared `#[async_trait]`. They are
object-safe and exposed as `DynContainerCommitter`, `DynImageBuilder`, `DynImagePusher`
type aliases.

---

## Capability Flags

`BackendCapability` (in `minibox-core::domain`) enumerates the operations:

```rust
pub enum BackendCapability {
    Commit,            // ContainerCommitter::commit
    BuildFromContext,  // ImageBuilder::build_image
    PushToRegistry,    // ImagePusher::push_image
}
```

`BackendCapabilitySet` is a `HashSet`-backed builder that each backend declares at test time via
`BackendDescriptor`. The conformance suite checks `backend.capabilities.supports(cap)` before
running any test for that capability.

---

## Backend Support Matrix

| Backend        | `Commit` | `BuildFromContext` | `PushToRegistry` | Notes |
|----------------|:--------:|:-----------------:|:----------------:|-------|
| linux-native   | yes      | yes               | yes              | Requires root + overlay FS |
| Colima (macOS) | no       | no                | yes              | Uses `nerdctl push` via lima VM |
| GKE (proot)    | no       | no                | no               | No writable upperdir exposed |
| vz             | blocked  | blocked           | blocked          | VZErrorInternal on macOS 26 ARM64 |

### linux-native detail

- **Commit**: `OverlayCommitAdapter` — tars the overlay `upperdir`, stores as new layer blob,
  produces an OCI manifest. Calls `commit_upper_dir_to_image` (sync inner fn, `spawn_blocking`
  wrapper in the adapter).
- **Build**: `MiniboxImageBuilder` — interprets a minimal Dockerfile subset, runs each step as
  a container via the native runtime, commits the diff at each step.
- **Push**: `OciPushAdapter` — re-compresses extracted layer dirs back to gzip tarballs, uploads
  via OCI Distribution Spec v1 (HEAD-check → POST+PUT), pushes manifest. Digest will not match
  original pull digest (layers re-compressed); faithful for commit-produced images.

### Colima detail

- **Push only**: `ColimaImagePusher` delegates to `nerdctl push` inside the Lima VM.
- **No commit**: Lima/nerdctl containers do not expose a host-visible `upperdir`. The overlay
  merge is inside the VM. No `ContainerCommitter` implementation exists for Colima.
- **No build**: No Dockerfile build path wired into the Colima adapter suite. `nerdctl build`
  could be delegated but is not yet implemented.

---

## Conformance Test Entrypoints

```bash
# Run the full conformance suite (all backends, all tiers)
cargo xtask test-conformance

# Per-operation test files (in crates/mbx/tests/)
cargo nextest run --test conformance_commit    # ContainerCommitter tests
cargo nextest run --test conformance_build     # ImageBuilder tests
cargo nextest run --test conformance_push      # ImagePusher tests
cargo nextest run --test conformance_report    # Report generation (Markdown + JSON)
```

Reports are written to `artifacts/conformance/` (gitignored; CI uploads as artifacts).
Override with `CONFORMANCE_ARTIFACT_DIR`.

Optional env vars:
- `CONFORMANCE_PUSH_REGISTRY=localhost:5000` — enable Tier 2 push tests against a live registry
- `CONFORMANCE_COLIMA=1` — enable Colima backend tests (requires running Lima VM)

---

## Skip / Fail Semantics

A test **skips** (returns early, does not fail) when:

```rust
if !backend.capabilities.supports(BackendCapability::Commit) {
    return; // skip — backend does not support commit
}
```

A test **fails** when the backend declares the capability but the operation returns an error or
produces incorrect output.

This means:
- Colima not supporting `Commit` is a **skip**, not a failure.
- `OciPushAdapter` failing on a reachable registry is a **failure**.
- Adding a new backend that declares `Commit` without a working implementation causes **failures**
  in the commit conformance tests — use this as a gate.

---

## Fixture Infrastructure

`minibox-core::adapters::conformance` (behind the `test-utils` feature) provides:

| Fixture | Purpose |
|---------|---------|
| `MinimalStoredImageFixture` | Creates a minimal extracted image layer on disk |
| `WritableUpperDirFixture` | Creates a writable `upperdir` with test files for commit |
| `BuildContextFixture` | Creates a minimal build context dir with a stub Dockerfile |
| `LocalPushTargetFixture` | Provides a local OCI registry ref for push round-trip tests |
| `BackendDescriptor` | Declares backend capabilities + zero-arg factory closures for each adapter |

`BackendDescriptor` factories take no arguments — required context (image store paths, state
handles) is captured from the surrounding fixture, so each test invocation gets a fresh adapter
with no shared mutable state between cases.

---

## Open Gaps (Colima path — minibox-40–42, minibox-49–51)

The following work is required to extend the conformance boundary to macOS Colima:

| ID | Gap |
|----|-----|
| minibox-40 | Define the rootfs metadata contract — what fields does a Colima container expose to host-side commit logic? |
| minibox-41 | Persist rootfs metadata into `ContainerRecord` during `create`/`run` so commit can retrieve it without re-querying the VM |
| minibox-42 | Implement `ColimaFilesystemMetadata` — map Lima container paths to the metadata contract |
| minibox-49 | Extend `FilesystemProvider::setup_rootfs` return type (`RootfsLayout`) to carry backend-specific writable-layer metadata |
| minibox-50 | Wire the macbox Colima path to existing local commit/build adapters using the extended `RootfsLayout` |
| minibox-51 | End-to-end dogfood: `create` → `commit` → `push` on macOS Colima |

Until these are complete, Colima declares only `PushToRegistry` in its `BackendCapabilitySet`.
Conformance tests for `Commit` and `BuildFromContext` skip on Colima backends.
