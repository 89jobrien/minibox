---
source_sha: 045070e8926941810fbe1c48663b9ea3640cffd0
sources:
  - crates/minibox-core/Cargo.toml
  - crates/minibox-core/src/
  - crates/minibox/Cargo.toml
  - crates/minibox/src/
  - xtask/src/protocol_drift.rs
generated: 2026-08-26
---

# Design: Minibox Crate Boundaries

## Goal

Establish enforceable inward-pointing dependencies by extracting a pure domain kernel from
`minibox-core`, retaining shared application infrastructure in `minibox-core`, and limiting
`minibox` to daemon orchestration and concrete runtime adapters.

## Approved Approach

Use the selected deep core split: add `minibox-domain`, migrate domain contracts into it behind
compatibility re-exports, canonicalize duplicated implementations, and add an architecture gate.

## Crate Ownership

- **`minibox-domain` (ring 0)**: domain value types, typed identifiers, ports, lifecycle events,
  execution policy, manifests, workflow rules, and validated internal paths. It performs no
  network, filesystem, process, socket, tracing-subscriber, or Tokio channel I/O.
- **`minibox-core` (ring 1)**: protocol and client transport, OCI parsing and storage, registry
  clients, preflight probes, tracing setup, shared adapter implementations, and test conformance
  infrastructure. It depends on `minibox-domain` and re-exports migrated APIs during transition.
- **`minibox` (ring 2)**: daemon handlers/server/state, native container primitives, telemetry
  servers, and concrete platform/runtime adapters. It depends inward on `minibox-core` and
  `minibox-domain` and contains no duplicate shared registry, domain, or preflight implementation.

Allowed dependency direction:

```text
minibox -> minibox-core -> minibox-domain
       \-----------------> minibox-domain
```

Neither `minibox-domain` nor `minibox-core` may depend on `minibox`.

## Public API

### Compatibility Surface

`minibox-core` preserves existing downstream paths while consumers migrate:

```rust
pub use minibox_domain as domain;
pub use minibox_domain::error::DomainError;
pub use minibox_domain::events::{ContainerEvent, EventSink};
pub use minibox_domain::path::InternalPath;
```

`minibox` preserves its convenience paths by re-exporting canonical definitions rather than
declaring parallel types or implementations:

```rust
pub use minibox_core::domain;
pub use minibox_core::error;
pub use minibox_core::preflight;
pub use minibox_core::protocol;
```

### Domain Progress Port

The domain no longer streams transport-level `DaemonResponse` values. It emits execution output
that the daemon maps to protocol responses at the inbound transport boundary.

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExecOutput {
    Stdout(Vec<u8>),
    Stderr(Vec<u8>),
    Exit(i32),
    Error(String),
}

#[async_trait::async_trait]
pub trait ProgressSink<T: Send + 'static>: Send + Sync {
    async fn send(&self, value: T) -> Result<(), ProgressClosed>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("progress receiver is closed")]
pub struct ProgressClosed;

#[async_trait::async_trait]
pub trait ExecOutputStream: Send {
    async fn next(&mut self) -> Option<ExecOutput>;
}

pub struct ExecSession {
    pub handle: ExecHandle,
    pub output: Box<dyn ExecOutputStream>,
}

#[async_trait::async_trait]
pub trait ExecRuntime: AsAny + Send + Sync {
    async fn run_in_container(
        &self,
        container_id: &ContainerId,
        spec: ExecSpec,
    ) -> anyhow::Result<ExecSession>;
}
```

Tokio channel integration remains outside the domain:

```rust
pub struct TokioProgressSink<T> {
    sender: tokio::sync::mpsc::Sender<T>,
}

impl<T: Send + 'static> TokioProgressSink<T> {
    pub const fn new(sender: tokio::sync::mpsc::Sender<T>) -> Self;
}

pub struct TokioExecOutputStream {
    receiver: tokio::sync::mpsc::Receiver<ExecOutput>,
}
```

### Domain Image Reference

`ImageRegistry` and `RegistryRouter` require a domain-owned reference type rather than importing
from the infrastructure-owned image store module.

```rust
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ImageRef {
    pub registry: Option<String>,
    pub namespace: String,
    pub name: String,
    pub tag: String,
    pub digest: Option<String>,
}
```

Parsing and formatting move with `ImageRef`; OCI manifests, HTTP operations, layer extraction,
and disk storage remain in `minibox-core::image`.

### Architecture Gate

The gate is an internal xtask API, not a workspace library API:

```rust
pub(crate) fn check_architecture(workspace_root: &std::path::Path) -> anyhow::Result<()>;
```

It validates Cargo metadata edges, canonical ownership paths, and forbidden shadow paths. The
pre-commit/CI gate fails when an inner ring depends on an outer ring or a removed shadow module
reappears.

## Data Flow

1. CLI/MCP/TUI clients create protocol requests through `minibox-core::client`.
2. `minibox` daemon handlers translate protocol requests into `minibox-domain` values and ports.
3. Concrete adapters in `minibox`, `macbox`, `smolbox`, or `winbox` implement domain ports.
4. Adapter results return as domain values; daemon handlers map them to protocol responses.
5. Shared registry, image-store, preflight, and transport I/O stay in `minibox-core` and never
   enter `minibox-domain`.

## Hexagonal Boundaries

- **Ports**: traits in `minibox-domain`, including `ImageRegistry`, `ContainerRuntime`,
  `FilesystemProvider`, `ResourceLimiter`, `ExecRuntime`, and `EventSink`.
- **Shared adapters**: registry routing, Docker Hub registry, Tokio progress sink, event broker,
  preflight probes, and socket client in `minibox-core`.
- **Runtime adapters**: native Linux, GKE, Colima, smolvm, krun, and WSL2 implementations in the
  platform/runtime crates.
- **Composition root**: `miniboxd`; environment selection and dependency wiring remain there.

## Canonicalization

- Delete undeclared shadows under `crates/minibox/src/domain/` and
  `crates/minibox/src/preflight.rs` after proving they have no module declarations.
- Remove `crates/minibox/src/adapters/registry.rs`; re-export
  `minibox_core::adapters::DockerHubRegistry` from `minibox::adapters`.
- Re-export core test fixtures instead of maintaining parallel builders in `minibox::testing`.
- Keep only Linux/runtime-specific errors in `minibox`; shared errors have one canonical owner.
- Rename or privatize any Linux-internal state enum that is not the canonical domain
  `ContainerState`.

## Test Strategy

- **Unit**: domain value parsing, protocol-to-domain output mapping, and architecture rule parsing.
- **Property**: `ImageRef` parse/display round trips and architecture graph acyclicity.
- **Fuzz**: retain protocol and image-reference parser targets after module moves; update crate
  imports without reducing corpus coverage.
- **Conformance**: run every existing port suite against all adapter implementations after trait
  paths move; add an `ExecOutput` contract case for stdout, stderr, exit, and closed sinks.
- **Integration**: compile and exercise one daemon-to-adapter request through compatibility
  re-exports and one direct `minibox-domain` consumer.
- **Regression**: architecture tests pin the current duplicate/shadow failures so they cannot
  return.

No new Kani proof is required: this change introduces no new arithmetic, unsafe code, or bounded
state transition. Existing proofs remain in place and imports are updated as needed.

## Migration Sequence

1. Add `minibox-domain` and move dependency-free domain types without changing behavior.
2. Add compatibility re-exports from `minibox-core`; migrate `minibox` first while downstream
   crates continue compiling through old paths.
3. Decouple `ExecRuntime` from `DaemonResponse` and split event ports from Tokio adapters.
4. Canonicalize registry, preflight, error, state, and test-fixture ownership.
5. Enable the architecture gate in local verification and CI.
6. Migrate downstream crates to direct `minibox-domain` imports in later, independently reviewable
   changes; remove compatibility aliases only in a semver-major release.

## Out of Scope

- Changing daemon protocol wire formats.
- Rewriting OCI storage or registry behavior.
- Moving platform adapters between `minibox`, `macbox`, `smolbox`, and `winbox` in this pass.
- Removing compatibility re-exports before a semver-major release.
- Refactoring unrelated Dupehound findings in tests or CLI wrappers.

## Risk

- [x] Breaking API changes: trait source crates and `ExecRuntime` output type change internally;
      compatibility re-exports preserve existing type paths where possible.
- [x] New workspace dependency: `minibox-domain`, with only domain-safe dependencies.
- [x] Feature flags: `test-utils` remains owned by `minibox-core`; `minibox-domain` has no default
      infrastructure features.
- [x] Serialization risk: moved types retain their existing Serde names and representations.
- [x] Cross-platform risk: every adapter crate must pass conformance and compile gates before the
      compatibility layer is reduced.
