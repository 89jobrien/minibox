---
source_sha: 06f61a79ec807b05238479050a4d43864ec9bf3b
source_scope: committed-tree
status: proposed
approval_required: true
generated: 2026-09-18
sources:
  - Cargo.toml
  - crates/minibox-domain/Cargo.toml
  - crates/minibox-domain/src/execution_manifest.rs
  - crates/minibox-domain/src/lib.rs
  - crates/minibox-domain/src/runtime.rs
  - crates/minibox-core/src/preflight.rs
  - crates/minibox-core/src/protocol.rs
  - crates/minibox/src/adapters/colima.rs
  - crates/minibox/src/adapters/gke.rs
  - crates/minibox/src/adapters/runtime.rs
  - crates/minibox/src/adapters/smolvm.rs
  - crates/minibox/src/container/cgroups.rs
  - crates/minibox/src/container/mount_seccomp.rs
  - crates/minibox/src/container/namespace.rs
  - crates/minibox/src/container/process.rs
  - crates/minibox/src/daemon/handler/mod.rs
  - crates/minibox/src/daemon/handler/pipeline.rs
  - crates/minibox/src/daemon/handler/run.rs
  - crates/minibox/src/daemon/handler/run/model.rs
  - crates/minibox/src/daemon/handler/run/preparation.rs
  - crates/minibox/src/daemon/handler/run/request.rs
  - crates/minibox/src/daemon/server.rs
  - crates/minibox/src/daemon/state.rs
  - crates/miniboxd/src/adapter_registry.rs
  - crates/miniboxd/src/config.rs
  - crates/miniboxd/src/main.rs
  - crates/macbox/src/krun/runtime.rs
  - crates/macbox/src/lib.rs
  - crates/macbox/src/vz/adapter.rs
  - crates/macbox/src/vz/agent_init.rs
  - crates/macbox/src/vz/proxy.rs
  - crates/mbx/src/commands/doctor.rs
  - crates/mcp/src/tools/containers.rs
  - crates/minibox-crux-plugin/src/lib.rs
  - crates/minibox-testsuite/src/adapters/runtime.rs
  - crates/smolbox/src/krun/mod.rs
  - crates/smolbox/src/lib.rs
  - crates/smolbox/src/smolvm/mod.rs
  - docs/designs/2026-09-12-guaranteed-security-profiles-design.md
---

# Design: Environment-Attested Isolation

## Status

This design is proposed and requires explicit user approval before planning or implementation.
It takes Environment-Attested Isolation and portable outcomes as session inputs, but makes no
durable approval claim; approval of this document is the planning gate.

## Goal

Make hostile multi-tenant runtime isolation the secure default for every Unix-selectable Minibox
adapter, using direct kernel enforcement or verifiable substrate and measured-guest evidence while
supporting root-owned and rootless daemon deployments.

## Approved Approach

Minibox derives one complete effective workload, verifies environment-specific evidence against a
portable policy, durably reserves resources, stages the workload behind an execution latch, verifies
the staged subject, durably authorizes release, and only then opens the latch. Trusted compatibility
is an immutable daemon startup mode and cannot be selected by a workload request.

## Contents

- [Decisions And Scope](#decisions-and-scope)
- [Terminology](#terminology)
- [Context Map](#context-map)
- [Architecture](#architecture)
- [Public API](#public-api)
- [Private Durable Ledger](#private-durable-ledger)
- [Data Flow](#data-flow)
- [Adapter Qualification](#adapter-qualification)
- [Protocol And Persistence](#protocol-and-persistence)
- [Verification](#verification)
- [Compatibility And Risks](#compatibility-and-risks)
- [Acceptance Criteria](#acceptance-criteria)

## Decisions And Scope

### Explicit Constraints

- In-scope adapters: `native`, `gke`, `colima`, `smolvm`, `krun`, and feature-gated `vz`.
- Hostile multi-tenant mode is the final public default.
- Trusted compatibility is selectable only at daemon startup and uses a separate state root.
- Root-owned and rootless daemon modes qualify separately.
- Admission fails before workload resource allocation.
- Workload instructions remain blocked until staged evidence and durable release authorization exist.
- Weak adapter mechanisms may be replaced while adapter names remain stable.
- No adapter is called production-qualified until all six pass their applicable real-environment
  matrices.

### Portable Security Decision

The portable contract specifies outcomes rather than requiring identical Linux mechanisms. Native
Linux uses `no_new_privileges`, capability ceilings, and default-deny seccomp. VM adapters combine a
hypervisor boundary with a measured guest authority. GKE uses a qualified gVisor boundary and
provider controls because GKE Sandbox documents seccomp and `NoNewPrivileges` as incompatible:
[GKE Sandbox concepts](https://cloud.google.com/kubernetes-engine/docs/concepts/sandbox-pods).

Required hostile outcomes are:

- host-root separation;
- process, mount, filesystem, and network isolation;
- privilege-escalation prevention and a least-privilege capability ceiling;
- restricted host-kernel attack surface;
- finite memory and process limits;
- exact enforcement of requested CPU or I/O controls when an adapter accepts those options.

An adapter may reject an optional CPU or I/O request it cannot enforce without losing baseline
qualification. It may not accept and ignore the request.

### Out Of Scope

- Windows, WSL2, HCS, and Named Pipe qualification.
- Signed workload authorization, tenant policy bundles, and image-signature policy.
- Confidential-computing attestation, compromised host kernels or hypervisors, physical attacks,
  and microarchitectural side channels.
- Fair scheduling beyond finite per-workload resource controls.
- Hostile snapshot restore and Dockerfile build until separate execution-path designs qualify them.

## Terminology

| Term                   | Meaning                                                                                   |
| ---------------------- | ----------------------------------------------------------------------------------------- |
| Environment epoch      | Digest of security-relevant runtime, configuration, trust-root, and substrate state       |
| Reservation            | Durable ownership claim created before any external resource mutation                     |
| Ownership selector     | Deterministic label derived from a reservation and attached to every resource             |
| Observed locator       | Authoritative identity learned after a resource exists, such as PID generation or Pod UID |
| Stage permit           | Non-cloneable authority to allocate and stage one exact workload                          |
| Release permit         | Non-cloneable authority minted only after staged evidence is durably committed            |
| Cleanup permit         | Fenced, leased authority to remove one reservation's resources                            |
| Execution latch        | Backend mechanism that prevents workload instructions from running before release         |
| Qualification manifest | Signed detached artifact proving an exact released binary passed a named matrix           |

## Context Map

### Current Gaps

- `RuntimeCapabilities` is static metadata and does not participate in admission.
- Run preparation allocates rootfs, cgroup, and networking before runtime spawn.
- Native execution lacks user-namespace mapping, default capability dropping, and general seccomp.
- GKE uses proot, copied filesystems, and no-op resource/network adapters.
- Colima uses a shared VM; SmolVM and krun omit several effective controls.
- VZ forwards unauthenticated newline-delimited JSON over vsock.
- Adapter selection can fall back after failure.
- State persistence is best-effort JSON and has no crash-durable isolation transaction.
- Pipeline policy overrides and direct raw-runtime dependencies can bypass a future central path.

### Ownership Map

| Component                         | Owner                        | Responsibility                                                                           |
| --------------------------------- | ---------------------------- | ---------------------------------------------------------------------------------------- |
| Portable policy and evidence DTOs | `minibox-domain`             | Pure validated values; no store or infrastructure authority                              |
| Isolation transaction and ledger  | `minibox`                    | Sole mutation authority, permits, WAL, reconciliation, native/GKE/Colima/SmolVM adapters |
| krun and VZ host adapters         | `macbox`                     | Hypervisor-specific enforcement and discovery                                            |
| Guest enforcement                 | new `minibox-guest-agent`    | Measured guest controls, evidence, latch, and release acknowledgement                    |
| GKE release controller            | new `minibox-gke-controller` | Immutable hold dependencies and authoritative release transaction                        |
| Composition                       | `miniboxd`                   | Startup mode, identity, qualification manifest, and concrete backend selection           |
| Wire reporting                    | `minibox-core`, `mbx`        | Read-only status and structured denial reporting                                         |
| Qualification                     | `minibox-testsuite`          | Shared non-skippable contract plus real-environment matrices                             |

### Bounded Delivery Components

Each implementation component has at most three owning crates and consumes a frozen predecessor API:

1. Contract: `minibox-domain`, `minibox`, `minibox-testsuite`.
2. Admission: `minibox-core`, `minibox`, `miniboxd`.
3. Native: `minibox-domain`, `minibox`, `minibox-testsuite`.
4. Guest: `minibox-domain`, `minibox-guest-agent`, `minibox-testsuite`.
5. VM host adapters: `minibox`, `macbox`, `smolbox`.
6. GKE: `minibox-domain`, `minibox`, `minibox-gke-controller`.
7. Reporting: `minibox-core`, `miniboxd`, `mbx`.

Incomplete components remain behind an internal non-default `isolation-v1-preview` feature. The
feature is removed before the final hostile-default release.

### Test Surfaces

- Domain digest, policy, phase, and evidence vectors.
- Handler denial-before-allocation and failure-injection tests.
- Native namespace, capability, seccomp, cgroup, mount, and network tests.
- Real GKE held-Pod race tests.
- Colima, SmolVM, krun, and VZ dedicated-VM and channel tests.
- Protocol evolution, state migration, crash recovery, and doctor reporting tests.

## Architecture

### Boundary Placement

`minibox-domain` owns only serializable facts and pure validation. Infrastructure-facing ports,
authority tokens, and the durable ledger live in `minibox::daemon::isolation`; this lets `minibox`
keep token constructors and ledger commit witnesses crate-private while `macbox` implements public
ports using read-only token views.

`IsolationService` uniquely owns `Box<dyn IsolationLedger>`. No `Arc`, clone, or public ledger
mutation handle escapes the service. A scoped `MutationJournal` capability can append observed
resource transitions for one reservation but cannot authorize release, cleanup, or lifecycle CAS.

### Evidence Phases

Evidence uses distinct types, not option-heavy shared records:

1. Startup evidence proves prerequisites and produces an environment epoch.
2. Workload evidence maps every policy requirement to a mechanism and authority before allocation.
3. Staged evidence proves the concrete blocked subject and all required outcomes.
4. Cleanup evidence proves every reserved or observed resource is absent.

Startup evidence never claims per-workload enforcement. All workload requirements must appear exactly
once in staged evidence. Hypervisor and measured-guest assertions may jointly satisfy one requirement
only when its policy mapping explicitly requires composite authority.

### Lifecycle State Machine

```text
EnvironmentAdmitted
  -> Reserved
  -> Staging
  -> Staged
  -> ReleaseAuthorized
  -> Released
  -> Terminal
  -> Cleaning
  -> Cleaned
```

Any nonterminal state may transition to `Quarantined`. `Aborted` is a terminal execution outcome that
still transitions through `Cleaning -> Cleaned`. Every transition is a ledger CAS over reservation,
environment epoch, predecessor digest, lifecycle state, and sequence.

`ReleaseAuthorized` means durable permission exists but the latch may still be closed. `Released`
requires an authenticated backend acknowledgement. Recovery never retries an ambiguous release: it
queries the subject, adopts a proven running subject, or stops and cleans it.

## Public API

### Domain Identity And Policy

These types live in `minibox_domain::isolation`.

```rust
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum IsolationMode {
    HostileMultiTenantV1,
    TrustedCompatibilityV1,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdRange {
    start: u32,
    count: u32,
}

impl IdRange {
    pub fn try_new(start: u32, count: u32) -> Result<Self, IsolationError>;
    pub const fn start(&self) -> u32;
    pub const fn count(&self) -> u32;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootlessIdentity {
    dto: RootlessIdentityDto,
}

impl RootlessIdentity {
    pub fn try_from_dto(dto: RootlessIdentityDto) -> Result<Self, IsolationError>;
    pub const fn dto(&self) -> &RootlessIdentityDto;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DaemonIdentity {
    RootOwned { uid: u32, gid: u32 },
    Rootless(RootlessIdentity),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum IsolationRequirement {
    HostRootSeparation,
    ProcessBoundary,
    MountBoundary,
    FilesystemContainment,
    PrivateNetwork,
    CapabilityCeiling,
    PrivilegeEscalationPrevention,
    KernelAttackSurfaceRestriction,
    MemoryLimit { maximum_bytes: u64 },
    ProcessLimit { maximum_processes: u64 },
    CpuWeight { weight: u64 },
    IoBandwidthLimit { maximum_bytes_per_second: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IsolationPolicy {
    mode: IsolationMode,
    requirements: Vec<IsolationRequirement>,
    digest: IsolationDigest,
}

impl IsolationPolicy {
    pub fn try_from_dto(dto: IsolationPolicyDto) -> Result<Self, IsolationError>;
    pub const fn mode(&self) -> IsolationMode;
    pub fn requirements(&self) -> &[IsolationRequirement];
    pub const fn digest(&self) -> &IsolationDigest;
}
```

`IsolationPolicy`, `IdRange`, `RootlessIdentity`, and `DaemonIdentity` implement Serde only through
versioned DTO conversion. Deserialization validates unique requirements, required hostile outcomes,
resource values, non-overlap, reserved identities, helper provenance, nonzero ranges, and digests.

### Effective Workload And Plaintext

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EffectiveWorkloadDto {
    pub schema_version: u32,
    pub operation: IsolationOperation,
    pub image_ref: String,
    pub image_digest: IsolationDigest,
    pub argv: Vec<String>,
    pub environment_bindings: Vec<EnvironmentBinding>,
    pub mounts: Vec<BindMount>,
    pub resources: EffectiveResourceLimits,
    pub network: NetworkMode,
    pub hostname: String,
    pub user: EffectiveUser,
    pub working_directory: String,
    pub tty: bool,
    pub auto_remove: bool,
    pub cgroup_parent: Option<CanonicalInternalPath>,
    pub platform: Option<String>,
    pub capture_output: bool,
    pub workload_digest: IsolationDigest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveWorkload(EffectiveWorkloadDto);

impl EffectiveWorkload {
    pub fn try_from_dto(dto: EffectiveWorkloadDto) -> Result<Self, IsolationError>;
    pub fn dto(&self) -> &EffectiveWorkloadDto;
    pub const fn digest(&self) -> &IsolationDigest;
}

pub struct PlaintextEnvironment(zeroize::Zeroizing<Vec<String>>);

impl PlaintextEnvironment {
    pub fn try_new(entries: Vec<String>) -> Result<Self, IsolationError>;
    pub fn entries(&self) -> &[String];
    pub fn into_entries(self) -> zeroize::Zeroizing<Vec<String>>;
}

pub struct IsolationExecutionRequest {
    pub request_id: String,
    pub container_id: String,
    pub workload: EffectiveWorkload,
    pub environment: PlaintextEnvironment,
    pub image_layers: Vec<CanonicalInternalPath>,
}
```

`PlaintextEnvironment` has a redacted custom `Debug`. Restart persists a complete versioned
owner-only `RestartInputRecord`, including plaintext environment, and verifies every binding against
the canonical effective workload before fresh admission. Legacy records lacking the complete record
cannot restart in hostile mode.

### Phase-Specific Evidence

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawEvidenceBlob {
    media_type: String,
    digest: IsolationDigest,
    bytes: Vec<u8>,
}

impl RawEvidenceBlob {
    pub fn try_new(
        media_type: String,
        bytes: Vec<u8>,
        maximum_bytes: usize,
    ) -> Result<Self, IsolationError>;
    pub fn media_type(&self) -> &str;
    pub const fn digest(&self) -> &IsolationDigest;
    pub fn bytes(&self) -> &[u8];
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StartupEvidence {
    pub environment_epoch: IsolationDigest,
    pub backend_id: String,
    pub backend_instance_id: String,
    pub daemon_boot_id: IsolationDigest,
    pub prerequisite_assertions: Vec<IsolationAssertion>,
    pub evidence_digest: IsolationDigest,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkloadPlanEvidence {
    pub environment_epoch: IsolationDigest,
    pub workload_digest: IsolationDigest,
    pub policy_digest: IsolationDigest,
    pub mappings: Vec<RequirementMapping>,
    pub evidence_digest: IsolationDigest,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StagedIsolationEvidence {
    pub environment_epoch: IsolationDigest,
    pub reservation_id: IsolationDigest,
    pub workload_digest: IsolationDigest,
    pub subject_generation: IsolationDigest,
    pub assertions: Vec<IsolationAssertion>,
    pub proofs: Vec<IsolationProof>,
    pub evidence_digest: IsolationDigest,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CleanupEvidence {
    pub environment_epoch: IsolationDigest,
    pub reservation_id: IsolationDigest,
    pub fencing_token: u64,
    pub absent_resources: Vec<ResourceIdentity>,
    pub evidence_digest: IsolationDigest,
}

#[derive(Debug, Clone)]
pub struct Attested<T> {
    pub evidence: T,
    pub raw_blobs: Vec<RawEvidenceBlob>,
}
```

`IsolationProof` is a tagged enum with native-kernel, Kubernetes-controller, hypervisor, and
measured-guest variants. VM policy mappings assign host separation and physical resource ceilings to
the hypervisor authority, and guest capability, syscall, mount, and process controls to a guest key
bound to the measured boot chain. Composite mappings require both proofs.

### Reservations And Observed Resources

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceReservation {
    reservation_id: IsolationDigest,
    ownership_selector: IsolationDigest,
    backend_instance_id: String,
    expected_kinds: Vec<ResourceKind>,
}

impl ResourceReservation {
    pub fn try_new(input: ResourceReservationDto) -> Result<Self, IsolationError>;
    pub const fn reservation_id(&self) -> &IsolationDigest;
    pub const fn ownership_selector(&self) -> &IsolationDigest;
    pub fn backend_instance_id(&self) -> &str;
    pub fn expected_kinds(&self) -> &[ResourceKind];
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum ResourceLocator {
    Process { pid: u32, start_time_ticks: u64 },
    Cgroup { canonical_path: CanonicalInternalPath, inode: u64 },
    HostPath { canonical_path: CanonicalInternalPath, inode: u64 },
    KubernetesObject {
        api_group: String,
        namespace: String,
        name: String,
        uid: String,
        resource_version: String,
    },
    VirtualMachine { instance_id: String, boot_id: String },
    Network { network_id: String, generation: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedResource {
    resource_kind: ResourceKind,
    locator: ResourceLocator,
    ownership_selector: IsolationDigest,
    subject_generation: IsolationDigest,
}

impl ObservedResource {
    pub fn try_new(input: ObservedResourceDto) -> Result<Self, IsolationError>;
    pub const fn resource_kind(&self) -> &ResourceKind;
    pub const fn locator(&self) -> &ResourceLocator;
    pub const fn ownership_selector(&self) -> &IsolationDigest;
    pub const fn subject_generation(&self) -> &IsolationDigest;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceInventory {
    dto: ResourceInventoryDto,
}

impl ResourceInventory {
    pub fn try_from_dto(dto: ResourceInventoryDto) -> Result<Self, IsolationError>;
    pub const fn reservation(&self) -> &ResourceReservation;
    pub fn observed(&self) -> &[ObservedResource];
    pub const fn dto(&self) -> &ResourceInventoryDto;
}
```

The ledger commits a deterministic reservation before mutation. Each resource receives the ownership
selector at creation. The scoped mutation journal appends the typed observed locator immediately
after creation. If the process crashes between those steps, discovery searches by ownership selector.

### Application Ports And Authority Tokens

These APIs live in `minibox::daemon::isolation`.

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReleaseAcknowledgement {
    pub reservation_id: IsolationDigest,
    pub release_authorization_digest: IsolationDigest,
    pub environment_epoch: IsolationDigest,
    pub subject_generation: IsolationDigest,
    pub observed_at_unix_millis: i64,
    pub proof: IsolationProof,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionObservation {
    pub reservation_id: IsolationDigest,
    pub state: ObservedExecutionState,
    pub subject_generation: IsolationDigest,
    pub proof: IsolationProof,
}

#[async_trait::async_trait]
pub trait EnvironmentAttestor: Send + Sync {
    async fn attest_startup(
        &self,
        request: &StartupAttestationRequest,
    ) -> Result<Attested<StartupEvidence>, IsolationError>;

    async fn attest_workload(
        &self,
        request: &WorkloadAttestationRequest,
    ) -> Result<Attested<WorkloadPlanEvidence>, IsolationError>;

    async fn attest_staged(
        &self,
        request: &StagedAttestationRequest,
    ) -> Result<Attested<StagedIsolationEvidence>, IsolationError>;

    async fn attest_cleanup(
        &self,
        request: &CleanupAttestationRequest,
    ) -> Result<Attested<CleanupEvidence>, IsolationError>;
}

#[async_trait::async_trait]
pub trait IsolationEnforcer: Send + Sync {
    async fn stage(
        &self,
        request: IsolationExecutionRequest,
        permit: StagePermit,
        journal: MutationJournal,
    ) -> Result<StagedExecution, IsolationStageError>;

    async fn release(
        &self,
        staged: StagedExecution,
        permit: ReleasePermit,
    ) -> Result<ReleasedExecution, IsolationReleaseError>;

    async fn inspect(
        &self,
        handle: &WorkloadHandle,
    ) -> Result<ExecutionObservation, IsolationError>;

    async fn wait_for_exit(
        &self,
        handle: &WorkloadHandle,
    ) -> Result<ExitStatus, IsolationError>;

    async fn stop(&self, handle: &WorkloadHandle) -> Result<(), IsolationError>;

    async fn cleanup(
        &self,
        handle: WorkloadHandle,
        permit: CleanupPermit,
        journal: CleanupJournal,
    ) -> Result<ResourceInventory, IsolationCleanupError>;
}

#[async_trait::async_trait]
pub trait IsolationResourceDiscovery: Send + Sync {
    async fn discover(
        &self,
        ownership_selector: &IsolationDigest,
    ) -> Result<Vec<ObservedResource>, IsolationError>;
}

pub struct StagePermit {
    binding: PermitBinding,
    reservation: ResourceReservation,
    expires_at: std::time::Instant,
}

pub struct ReleasePermit {
    binding: PermitBinding,
    release_authorization_digest: IsolationDigest,
    expected_subject_generation: IsolationDigest,
    expires_at: std::time::Instant,
}

pub struct CleanupPermit {
    reservation_id: IsolationDigest,
    environment_epoch: IsolationDigest,
    fencing_token: u64,
    lease_expires_at: std::time::Instant,
}

pub struct MutationJournal {
    scope: ScopedLedgerSender,
}

pub struct CleanupJournal {
    scope: ScopedLedgerSender,
    fencing_token: u64,
}

impl MutationJournal {
    pub async fn append_observed(
        &self,
        resource: ObservedResource,
    ) -> Result<(), IsolationError>;
}

impl CleanupJournal {
    pub async fn validate_fence(&self) -> Result<(), IsolationError>;
    pub async fn append_removed(
        &self,
        resource: &ObservedResource,
    ) -> Result<(), IsolationError>;
}

impl StagePermit {
    pub const fn binding(&self) -> &PermitBinding;
    pub const fn reservation(&self) -> &ResourceReservation;
    pub fn is_expired(&self, now: std::time::Instant) -> bool;
}

impl ReleasePermit {
    pub const fn binding(&self) -> &PermitBinding;
    pub const fn release_authorization_digest(&self) -> &IsolationDigest;
    pub const fn expected_subject_generation(&self) -> &IsolationDigest;
    pub fn is_expired(&self, now: std::time::Instant) -> bool;
}

impl CleanupPermit {
    pub const fn reservation_id(&self) -> &IsolationDigest;
    pub const fn environment_epoch(&self) -> &IsolationDigest;
    pub const fn fencing_token(&self) -> u64;
    pub fn is_expired(&self, now: std::time::Instant) -> bool;
}

#[derive(Debug, thiserror::Error)]
#[error("release failed: {source}")]
pub struct IsolationReleaseError {
    #[source]
    pub source: IsolationError,
    pub staged: StagedExecution,
}

#[derive(Debug, thiserror::Error)]
#[error("cleanup failed: {source}")]
pub struct IsolationCleanupError {
    #[source]
    pub source: IsolationError,
    pub handle: WorkloadHandle,
}
```

Permit and scoped-journal fields are private; constructors are `pub(crate)` and exist only beside
`IsolationService`. Public getters let external adapter crates validate bindings but cannot mint
authority. `CleanupJournal::validate_fence` asks the ledger actor to confirm the current lease before
every destructive removal.

`IsolationStageError` is the only stage failure type:

```rust
#[derive(Debug, thiserror::Error)]
pub enum IsolationStageError {
    #[error("staging failed before allocation: {source}")]
    BeforeAllocation { #[source] source: IsolationError },
    #[error("staging failed after allocation: {source}")]
    AfterAllocation {
        #[source]
        source: IsolationError,
        partial: PartialAllocation,
    },
}
```

`IsolationService::execute` consumes either failure. It verifies absence after `BeforeAllocation`; for
`AfterAllocation`, it persists the typed partial inventory, acquires a cleanup lease, and cleans or
quarantines before returning `IsolationExecutionError`. No cleanup ownership escapes accidentally.
`IsolationExecutionRequest`, `StagePermit`, `ReleasePermit`, `CleanupPermit`, `StagedExecution`,
`ReleasedExecution`, `IsolationBackend`, and `IsolationService` provide custom redacted `Debug`
implementations. All public stage, release, cleanup, and execution errors implement `Debug`,
`Display`, and `std::error::Error`, preserving their nested `IsolationError` as `source()` and
returning the owned staged execution or workload handle needed for recovery.

### Backend And Service

```rust
pub struct IsolationBackend {
    attestor: Box<dyn EnvironmentAttestor>,
    enforcer: Box<dyn IsolationEnforcer>,
    discovery: Box<dyn IsolationResourceDiscovery>,
}

impl IsolationBackend {
    pub fn try_new(
        binding: BackendBinding,
        attestor: Box<dyn EnvironmentAttestor>,
        enforcer: Box<dyn IsolationEnforcer>,
        discovery: Box<dyn IsolationResourceDiscovery>,
    ) -> Result<Self, IsolationError>;
}

pub struct IsolationService {
    context: IsolationContext,
    backend: IsolationBackend,
    ledger: LedgerHandle,
    qualification: VerifiedQualificationManifest,
}

impl IsolationService {
    pub async fn initialize(
        context: IsolationContext,
        backend: IsolationBackend,
        state_root: &std::path::Path,
        qualification_manifest: &std::path::Path,
    ) -> Result<Self, IsolationError>;

    pub async fn preflight(
        &self,
        request: &IsolationExecutionRequest,
    ) -> Result<IsolationPreflightReport, IsolationError>;

    pub async fn execute(
        &self,
        request: IsolationExecutionRequest,
    ) -> Result<ReleasedExecution, IsolationExecutionError>;

    pub async fn wait_for_exit(
        &self,
        execution: &ReleasedExecution,
    ) -> Result<ExitStatus, IsolationExecutionError>;

    pub async fn stop(&self, handle: &WorkloadHandle) -> Result<(), IsolationError>;

    pub async fn cleanup(
        &self,
        handle: WorkloadHandle,
    ) -> Result<CleanupEvidence, IsolationCleanupError>;

    pub async fn reconcile(&self) -> Result<ReconciliationReport, IsolationError>;

    pub fn status(&self) -> IsolationStatusReport;
}
```

Production initialization constructs the private file ledger and starts one ledger actor that owns
it. `LedgerHandle` is crate-private and held only by the service; scoped journal senders expose only
reservation-bound append or fence-validation commands. Tests use a crate-private constructor with an
in-memory actor. `preflight`, `execute`, and `status` recheck qualification expiry; expiry immediately
removes the backend from admission while allowing stop and cleanup.

## Private Durable Ledger

`IsolationLedger` is `pub(crate)` in `minibox` and is not re-exported. One actor task uniquely owns
its `Box<dyn IsolationLedger>` and processes typed commands serially. `LedgerHandle` is a private
sender available only to `IsolationService`; `MutationJournal` and `CleanupJournal` contain narrower
reservation-bound senders. Commit witnesses have private fields and crate-private constructors.
They are not Serde types and cannot cross protocol boundaries.

```rust
#[async_trait::async_trait]
pub(crate) trait IsolationLedger: Send {
    async fn persist_startup(
        &mut self,
        record: StartupAdmissionRecord,
        blobs: Vec<RawEvidenceBlob>,
    ) -> Result<StartupCommit, IsolationError>;

    async fn reserve(
        &mut self,
        expected_epoch: &IsolationDigest,
        record: ReservationRecord,
        evidence: WorkloadPlanEvidence,
        blobs: Vec<RawEvidenceBlob>,
    ) -> Result<ReservationCommit, IsolationError>;

    async fn append_observed(
        &mut self,
        reservation: &IsolationDigest,
        expected_sequence: u64,
        resource: ObservedResource,
    ) -> Result<MutationCommit, IsolationError>;

    async fn commit_staged(
        &mut self,
        expected: LifecyclePredecessor,
        record: StagedRecord,
        blobs: Vec<RawEvidenceBlob>,
    ) -> Result<StagedCommit, IsolationError>;

    async fn authorize_release(
        &mut self,
        expected: LifecyclePredecessor,
        record: ReleaseAuthorizationRecord,
    ) -> Result<ReleaseAuthorizationCommit, IsolationError>;

    async fn acknowledge_release(
        &mut self,
        expected: LifecyclePredecessor,
        acknowledgement: ReleaseAcknowledgement,
    ) -> Result<ReleasedCommit, IsolationError>;

    async fn persist_terminal(
        &mut self,
        expected: LifecyclePredecessor,
        record: TerminalRecord,
    ) -> Result<TerminalCommit, IsolationError>;

    async fn acquire_cleanup_lease(
        &mut self,
        expected: LifecyclePredecessor,
        owner_boot_id: &IsolationDigest,
        lease_duration: std::time::Duration,
    ) -> Result<CleanupLeaseCommit, IsolationError>;

    async fn renew_cleanup_lease(
        &mut self,
        lease: &CleanupLeaseCommit,
        lease_duration: std::time::Duration,
    ) -> Result<CleanupLeaseCommit, IsolationError>;

    async fn finish_cleanup(
        &mut self,
        expected: LifecyclePredecessor,
        evidence: CleanupEvidence,
        blobs: Vec<RawEvidenceBlob>,
    ) -> Result<CleanedCommit, IsolationError>;

    async fn quarantine(
        &mut self,
        expected: LifecyclePredecessor,
        reason: String,
    ) -> Result<QuarantineCommit, IsolationError>;

    async fn load_incomplete(&mut self) -> Result<Vec<IncompleteRecord>, IsolationError>;

    async fn read_blob(&mut self, digest: &IsolationDigest) -> Result<Vec<u8>, IsolationError>;

    async fn collect_expired_blobs(
        &mut self,
        retention_before_unix_millis: i64,
    ) -> Result<BlobCollectionReport, IsolationError>;
}
```

The file implementation uses an append-only WAL, content-addressed evidence blobs, file and parent
directory `fsync`, atomic snapshot replacement, and an exclusive process lock. It opens the state root
descriptor-relatively, rejects symlinks and writable ancestors, verifies owner/mode/inode after open,
and creates owner-only files and directories. Blob bytes are size-checked, written, rehashed, and
fsynced before a record may reference them. Configured per-blob, per-reservation, and total-store
quotas deny writes before exhaustion; retention never deletes blobs referenced by live records or
quarantine tombstones. Terminal and cleanup transitions require an exact predecessor and return a
new chain head.

Reservation records contain deterministic ownership selectors and expected resource kinds, not
post-allocation locators. `append_observed` adds each authoritative locator and generation after the
resource exists. This two-phase model makes both pre-mutation ownership and post-crash discovery
possible.

Cleanup leases contain owner boot ID, expiry, monotonic fencing token, reservation, and predecessor.
Expired leases may be stolen only by a higher fencing token. Enforcers validate the token immediately
before every removal, preventing stale reconcilers from deleting reused resources.

No synchronous lock is held across I/O. Admission snapshots the in-memory epoch generation, performs
attestation and ledger CAS without a guard, then rechecks the generation. A changed generation causes
retry or denial. The ledger CAS is the durable serialization point.

## Data Flow

### Startup

1. `miniboxd` resolves `DaemonIdentity`, peer authentication, immutable mode, and separate state root.
2. It verifies a detached qualification manifest against the embedded release public key and its own
   executable digest.
3. `IsolationService` constructs the unique file ledger and reconciles incomplete records.
4. The attestor returns startup evidence and raw blobs.
5. The service validates evidence, writes blobs, and commits `EnvironmentAdmitted`.
6. The daemon listens only after successful reconciliation and startup commit.

### Workload

1. The handler resolves image metadata, entrypoint, user, working directory, environment, mounts,
   terminal settings, resources, network, platform, and operation into `EffectiveWorkload`.
2. The service derives policy from immutable daemon mode and verifies workload evidence.
3. It writes workload evidence blobs and commits a deterministic reservation before mutation.
4. It creates `StagePermit` and a scoped `MutationJournal` from the private reservation commit.
5. The enforcer tags every resource with the ownership selector, appends each observed locator, and
   returns a blocked `StagedExecution` or the single canonical `IsolationStageError`.
6. The attestor independently inspects the staged subject. The service validates all required
   assertions and commits `Staged` plus raw evidence.
7. The ledger CASes `Staged -> ReleaseAuthorized`; only that private commit can mint `ReleasePermit`.
8. The enforcer atomically compares subject generation and opens the execution latch, returning an
   authenticated release acknowledgement bound to reservation, release-authorization commit,
   environment epoch, subject generation, timestamp, and backend proof.
9. The service CASes `ReleaseAuthorized -> Released`. Failure to persist the acknowledgement causes
   stop, fenced cleanup, and quarantine if absence cannot be proven.
10. The handler calls `IsolationService::wait_for_exit`; the service waits through the enforcer and
    CASes the exact released predecessor to `Terminal` before returning the exit status.
11. Removal acquires a cleanup lease, removes resources, independently attests absence, and commits
    `Cleaned`.

### Recovery

1. The service loads every nonterminal record and acquires or steals a fenced cleanup lease.
2. Discovery searches by ownership selector and merges findings with journaled observed locators.
3. `ReleaseAuthorized` is queried through `IsolationEnforcer::inspect` rather than replayed: a
   matching authenticated running subject is adopted; otherwise it is stopped and cleaned.
4. Unknown resources carrying a Minibox ownership selector are quarantined.
5. Identifiers, subordinate ranges, and resource names remain reserved until `Cleaned` is committed.

## Adapter Qualification

### Native Linux

Each workload receives user, PID, mount, UTS, IPC, and network namespaces. Root-owned mode writes
disjoint subordinate UID/GID mappings. Rootless mode requires the validated scheduler identity,
root-owned non-writable helper binaries resolved by descriptor, delegated cgroup v2 controllers,
rootless-safe overlay support, and networking that denies host loopback and metadata access.

The child blocks on a private pre-exec latch after capability bounding, `no_new_privileges`, general
default-deny seccomp, pivot-root/mount hardening, and cgroup placement. The attestor verifies PID plus
start time, namespace inodes, UID/GID maps, capability sets, seccomp mode/program digest, cgroup
values, mount state, and network identity before release.

### VM Adapters

Colima, SmolVM, krun, and VZ allocate one VM per workload. Hypervisor evidence authoritatively proves
host separation, VM identity, boot artifact digests, memory ceiling, host-side disk/network topology,
and dedicated ownership. A measured guest agent is authoritative for guest process identity,
capability, no-new-privileges, syscall, mount, guest-network, and process-limit controls.

The host opens immutable boot artifacts before hashing. A boot secret binds an ephemeral guest
Ed25519 key to the hypervisor-observed boot identity. HKDF-SHA256 derives directional
ChaCha20-Poly1305 keys; transcripts bind protocol version, direction, sequence, VM identity, boot ID,
plan, and workload. Evidence and release acknowledgements are signed by the bound guest key.

### GKE

The GKE adapter replaces proot with one held GKE Sandbox Pod per workload. A new
`minibox-gke-controller` owns the release transaction. Admission policy makes the Pod spec,
per-workload NetworkPolicy, release object, launcher image digest, RuntimeClass reference, and
qualified node-pool selector immutable for the held workload lifetime.

The controller verifies those objects, actual node/runtime identity, Pod UID, node boot identity,
CNI policy generation, image digests, and GKE qualification epoch. It alone can activate the
projected one-time latch credential. The controller revalidates all immutable dependencies and
activates release in one serialized reconcile operation. If admission policy cannot prevent mutation
or deletion of any dependency, the adapter is unqualified.

Memory uses Pod cgroup limits. Process limits require a qualified node-pool `podPidsLimit` no greater
than the effective policy value. Requested CPU controls use accepted Kubernetes CPU limits. Optional
I/O bandwidth requests are rejected until GKE exposes a separately qualified mechanism.

Google documents that RuntimeClass should be checked outside the sandbox rather than inferred from
inside it: [Harden workload isolation with GKE Sandbox](https://cloud.google.com/kubernetes-engine/docs/how-to/sandbox-pods).

### Qualification Manifest

The release pipeline emits a detached signed envelope. Its `postcard` payload contains:

- schema and qualification profile versions;
- source commit and `Cargo.lock` digests;
- exact executable, adapter, guest agent, GKE controller, launcher, and boot artifact digests;
- enabled Cargo features and target triple;
- adapter, architecture, privilege-mode, kernel/hypervisor/runtime, and environment matrix;
- test-suite digest, test manifest, result Merkle root, runner identity, timestamps, and expiry;
- trust-root identifier and evidence-retention root.

The daemon embeds only the release verification public key. At startup it hashes its executable,
loads the detached manifest, verifies its signature and expiry, and requires an exact matrix entry.
The manifest digest is part of the environment epoch and `IsolationStatusReport`.

## Protocol And Persistence

Daemon configuration uses an explicit identity rather than auto-promoting a root-owned default into
rootless mode:

```rust
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum DaemonIdentityConfig {
    RootOwned,
    Rootless { identity: RootlessIdentityDto },
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct IsolationConfig {
    pub mode: IsolationMode,
    pub identity: DaemonIdentityConfig,
    pub state_root: std::path::PathBuf,
    pub evidence_ttl_millis: u64,
    pub maximum_evidence_blob_bytes: usize,
    pub maximum_reservation_evidence_bytes: u64,
    pub maximum_evidence_store_bytes: u64,
}
```

`DaemonConfig` adds `#[serde(default)] pub isolation: IsolationConfig`; its default is hostile,
root-owned operation. Rootless operation always requires an explicit validated identity. The
validated identity, helper file identities, reserved primary groups, subordinate ranges, socket
identity, and state-root identity are included in the environment epoch.

`DaemonRequest::Run` receives no mode, permit, evidence, or downgrade field. Additive responses are:

```rust
DaemonRequest::IsolationStatus

DaemonResponse::IsolationStatus {
    report: IsolationStatusReport,
}

DaemonResponse::IsolationDenied {
    code: IsolationDenialCode,
    reason: String,
}
```

`ExecutionManifest` gains optional `effective_workload` and `isolation_record` fields. `ContainerRecord`
gains optional `restart_input`, `resource_inventory`, and `isolation_summary` fields. Old records
deserialize but are unqualified and cannot restart or exec in hostile mode.

`mbx doctor` reports compiled, operational, environment-admitted, and release-qualified states
separately. Static `RuntimeCapabilities` remains advisory and never produces a qualification claim.

Hostile root-owned peer authentication remains root-only. Hostile rootless mode requires
`ExactUid(scheduler_uid)` on a systemd-activated or pre-created owner-controlled socket. Socket
validation uses descriptor-relative traversal, rejects symlinks and writable ancestors, verifies
inode and owner after bind/accept, and never unlinks an operator-owned socket.

## Verification

### Domain And Ledger

- Golden vectors for workload, policy, epoch, evidence, and qualification payload digests.
- Rejection of duplicate, missing, wrong-phase, stale, replayed, and mismatched assertions.
- Receipt-forgery compile tests: external crates cannot construct permits or ledger commits.
- CAS race, cleanup-lease fencing, allocation reservation, and identifier-reuse tests.
- Crash injection before and after every WAL write, blob write, rename, and `fsync`.
- Recovery tests where mutation succeeds before observed-locator journaling.

### Adversarial Adapter Matrices

- Native host-root, capability regain, forbidden syscall, namespace substitution, PID reuse, fork
  bomb, memory exhaustion, network escape, metadata access, FD leakage, and rootless-helper attacks.
- GKE Pod/NetworkPolicy/runtime mutation and deletion races during hold and release.
- VM stale guest, cloned VM, boot substitution, transcript replay, wrong key, channel substitution,
  parallel tenant, and incomplete destruction attacks.
- Root-owned/rootless Linux `x86_64` and `aarch64` where supported; macOS architectures are qualified
  independently.

A skipped, ignored, mocked, or source-shape test cannot count toward production qualification.
Required repository gates remain `cargo fmt --all`, `cargo clippy --workspace -- -D warnings`, and
`cargo nextest run --workspace`; release qualification additionally runs real environment matrices.

## Compatibility And Risks

| Surface                                      | Impact                                                                        |
| -------------------------------------------- | ----------------------------------------------------------------------------- |
| `DaemonRequest::Run`                         | Wire-compatible; secure defaults are a semantic break                         |
| `DaemonResponse`                             | Additive variants break exhaustive source matches                             |
| `HandlerDependencies`                        | Source-breaking isolation-service dependency                                  |
| Config loading                               | Becomes fallible; existing config gets `#[serde(default)]` isolation settings |
| State and manifest JSON                      | Backward-readable optional additions; old records unqualified                 |
| Privileged, host network, hooks, host mounts | Rejected in hostile mode; explicit trusted daemon required                    |
| Colima/GKE mechanisms                        | Shared VM and proot paths remain trusted-only                                 |
| Startup and resource cost                    | Dedicated Pod/VM and durable fsync increase latency and storage               |

New dependencies are `zeroize`, `postcard`, `ed25519-dalek`, `hkdf`, `chacha20poly1305`,
`getrandom`, `kube`, and `k8s-openapi`. Kubernetes dependencies are Linux/feature gated; cryptographic
and serialization dependencies sit behind application or adapter boundaries, not policy decisions.

## Acceptance Criteria

1. The design remains `proposed` until explicit approval is recorded.
2. Hostile mode is the final immutable default; requests cannot downgrade it.
3. Root-owned and rootless identities are validated once and included in the environment epoch.
4. `IsolationService` uniquely owns a non-cloneable private ledger mutation capability.
5. No public constructor can mint a stage, release, cleanup, or durable commit authority.
6. Startup evidence, raw blobs, reservations, observed locators, staged evidence, release
   authorization, acknowledgement, terminal state, cleanup, and quarantine are crash-durable.
7. Deterministic reservations precede mutation; typed observed locators follow mutation; discovery
   can recover the gap between them.
8. One canonical stage failure preserves partial ownership for fenced cleanup or quarantine.
9. No synchronous lock is held across async I/O; ledger CAS serializes durable transitions.
10. Staged verification derives its subject from `StagedExecution`, not caller-assembled identifiers.
11. Release requires durable `ReleaseAuthorized`, an exact subject-generation comparison, and an
    authenticated backend acknowledgement before `Released` is committed.
12. Cleanup requires a renewable fenced lease and independent absence evidence.
13. Hypervisor and measured-guest authorities prove distinct VM requirement classes.
14. GKE release dependencies are admission-immutable and released only by the trusted controller's
    serialized transaction.
15. Qualification is a signed detached manifest bound to the exact shipped executable and matrix.
16. Complete restart inputs reproduce the effective workload or hostile restart is denied.
17. Old records remain readable but never become qualified by deserialization.
18. All six adapters pass non-skipped real-environment matrices before a production-ready hostile
    claim or `/gm-plan` handoff.
