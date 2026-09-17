---
source_sha: d8e7ef8cfb6b1e9005e632e8d8183f8148b501bb
sources:
  - crates/minibox-domain/src/execution_manifest.rs
  - crates/minibox-domain/src/execution_policy.rs
  - crates/minibox-core/src/protocol.rs
  - crates/minibox-core/src/trace.rs
  - crates/minibox/src/daemon/handler/
  - crates/minibox/src/daemon/server.rs
  - crates/minibox/src/daemon/state.rs
  - crates/miniboxd/src/config.rs
  - crates/miniboxd/src/main.rs
  - crates/mbx/src/commands/sandbox.rs
  - crates/mcp/src/
generated: 2026-09-16
---

# Design: Daemon-Native Verified Execution

> Paths labelled as new in the context map are proposed implementation targets and may not
> exist until this historical design is implemented. Existing domain sources moved from
> `minibox-core/src/domain/` to the canonical `minibox-domain` crate; references below use
> their current locations.

## Goal

Provide one daemon-owned operation that policy-checks, executes, attests, cleans up, and audits an isolated workload so CLI and MCP clients receive the same verifiable result.

## Approved Approach

Add a daemon-native verified-execution transaction with persistent, bounded audit records retained by TTL and count cap.

## Delivery Slices

The design is split into two sequential slices to keep each implementation boundary at three crates or fewer:

1. **Daemon capability**: `minibox-core`, `minibox`, and `miniboxd` own the domain types, wire protocol, orchestration, audit adapter, and configuration.
2. **Client surfaces**: `mbx` and `minibox-mcp` map their existing inputs onto the daemon operation without duplicating lifecycle or verification logic.

The daemon slice must land and pass protocol/conformance gates before either client surface is changed.

## Context Map

### Files To Modify: Daemon Capability

| File                                                      | Purpose                                            | Changes Needed                                                                                   |
| --------------------------------------------------------- | -------------------------------------------------- | ------------------------------------------------------------------------------------------------ |
| `crates/minibox-domain/src/verified_execution.rs`         | New verified-execution domain model and audit port | Add request, evidence, status, retention, query, and store types                                 |
| `crates/minibox-domain/src/lib.rs`                        | Domain exports                                     | Export verified-execution types                                                                  |
| `crates/minibox-core/src/protocol.rs`                     | Canonical daemon protocol                          | Add execute/list/show/prune requests and responses; update terminal classification and snapshots |
| `crates/minibox-core/tests/protocol_evolution.rs`         | Protocol compatibility snapshots                   | Cover additive request and response shapes                                                       |
| `crates/minibox-core/tests/property_roundtrip.rs`         | Protocol serialization properties                  | Generate and round-trip new variants                                                             |
| `crates/minibox/src/adapters/execution_audit.rs`          | Filesystem audit adapter                           | Persist owner-only JSON records with atomic replacement and bounded pruning                      |
| `crates/minibox/src/adapters/mod.rs`                      | Adapter exports                                    | Export `FileExecutionAuditStore`                                                                 |
| `crates/minibox/src/daemon/handler/verified_execution.rs` | Transaction orchestration                          | Implement policy, run, timeout, evidence, cleanup, audit, query, and prune handlers              |
| `crates/minibox/src/daemon/handler/run.rs`                | Existing run pipeline                              | Expose crate-internal preparation/streaming primitives without changing `Run` behavior           |
| `crates/minibox/src/daemon/handler/lifecycle.rs`          | Existing cleanup path                              | Extract reusable cleanup that preserves audit evidence while deleting runtime state              |
| `crates/minibox/src/daemon/handler/mod.rs`                | Handler dependencies and exports                   | Add focused verified-execution dependencies and handler exports                                  |
| `crates/minibox/src/daemon/server.rs`                     | Request dispatch                                   | Route new requests and test response termination                                                 |
| `crates/miniboxd/src/config.rs`                           | Layered daemon configuration                       | Add retention and execution-limit configuration with environment overrides                       |
| `crates/miniboxd/src/main.rs`                             | Composition root                                   | Build the file audit adapter, inject limits, prune, and reconcile interrupted records            |

### Files To Modify: Client Surfaces

| File                                         | Purpose                      | Changes Needed                                                                |
| -------------------------------------------- | ---------------------------- | ----------------------------------------------------------------------------- |
| `crates/mbx/src/main.rs`                     | CLI command model            | Route existing `sandbox` through verified execution and add audit subcommands |
| `crates/mbx/src/commands/sandbox.rs`         | Existing sandbox client      | Preserve script detection/mounting while sending `ExecuteVerified`            |
| `crates/mbx/src/commands/audit.rs`           | New audit client             | List, show, and prune execution records                                       |
| `crates/mbx/tests/sandbox_tests.rs`          | Sandbox integration coverage | Assert attestation output, timeout cleanup, and nonzero workload results      |
| `crates/mcp/src/types.rs`                    | MCP schemas                  | Add verified execution and audit inputs/outputs                               |
| `crates/mcp/src/tools/verified_execution.rs` | MCP tool mapping             | Map typed MCP calls to daemon requests                                        |
| `crates/mcp/src/server.rs`                   | MCP tool router              | Expose execute and read-only audit tools; mutation-gate prune                 |
| `crates/mcp/src/policy.rs`                   | MCP boundary policy          | Keep MCP-specific escalation checks before daemon policy evaluation           |
| `crates/mcp/tests/integration.rs`            | MCP stdio integration        | Prove tools map to the new protocol and enforce mutation policy               |

### Dependencies

| File                                              | Relationship                                                                     |
| ------------------------------------------------- | -------------------------------------------------------------------------------- |
| `crates/minibox-domain/src/execution_manifest.rs` | Supplies sealed workload identity embedded in every audit record                 |
| `crates/minibox-domain/src/execution_policy.rs`   | Evaluates operator and request policies independently; both must allow           |
| `crates/minibox/src/daemon/state.rs`              | Tracks transient container state but does not own retained execution evidence    |
| `crates/minibox/src/daemon/handler/manifest.rs`   | Existing manifest verification remains supported for ordinary containers         |
| `crates/minibox-core/src/trace.rs`                | Reference pattern for a synchronous storage port and one-file-per-record adapter |
| `crates/mbx/src/commands/run.rs`                  | Reference pattern for streaming `ContainerOutput` responses                      |
| `crates/mcp/src/tools/containers.rs`              | Reference pattern for bounded output normalization and daemon calls              |

### Existing Test Coverage

| Test Location                                     | Current Coverage                                   | Required Extension                                            |
| ------------------------------------------------- | -------------------------------------------------- | ------------------------------------------------------------- |
| `crates/minibox-domain/src/execution_manifest.rs` | Stable digest, hashed environment, serialization   | Embed manifests in audit records without weakening privacy    |
| `crates/minibox-domain/src/execution_policy.rs`   | Image, network, privilege, memory, and mount rules | Prove operator/request intersection semantics                 |
| `crates/minibox-core/src/protocol.rs`             | Wire snapshots and terminal table                  | Add all verified-execution variants                           |
| `crates/minibox/src/daemon/handler/mod.rs`        | Mock handler dependencies and policy paths         | Add failure-boundary cleanup and audit tests                  |
| `crates/mbx/tests/sandbox_tests.rs`               | Timeout and exit-code behavior                     | Move timeout ownership to daemon and assert returned evidence |
| `crates/mcp/tests/integration.rs`                 | Real MCP stdio mapping for run/list                | Add verified execution, audit reads, and prune denial         |

### Risk

- [x] Public API change: additive domain and protocol types in `minibox-core`.
- [x] Serialization change: new tagged protocol variants and persisted audit schema require snapshots.
- [x] Cross-crate boundary: daemon capability must land before client wrappers.
- [x] Security boundary: request policy must never weaken operator policy or `ContainerPolicy`.
- [x] Lifecycle boundary: client disconnect and timeout must not skip cleanup or audit finalization.
- [ ] New external dependency: none; existing Serde, SHA-256, Tokio, and filesystem facilities are sufficient.

## Crate Ownership

### Slice 1: Daemon Capability

- **Domain owner**: `minibox-core` owns portable verified-execution and audit contracts.
- **Application owner**: `minibox` owns daemon orchestration and the filesystem audit adapter.
- **Composition owner**: `miniboxd` creates configured adapters and performs startup retention/reconciliation.

### Slice 2: Client Surfaces

- **CLI owner**: `mbx` preserves the existing `sandbox` UX and adds audit inspection.
- **Agent owner**: `minibox-mcp` exposes typed MCP tools over the same daemon protocol.

No new crate is required.

## Public API

### Domain Types

```rust
// minibox-core::domain::verified_execution

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VerifiedExecutionSpec {
    pub image: String,
    pub tag: Option<String>,
    pub command: Vec<String>,
    pub memory_limit_bytes: Option<u64>,
    pub cpu_weight: Option<u64>,
    pub network: NetworkMode,
    pub env: Vec<String>,
    pub mounts: Vec<BindMount>,
    pub privileged: bool,
    pub name: Option<String>,
    pub platform: Option<String>,
    pub timeout_secs: u64,
    pub max_output_bytes: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ExecutionAuditStatus {
    Pending,
    Running,
    Completed,
    Denied,
    Failed,
    Interrupted,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ExecutionPolicySource {
    Operator,
    Request,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ExecutionPolicyResult {
    Allowed,
    Denied {
        source: ExecutionPolicySource,
        reason: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionPolicyEvidence {
    pub operator: Option<ExecutionPolicy>,
    pub request: ExecutionPolicy,
    pub result: ExecutionPolicyResult,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionOutputEvidence {
    pub sha256: String,
    pub bytes_observed: u64,
    pub bytes_forwarded: u64,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ExecutionCleanupStatus {
    Pending,
    Succeeded,
    Failed { reason: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionAuditRecord {
    pub schema_version: u32,
    pub id: String,
    pub status: ExecutionAuditStatus,
    pub created_at: String,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub manifest: ExecutionManifest,
    pub policy: ExecutionPolicyEvidence,
    pub exit_code: Option<i32>,
    pub output: ExecutionOutputEvidence,
    pub cleanup: ExecutionCleanupStatus,
    pub failure: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionAuditSummary {
    pub id: String,
    pub status: ExecutionAuditStatus,
    pub created_at: String,
    pub completed_at: Option<String>,
    pub image_ref: String,
    pub workload_digest: String,
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionAuditFilter {
    pub status: Option<ExecutionAuditStatus>,
    pub since: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionAuditRetention {
    pub max_age_secs: u64,
    pub max_records: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionAuditPruneReport {
    pub removed_ids: Vec<String>,
    pub retained: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VerifiedExecutionLimits {
    pub default_memory_limit_bytes: u64,
    pub default_cpu_weight: u64,
    pub max_timeout_secs: u64,
    pub max_output_bytes: u64,
}
```

`VerifiedExecutionSpec` is validated at the daemon boundary. Zero timeout/output limits and values above daemon limits are rejected rather than silently widened. Environment values appear only in the transient request and sealed manifest hashing path; audit records contain only `ExecutionManifestEnvVar::value_digest`.

### Audit Port

```rust
pub trait ExecutionAuditStore: Send + Sync {
    fn create(&self, record: &ExecutionAuditRecord) -> anyhow::Result<()>;
    fn replace(&self, record: &ExecutionAuditRecord) -> anyhow::Result<()>;
    fn load(&self, id: &str) -> anyhow::Result<Option<ExecutionAuditRecord>>;
    fn list(
        &self,
        filter: &ExecutionAuditFilter,
    ) -> anyhow::Result<Vec<ExecutionAuditSummary>>;
    fn prune(
        &self,
        retention: &ExecutionAuditRetention,
    ) -> anyhow::Result<ExecutionAuditPruneReport>;
}

pub type DynExecutionAuditStore = std::sync::Arc<dyn ExecutionAuditStore>;
```

`create` must fail when the ID already exists. `replace` must fail if the record does not exist or if a finalized record would transition to another status. Implementations must make each write atomic.

### Filesystem Adapter

```rust
// minibox::adapters::execution_audit

pub struct FileExecutionAuditStore { /* private fields */ }

impl FileExecutionAuditStore {
    pub fn new(base_dir: impl AsRef<std::path::Path>) -> anyhow::Result<Self>;
}

impl minibox_core::domain::ExecutionAuditStore for FileExecutionAuditStore { /* signatures above */ }
```

Records live at `<state_dir>/execution-audits/<id>.json`, use owner-only permissions, and are written through a same-directory temporary file plus atomic rename.

### Protocol Additions

```rust
pub enum DaemonRequest {
    ExecuteVerified {
        spec: VerifiedExecutionSpec,
        policy: ExecutionPolicy,
    },
    ListExecutionAudits {
        filter: ExecutionAuditFilter,
    },
    GetExecutionAudit {
        id: String,
    },
    PruneExecutionAudits,
    // existing variants unchanged
}

pub enum DaemonResponse {
    VerifiedExecutionComplete {
        record: ExecutionAuditRecord,
    },
    ExecutionAuditList {
        records: Vec<ExecutionAuditSummary>,
    },
    ExecutionAudit {
        record: ExecutionAuditRecord,
    },
    ExecutionAuditsPruned {
        report: ExecutionAuditPruneReport,
    },
    // existing variants unchanged
}
```

`ContainerOutput` remains the only streamed output response. All four new response variants are terminal. Denied, failed, interrupted, timed-out, and nonzero-exit workloads return `VerifiedExecutionComplete`; `DaemonResponse::Error` is reserved for malformed requests or failures that prevent creation of an initial audit record.

### Handler Dependencies And Functions

```rust
// minibox::daemon::handler

#[derive(Clone)]
pub struct VerifiedExecutionDeps {
    pub audit_store: DynExecutionAuditStore,
    pub retention: ExecutionAuditRetention,
    pub limits: VerifiedExecutionLimits,
}

pub struct HandlerDependencies {
    // existing fields unchanged
    pub verified_execution: VerifiedExecutionDeps,
}

pub async fn handle_execute_verified(
    spec: VerifiedExecutionSpec,
    request_policy: ExecutionPolicy,
    state: std::sync::Arc<DaemonState>,
    deps: std::sync::Arc<HandlerDependencies>,
    tx: tokio::sync::mpsc::Sender<DaemonResponse>,
);

pub async fn handle_list_execution_audits(
    filter: ExecutionAuditFilter,
    deps: std::sync::Arc<HandlerDependencies>,
) -> DaemonResponse;

pub async fn handle_get_execution_audit(
    id: String,
    deps: std::sync::Arc<HandlerDependencies>,
) -> DaemonResponse;

pub async fn handle_prune_execution_audits(
    deps: std::sync::Arc<HandlerDependencies>,
) -> DaemonResponse;
```

### Daemon Configuration

```rust
// miniboxd::config

#[derive(Debug, Clone, Deserialize)]
pub struct ExecutionAuditConfig {
    pub retention_secs: u64,
    pub max_records: usize,
    pub max_timeout_secs: u64,
    pub max_output_bytes: u64,
}

pub struct DaemonConfig {
    // existing fields unchanged
    pub execution_audit: ExecutionAuditConfig,
}
```

Environment overrides are `MINIBOX_AUDIT_RETENTION_SECS`, `MINIBOX_AUDIT_MAX_RECORDS`, `MINIBOX_VERIFIED_MAX_TIMEOUT_SECS`, and `MINIBOX_VERIFIED_MAX_OUTPUT_BYTES`.

### CLI Surface

The existing `mbx sandbox` command remains source-compatible. It gains `--policy <path>` and `--max-output-bytes <bytes>`, and its timeout moves from client cancellation into `VerifiedExecutionSpec::timeout_secs`.

```rust
// crates/mbx/src/commands/sandbox.rs

pub async fn execute_verified(
    params: SandboxExecParams,
    policy: ExecutionPolicy,
    socket_path: &std::path::Path,
) -> anyhow::Result<()>;

// crates/mbx/src/commands/audit.rs

pub async fn list(
    filter: ExecutionAuditFilter,
    socket_path: &std::path::Path,
) -> anyhow::Result<()>;

pub async fn show(id: String, socket_path: &std::path::Path) -> anyhow::Result<()>;

pub async fn prune(socket_path: &std::path::Path) -> anyhow::Result<()>;
```

The existing script-language detection and read-only script mount are reused; this design adds no second source-injection mechanism.

### MCP Surface

```rust
pub struct VerifiedExecutionInput {
    pub image: String,
    pub tag: Option<String>,
    pub command: Vec<String>,
    pub env: Vec<String>,
    pub memory_limit_bytes: Option<u64>,
    pub cpu_weight: Option<u64>,
    pub network: Option<String>,
    pub timeout_secs: Option<u64>,
    pub max_output_bytes: Option<u64>,
    pub policy: ExecutionPolicy,
}

pub struct VerifiedExecutionOutput {
    pub record: ExecutionAuditRecord,
    pub stdout: String,
    pub stderr: String,
}
```

The server adds `minibox_execute_verified`, `minibox_audit_list`, `minibox_audit_get`, and `minibox_audit_prune`. Execute uses safe MCP defaults and the daemon still applies operator policy. Audit list/get are read-only; prune requires `MINIBOX_MCP_ALLOW_MUTATION=true`.

## Data Flow

1. The client sends `ExecuteVerified` with a typed execution spec and request policy.
2. The daemon validates size/range limits and existing `ContainerPolicy` before image or runtime work.
3. The daemon resolves/pulls the image, builds and seals `ExecutionManifest`, then evaluates operator policy followed by request policy.
4. The daemon creates an owner-only pending audit record before container resources are created; failure aborts execution.
5. A denial finalizes the record as `Denied` and returns it without creating rootfs, cgroup, network, or process resources.
6. An allowed request reuses the existing run preparation/runtime ports, updates the record to `Running`, and streams bounded `ContainerOutput` while hashing every observed byte.
7. Timeout, workload exit, runtime failure, or client disconnect enters one cleanup path that tears down network, runtime state, rootfs, cgroup, and transient `ContainerRecord` state.
8. Cleanup outcome, exit status, output evidence, and final status are atomically persisted before `VerifiedExecutionComplete` is sent when a client remains connected.
9. Startup reconciliation changes pending/running audit records to `Interrupted`, records daemon-restart cleanup evidence, then applies TTL/count retention.
10. CLI and MCP audit reads query the retained store and never depend on transient container state.

## Hexagonal Boundaries

- **Port**: `ExecutionAuditStore` in `minibox-core::domain` defines persistence without filesystem knowledge.
- **Adapter**: `FileExecutionAuditStore` in `minibox::adapters` implements owner-only atomic JSON storage.
- **Existing ports reused**: `ContainerRuntime`, `FilesystemProvider`, `ResourceLimiter`, `NetworkProvider`, and `ImageRegistry` remain the only runtime infrastructure dependencies.
- **Application service**: `handle_execute_verified` owns transaction ordering and depends only on injected ports.

## Policy Semantics

Operator policy and request policy are evaluated independently against the same sealed manifest. Both must allow. The first denial is retained with `ExecutionPolicySource`; request policy can narrow but never widen operator policy. Existing `ContainerPolicy` checks for bind mounts and privilege run before manifest evaluation and cannot be overridden by this request.

## Failure Semantics

| Failure                               | Launch Workload         | Audit Result                              | Protocol Result                                  |
| ------------------------------------- | ----------------------- | ----------------------------------------- | ------------------------------------------------ |
| Invalid request or limits             | No                      | None                                      | `Error`                                          |
| Image resolution/pull before manifest | No                      | None                                      | `Error`                                          |
| Initial audit create                  | No                      | None                                      | `Error`                                          |
| Operator/request denial               | No                      | `Denied`                                  | `VerifiedExecutionComplete`                      |
| Runtime setup/spawn                   | No successful process   | `Failed`                                  | `VerifiedExecutionComplete`                      |
| Timeout                               | Process is stopped      | `Failed` with timeout reason              | `VerifiedExecutionComplete`                      |
| Nonzero workload exit                 | Yes                     | `Completed` with exit code                | `VerifiedExecutionComplete`                      |
| Cleanup failure                       | Yes or attempted        | Final status plus failed cleanup evidence | `VerifiedExecutionComplete`                      |
| Final audit replace                   | Already cleaned         | Best recoverable prior state remains      | `Error` if connected; critical daemon log always |
| Client disconnect                     | Continues independently | Normal final record                       | No response; cleanup/finalization still run      |

## Retention

- Retention uses both maximum age and maximum record count.
- Finalized records older than the TTL are removed first, then oldest finalized records are removed until the count cap is met.
- Pending/running records are never removed by ordinary retention; startup reconciliation finalizes them as interrupted first.
- Raw stdout/stderr are not persisted. The audit stores one SHA-256 digest, observed and forwarded byte counts, and truncation state.
- Prune uses daemon-configured retention only; clients cannot supply looser retention values.

## Test Boundaries

### Daemon Capability

- Domain tests: record serialization, final-state immutability, status filtering, retention ordering, environment privacy, operator/request denial source.
- Protocol tests: exact wire snapshots, exhaustive terminal table, property round trips, backward decoding of existing variants.
- Adapter tests: atomic create/replace, collision rejection, owner-only permissions, malformed-record tolerance, TTL/count pruning.
- Handler tests: denial before runtime resource creation, initial-write fail-closed behavior, output truncation with full-stream hashing, nonzero exit, timeout stop, runtime failure, cleanup failure, final-write failure, and disconnected receiver.
- Startup tests: pending/running records become interrupted before retention.

### Client Surfaces

- CLI subprocess tests: existing sandbox syntax remains valid, policy file reaches daemon, output streams, terminal evidence is printed, timeout is daemon-owned.
- MCP stdio tests: typed mapping, safe defaults, audit list/get, prune denial and allow paths.
- Real smoke: `smolvm` on macOS runs an inline Alpine workload and returns a retained record after runtime cleanup.
- Linux integration: native adapter proves namespace/cgroup cleanup and retained audit evidence.

## Out Of Scope

- Cryptographic signing or remote attestation of audit records.
- Persisting raw stdout/stderr.
- A second source upload/injection mechanism; `mbx sandbox` keeps its existing read-only bind mount.
- Interactive TTY or stdin for verified executions.
- Exec into an already-running container.
- TUI audit browsing; existing event observation remains compatible.
- Remote audit backends, multi-node replication, or external transparency logs.

## Integration And Compatibility

- Existing `Run`, `Manifest`, and `VerifyManifest` requests remain unchanged.
- Existing `mbx run`, MCP `minibox_run`, and ordinary container lifecycle behavior remain unchanged.
- The protocol change is additive but requires all exhaustive `DaemonRequest`/`DaemonResponse` matches and protocol drift locks to be updated together.
- No feature flag is required because the default file adapter uses existing dependencies and is part of daemon state integrity.
- The client-surface slice starts only after the daemon capability is released or tested from the same workspace revision.

## Doublecheck

- No circular dependency is introduced: `minibox-core` defines contracts, `minibox` implements them, and `miniboxd` composes them.
- Audit persistence is independent of transient `ContainerRecord` persistence, so runtime cleanup cannot erase evidence.
- Existing run preparation is reused rather than duplicated, preserving adapter behavior.
- Operator policy is evaluated separately, so a request cannot weaken it through merge semantics.
- Raw environment values and output are excluded from retained records.
- The design introduces no external dependency and no new crate.

## Risk Summary

- **Breaking API changes**: No intended source break; public protocol enums gain variants, so external exhaustive matches must update.
- **New external dependency**: No.
- **Feature flag required**: No.
- **Highest implementation risk**: extracting reusable run/cleanup internals without changing existing ephemeral-run behavior.
- **Highest operational risk**: final audit persistence failure after execution; this is fail-closed before launch and loudly surfaced after cleanup.
