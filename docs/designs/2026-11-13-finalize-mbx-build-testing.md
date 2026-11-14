# Design: Finalize `mbx build` (Testing + Protocol Drift)

## Goal

Surface the fully-implemented `mbx build` feature by adding protocol drift tracking for the Build protocol surface, comprehensive end-to-end tests, and removing the stale TODO comment.

## Approved Approach

Testing + documentation approach: the feature is production-ready and wired end-to-end. Add protocol drift surface, end-to-end tests (unit + integration + conformance), and remove TODO #183.

## Crate Ownership

- **Handler logic**: `minibox` crate (`crates/minibox/src/daemon/handler/image.rs`)
- **Tests**: `minibox` crate (unit tests in `crates/minibox/src/daemon/handler/image.rs`) + `minibox-testsuite` crate (conformance tests)
- **Protocol drift**: `xtask` crate (`xtask/src/protocol_drift.rs`)
- **Protocol cleanup**: `minibox-core` crate (`crates/minibox-core/src/protocol.rs`)

## Public API

### New Test Functions (Unit Tests in `crates/minibox/src/daemon/handler/image.rs`)

```rust
#[cfg(test)]
mod tests {
    /// Verify handle_build rejects unsupported adapters with clear error
    #[tokio::test]
    async fn handle_build_rejects_unsupported_adapter_with_error();

    /// Verify handle_build validates context_path is absolute
    #[tokio::test]
    async fn handle_build_validates_context_path_is_absolute();

    /// Verify handle_build canonicalizes context_path and rejects escaped symlinks
    #[tokio::test]
    async fn handle_build_rejects_symlink_escape_outside_context();

    /// Verify handle_build writes temporary Dockerfile and cleans up after success
    #[tokio::test]
    async fn handle_build_writes_and_cleans_temporary_dockerfile();

    /// Verify handle_build invokes ImageBuilder and streams BuildOutput + BuildComplete
    #[tokio::test]
    async fn handle_build_streams_output_and_completion();

    /// Verify handle_build propagates ImageBuilder errors to client
    #[tokio::test]
    async fn handle_build_propagates_builder_errors();
}
```

### Conformance Test (in `crates/minibox-testsuite/src/build_capability.rs`)

```rust
/// Verify adapters report BuildFromContext capability truthfully.
///
/// Adapters reporting BuildFromContext = true MUST handle Build requests without
/// "not supported" errors. Adapters reporting false MUST return "not supported" error.
#[conformance_test]
async fn adapter_build_capability_is_truthful(adapter: &dyn ContainerRuntime);
```

### Protocol Drift Surface (in `xtask/src/protocol_drift.rs`)

```rust
// Add to SURFACES array:
Surface {
    name: "build-protocol",
    path: "crates/minibox-core/src/protocol.rs",
}
```

## Data Flow

1. **Test setup**: Create mock `ImageBuilder`, spawn test daemon
2. **Client**: Send `DaemonRequest::Build { dockerfile, context_path, tag, build_args, no_cache }`
3. **Handler**: Validate context path → invoke builder → stream responses
4. **Response**: `BuildOutput { step, total_steps, message }` (multiple) → `BuildComplete { image_id, tag }`
5. **Verify**: Assert CLI receives all responses in order

## Hexagonal Boundaries

- **Port (trait)**: `minibox_core::domain::ImageBuilder` — already exists
- **Adapter (impl)**: `crate::adapters::OverlayImageBuilder` (native) — already exists
- **Test double**: `crates/minibox/src/daemon/handler/image.rs::tests::MockImageBuilder` — implement for tests

## Affected Files

| File                                               | Change                                           |
| -------------------------------------------------- | ------------------------------------------------ |
| `xtask/src/protocol_drift.rs`                      | Add `build-protocol` surface to `SURFACES` array |
| `crates/minibox/src/daemon/handler/image.rs`       | Add 6 unit tests in `#[cfg(test)]` module        |
| `crates/minibox-testsuite/src/build_capability.rs` | Create new conformance test file                 |
| `crates/minibox-testsuite/src/lib.rs`              | Register `build_capability` module               |
| `crates/minibox-core/src/protocol.rs`              | Remove TODO #183 comment at line 8               |

## Dependencies (May Need Updates)

- `crates/minibox/src/daemon/handler/mod.rs` — exports `handle_build` (no change; already exported)
- `crates/minibox/src/daemon/server.rs` — dispatches to `handle_build` (no change; already wired)

## Test Strategy

### Tier 1: Unit Tests (in `image.rs`)

- Capability gating: unsupported adapter returns error
- Path validation: absolute, canonicalized, escape-proof
- Temporary Dockerfile creation and cleanup
- Mock ImageBuilder integration and streaming
- Error propagation from builder

### Tier 2: Integration Tests (in `miniboxd/tests/` or as new file)

- Full CLI→daemon→builder flow on native adapter
- Verify CLI receives `BuildComplete` and exits 0
- Verify built image appears in daemon's store

### Tier 3: Conformance Tests (in `minibox-testsuite`)

- Adapters with `BuildFromContext = true` must not error
- Adapters with `BuildFromContext = false` must return "not supported"

## Out of Scope

- **Implementation changes**: handler is production-ready
- **Policy gating**: auth-policy for Build is v2 work
- **New protocol fields**: protocol is stable
- **macOS/Windows build support**: those adapters remain "not supported"

## Risk

- [ ] **Breaking API changes**: No — all APIs are already public and stable
- [ ] **New external dependency**: No
- [ ] **Serialization format change**: No — protocol fields unchanged
- [ ] **Feature flag required**: No
- [ ] **Cross-crate boundary changes**: No — just wiring tests across crates

## Verification Checklist

- [ ] Protocol drift surface added; `cargo xtask check-protocol-drift` passes
- [ ] All 6 unit tests pass; coverage for error paths
- [ ] Integration test runs end-to-end build on native adapter
- [ ] Conformance test verifies adapter truthfulness
- [ ] TODO #183 removed; no stale references remain
- [ ] `cargo xtask verify` passes (fmt + clippy + tests + architecture)
- [ ] Pre-commit gate passes (fmt + clippy + test)
