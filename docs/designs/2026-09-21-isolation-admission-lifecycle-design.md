---
source_sha: f9aa9a79348227ca3c517fe7ea86b3bc6eb16829
status: proposed
sources:
  - crates/minibox-domain/src/runtime.rs
  - crates/minibox-domain/src/execution_manifest.rs
  - crates/minibox/src/daemon/handler/mod.rs
  - crates/minibox/src/daemon/handler/run.rs
  - crates/minibox/src/daemon/handler/run/model.rs
  - crates/minibox/src/daemon/handler/run/request.rs
  - crates/minibox/src/daemon/handler/run/preparation.rs
  - crates/minibox/src/daemon/state.rs
  - crates/miniboxd/src/config.rs
  - docs/designs/2026-09-21-isolation-profile-contract-design.md
generated: 2026-09-21
---

# Design: Isolation Admission And Transaction Lifecycle

## Goal

Guarantee that policy resolution and planning happen before per-container mutation, that workload
release requires validated staged evidence, and that every failure preserves cleanup ownership.

## Ownership

- **Port owner:** `minibox-domain::isolation_runtime`
- **Application owner:** `minibox::daemon::isolation`
- **Composition owner:** `miniboxd`
- **Dependency:** approved isolation profile contract

## Mutation Boundary

Allowed before planning:

- Read-only runtime capability probes.
- Image metadata/layer resolution and cache population.
- Parsing and validating operator configuration.

Forbidden before a successful plan:

- Per-container directories or rootfs state.
- Cgroups, networks, VMs, processes, or guest sessions.
- Container state records or execution manifests.
- Host lifecycle hooks.

## Effective Workload

The application resolves image defaults, caller overrides, and profile defaults into one immutable
workload before admission.

```rust
pub struct EffectiveIsolationWorkload {
    kind: IsolationWorkloadKind,
    image_ref: String,
    image_layers: Vec<IsolationDigest>,
    command: Vec<String>,
    environment: Vec<ExecutionManifestEnvVar>,
    hostname: String,
    tty: bool,
    network: NetworkMode,
    mounts: Vec<BindMount>,
    resources: ResourceConfig,
    identity: ResolvedWorkloadIdentity,
    cgroup_parent: Option<InternalPath>,
}
```

Image `USER` and caller user names are resolved to numeric UID/GID before admission. The source is
recorded as image config, caller override, or profile default; callers cannot self-assert the source.

Host lifecycle hooks are not part of `standard-v1`: a non-empty hook request is rejected. An
authorized compatibility run records host hooks as an explicit degradation and executes them only
through the aggregate runtime's rollback transaction.

## Operator Admission

```rust
pub struct IsolationAdmissionPolicy {
    compatibility_authorization: Option<OperatorAuthorizationId>,
    capability_gate: LinuxCapabilityGate,
    host_mount_policy: HostMountPolicy,
    legacy_privileged_authorization: Option<OperatorAuthorizationId>,
}

pub fn resolve_isolation_mode(
    policy: &IsolationAdmissionPolicy,
    request: &RunIsolationRequest,
    workload: &EffectiveIsolationWorkload,
    runtime: &IsolationRuntimeCapabilities,
) -> Result<ResolvedIsolationMode, IsolationError>;
```

The default policy denies compatibility, capability additions, host mounts, and legacy privileged
execution. Pipeline `PolicyOverride` may narrow but cannot widen this policy.

## Daemon Configuration

```rust
pub struct IsolationConfig {
    compatibility_authorization: Option<String>,
    capability_additions: CapabilityAdditionConfig,
    legacy_privileged_authorization: Option<String>,
    host_mount_prefixes: Vec<PathBuf>,
    host_mount_access: HostMountAccess,
}
```

`DaemonConfig` contains `#[serde(default)] isolation: IsolationConfig`. `IsolationConfig::default()`
denies every optional authorization and host mount. Config loading returns `Result`; malformed
profiles, capabilities, authorization IDs, or paths are startup errors.

Existing `allow_privileged=true` and `MINIBOX_ALLOW_PRIVILEGED=true` inputs are accepted during
migration and normalized to a deterministic authorization ID derived from configuration source and
digest.

## Binding Sequence

1. Allocate a container ID in memory without creating resources.
2. Derive deterministic container/run paths from validated daemon state and container ID.
3. Build manifest v2 workload, profile, request, and path projections.
4. Create `PrePlanManifestBinding` containing container ID, paths, capture/TTY mode, and workload,
   policy, and request digests.
5. Ask the selected runtime to plan without mutation.
6. Bind plan digest and runtime generation into `ExecutionManifestBinding`.
7. Persist the planned record.
8. Stage resources and controls.
9. Validate evidence and subject identity.
10. Persist staged evidence.
11. Release workload execution.
12. Persist enforced state and return the accepted summary.

## Aggregate Port

```rust
#[async_trait]
pub trait IsolationRuntime: Send + Sync {
    async fn capabilities(&self) -> Result<IsolationRuntimeCapabilities, IsolationError>;

    async fn plan(
        &self,
        request: &IsolationPlanRequest,
    ) -> Result<IsolationPlan, IsolationError>;

    async fn stage(
        &self,
        request: IsolationStageRequest,
        plan: IsolationPlan,
        binding: ExecutionManifestBinding,
    ) -> Result<StagedIsolation, IsolationStageFailure>;

    async fn release(
        &self,
        staged: ValidatedStagedIsolation,
    ) -> Result<IsolatedSpawn, IsolationReleaseFailure>;

    async fn wait(&self, handle: &IsolationHandle) -> Result<IsolationExit, IsolationError>;
    async fn stop(&self, handle: &IsolationHandle) -> Result<(), IsolationError>;
    async fn cleanup(&self, handle: &IsolationHandle) -> Result<CleanupEvidence, IsolationError>;
}
```

The aggregate implementation owns filesystem, network, limiter, raw runtime, and rollback ports.
Production run handlers do not receive those mutation ports independently.

## Staged Subject And Typestate

```rust
pub struct IsolationSubject {
    process_id: Option<u32>,
    process_start_time_ticks: Option<u64>,
    guest_subject_id: Option<String>,
    subject_generation: IsolationDigest,
}

pub struct StagedIsolation {
    handle: IsolationHandle,
    subject: IsolationSubject,
    evidence: IsolationEvidence,
}

pub struct ValidatedStagedIsolation(StagedIsolation);

pub fn validate_staged_isolation(
    plan: &IsolationPlan,
    binding: &ExecutionManifestBinding,
    staged: StagedIsolation,
) -> Result<ValidatedStagedIsolation, IsolationValidationFailure>;
```

Only validation constructs `ValidatedStagedIsolation`. Release consumes that value and derives the
returned `IsolatedSpawn` from the same handle, subject, and evidence; adapters cannot substitute a
new process or evidence object after validation.

## Failure And Cleanup Ownership

```rust
pub struct PartialIsolationAllocation {
    handle: IsolationHandle,
    allocated_controls: BTreeSet<IsolationControl>,
}

pub struct IsolationStageFailure {
    error: IsolationError,
    partial: Option<PartialIsolationAllocation>,
    rollback: RollbackStatus,
}

pub enum RollbackStatus {
    Complete,
    CleanupRequired,
}
```

- `Complete` means the adapter proved no allocation remains.
- `CleanupRequired` carries a handle and allocated-control set. The daemon persists recovery state
  and retries idempotent cleanup.
- Validation failure returns the staged value.
- Release failure returns the handle and subject.
- Cleanup borrows the handle so it can be retried.
- Every cleanup attempt produces `CleanupEvidence` and a `CleanupV1` digest.

## Persisted State Machine

```text
planned -> staged -> enforced -> exited -> cleaned
    |         |          |          |
    +---------+----------+----------+-> failed-cleanup-pending -> cleaned

legacy-authorized -> legacy-running -> exited -> cleaned
```

Illegal transitions fail closed. Compatibility is represented by profile plus operator
authorization, not a separate status. Legacy mode has its own canonical policy digest and evidence
mode so bindings are never optional.

## Secondary Workload Surfaces

- Image-build RUN and pipeline-step launches use the same admitted launch port.
- Restart/update rebuilds and re-admits the stored request.
- Profile-managed exec and snapshot restore reject until dedicated admitted ports are designed.
- Raw runtime/checkpoint ports cannot be used as fallback.

## Verification

- Failure injection at every allocation and rollback edge.
- Mutation counters prove zero mutation before planning.
- Compile-fail tests prove unvalidated typestate cannot be released.
- Subject identity tests prove PID/start-generation or guest ID stability through release.
- Repeated cleanup tests prove idempotence and durable recovery ownership.
- Hook and TTY fields participate in request/workload digests.

## Out Of Scope

- Platform-specific control application.
- Wire protocol and client behavior.
- Adapter qualification criteria beyond the aggregate port contract.
